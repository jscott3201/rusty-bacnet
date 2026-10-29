//! Writing a workspace file atomically: a complete, synced temporary file
//! published over the old one, so a crash leaves either the old file or
//! the new one, never half of either.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

/// Write `text` to `temp` (created new, 0600 on Unix, synced), then publish
/// it as `path`: a rename, or with `create` a hard link, so a concurrent
/// creation never overwrites. Profiles and the session ledger are saved
/// this way. Only files this attempt created are cleaned up: a stale
/// temporary name, or another writer's collision, survives an unsuccessful
/// `create_new`.
pub fn publish(temp: &Path, path: &Path, text: &str, create: bool) -> std::io::Result<()> {
    let mut created = false;
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(temp)?;
        created = true;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if create {
            fs::hard_link(temp, path)?;
        } else {
            fs::rename(temp, path)?;
        }
        Ok(())
    })();
    if created {
        let _ = fs::remove_file(temp);
    }
    result
}
