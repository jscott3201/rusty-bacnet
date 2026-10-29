//! An exclusive advisory lock on a profile workspace's ledger, so two
//! sessions on one profile (two terminals, say) can't both change it.
//!
//! The lock is `std::fs::File::try_lock` on `ledger.lock`: `flock` on Unix
//! and `LockFileEx` on Windows. The holder writes its front end and process
//! id into the file (`tui 4242`); another process reads them to say who
//! holds the lock, `bacnet tui (pid 4242)`, where the platform lets it read
//! a locked file (Windows does not).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::Path;

/// The lock file in the workspace.
pub const FILE: &str = "ledger.lock";

/// The held lock; dropping it releases the lock.
#[derive(Debug)]
pub struct WorkspaceLock {
    _file: File,
}

/// Take the workspace's lock for `front_end` (`tui`, later `mcp`), or say
/// who holds it.
pub fn lock(workspace: &Path, front_end: &str) -> Result<WorkspaceLock, String> {
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
            // Best effort: it only helps the message another process shows.
            let _ = file
                .set_len(0)
                .and_then(|()| write!(file, "{front_end} {}", std::process::id()))
                .and_then(|()| file.flush());
            Ok(WorkspaceLock { _file: file })
        }
        Err(TryLockError::WouldBlock) => {
            let holder = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| holder(&text));
            Err(match holder {
                Some((front_end, pid)) => {
                    format!("this profile's ledger is in use by bacnet {front_end} (pid {pid})")
                }
                None => "this profile's ledger is in use by another bacnet process".into(),
            })
        }
        Err(TryLockError::Error(error)) => Err(format!("cannot lock {}: {error}", path.display())),
    }
}

/// The front end and process id a lock file names: `tui 4242`, or a bare
/// id, which earlier releases wrote and only the TUI took.
fn holder(text: &str) -> Option<(String, u32)> {
    let mut words = text.split_whitespace();
    let (front_end, pid) = match (words.next()?, words.next(), words.next()) {
        (pid, None, None) => ("tui", pid),
        (front_end, Some(pid), None) => (front_end, pid),
        _ => return None,
    };
    let valid = !front_end.is_empty()
        && front_end
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '-');
    valid
        .then(|| pid.parse().ok())
        .flatten()
        .map(|pid| (front_end.to_string(), pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock(dir.path(), "tui").unwrap();
        let refused = lock(dir.path(), "mcp").unwrap_err();
        assert!(refused.contains("in use"), "{refused}");
        // Windows does not let another handle read a locked file.
        #[cfg(unix)]
        assert!(
            refused.contains(&format!("bacnet tui (pid {})", std::process::id())),
            "{refused}"
        );
        drop(first);
        lock(dir.path(), "mcp").unwrap();
    }

    #[test]
    fn the_holder_is_read_back() {
        assert_eq!(holder("tui 42\n"), Some(("tui".into(), 42)));
        assert_eq!(holder("mcp 7"), Some(("mcp".into(), 7)));
        assert_eq!(
            holder("42"),
            Some(("tui".into(), 42)),
            "an earlier release's file"
        );
        assert_eq!(holder(""), None);
        assert_eq!(holder("tui"), None);
        assert_eq!(holder("TUI 42"), None);
        assert_eq!(holder("tui 42 extra"), None);
    }
}
