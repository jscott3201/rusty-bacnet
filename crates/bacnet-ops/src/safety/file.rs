//! Writing a workspace file atomically: a complete, synced temporary file
//! published over the old one, so a crash leaves either the old file or
//! the new one, never half of either.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

/// Write `text` to `temp` (created new, 0600 on Unix, synced), then publish
/// it as `path`: a rename, or with `create` a hard link, so a concurrent
/// creation never overwrites. Profiles, the session ledger and the MCP
/// plan grant's write count are saved this way. On Unix the directory is
/// synced after the publish too, so the new name survives a power loss;
/// when that sync fails, the file is visible but may not survive one, and
/// the save reports the error. Only files this attempt created are cleaned
/// up: a stale temporary name, or another writer's collision, survives an
/// unsuccessful `create_new`.
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
    result?;
    sync_directory(path)
}

/// Sync the directory holding `path`, so a rename or link into it is
/// durable. Windows has no directory handle to sync this way (NTFS
/// journals the rename itself), so there it does nothing.
fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(directory)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(test)]
    tests::DIRECTORY_SYNCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Directory syncs made, for the test below.
    pub(super) static DIRECTORY_SYNCS: AtomicUsize = AtomicUsize::new(0);

    /// A publish replaces the file, leaves no temporary file, and syncs
    /// the directory after the rename and after a link.
    #[test]
    fn a_publish_replaces_the_file_and_syncs_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let before = DIRECTORY_SYNCS.load(Ordering::Relaxed);
        publish(&dir.path().join(".a.tmp"), &path, "one", true).unwrap();
        publish(&dir.path().join(".b.tmp"), &path, "two", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(DIRECTORY_SYNCS.load(Ordering::Relaxed) >= before + 2);
    }
}
