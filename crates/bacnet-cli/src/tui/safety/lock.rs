//! An exclusive advisory lock on a profile workspace's ledger, so two
//! sessions on one profile (two terminals, say) can't both change it.
//!
//! The lock is `std::fs::File::try_lock` on `ledger.lock`: `flock` on Unix
//! and `LockFileEx` on Windows. The holder writes its process id into the
//! file; another process reads it to say who holds the lock, where the
//! platform lets it read a locked file (Windows does not).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::Path;

/// The lock file in the workspace.
pub(crate) const FILE: &str = "ledger.lock";

/// The held lock; dropping it releases the lock.
#[derive(Debug)]
pub(crate) struct WorkspaceLock {
    _file: File,
}

/// Take the workspace's lock, or say who holds it.
pub(crate) fn lock(workspace: &Path) -> Result<WorkspaceLock, String> {
    let path = workspace.join(FILE);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => {
            // Best effort: the id only helps the message another process shows.
            let _ = file
                .set_len(0)
                .and_then(|()| write!(file, "{}", std::process::id()))
                .and_then(|()| file.flush());
            Ok(WorkspaceLock { _file: file })
        }
        Err(TryLockError::WouldBlock) => {
            let holder = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            Err(match holder {
                Some(pid) => {
                    format!("this profile's ledger is in use by another bacnet tui (process {pid})")
                }
                None => "this profile's ledger is in use by another bacnet tui".into(),
            })
        }
        Err(TryLockError::Error(error)) => Err(format!("cannot lock {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock(dir.path()).unwrap();
        let refused = lock(dir.path()).unwrap_err();
        assert!(refused.contains("in use"), "{refused}");
        drop(first);
        lock(dir.path()).unwrap();
    }
}
