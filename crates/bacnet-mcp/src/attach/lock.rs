//! The listener's lock in the workspace: one listener per profile, on
//! every platform, and the probe `bacnet mcp connect` uses to find the
//! profile a TUI listens on (a socket file alone may be a stale one).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::Duration;

/// The lock file a listener holds in the workspace for its whole life.
pub const LOCK_FILE: &str = "mcp.lock";

/// How many times a listener tries for its lock, and how long it waits
/// between tries: a `connect` probe holds a shared lock only for a moment.
const LOCK_TRIES: u32 = 5;
const LOCK_RETRY: Duration = Duration::from_millis(50);

/// The lock file opened for `options`; on Unix never through a symbolic
/// link, and made with the socket's owner-only mode.
fn open(options: &mut OpenOptions, path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Take the listener's lock in `dir`, or say that another listener holds
/// it.
pub(super) async fn lock(dir: &Path) -> Result<File, String> {
    let path = dir.join(LOCK_FILE);
    let shown = path.display();
    let file = open(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false),
        &path,
    )
    .map_err(|e| format!("cannot open {shown}: {e}"))?;
    for _ in 1..LOCK_TRIES {
        match file.try_lock() {
            Err(TryLockError::WouldBlock) => tokio::time::sleep(LOCK_RETRY).await,
            Ok(()) => return Ok(file),
            Err(TryLockError::Error(e)) => return Err(format!("cannot lock {shown}: {e}")),
        }
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(format!(
            "another TUI is listening for MCP on this profile (it holds {shown})"
        )),
        Err(TryLockError::Error(e)) => Err(format!("cannot lock {shown}: {e}")),
    }
}

/// Whether a listener holds the lock in `dir` now. The probe takes a
/// shared lock, which neither another probe nor (once it is released, at
/// once) a listener starting meanwhile is refused by.
pub(super) fn held(dir: &Path) -> bool {
    open(OpenOptions::new().read(true), &dir.join(LOCK_FILE))
        .is_ok_and(|file| matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// One listener's lock at a time; a probe shows whether one is held,
    /// without refusing another probe.
    #[tokio::test(start_paused = true)]
    async fn the_lock_admits_one_listener() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!held(dir.path()), "no lock file yet");
        let first = lock(dir.path()).await.unwrap();
        assert!(held(dir.path()));
        let error = lock(dir.path()).await.unwrap_err();
        assert!(error.contains("another TUI is listening"), "{error}");
        drop(first);
        assert!(!held(dir.path()));
        let probe = File::open(dir.path().join(LOCK_FILE)).unwrap();
        probe.try_lock_shared().unwrap();
        assert!(
            !held(dir.path()),
            "another probe isn't taken for a listener"
        );
    }

    /// A listener starting while a probe holds its shared lock waits for
    /// it and takes the lock once it goes; a lock held throughout refuses
    /// it.
    #[tokio::test(start_paused = true)]
    async fn a_listener_waits_out_a_probe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        fs::write(&path, "").unwrap();
        let probe = File::open(&path).unwrap();
        probe.try_lock_shared().unwrap();
        let starting = tokio::spawn({
            let dir = dir.path().to_path_buf();
            async move { lock(&dir).await }
        });
        tokio::time::sleep(Duration::from_millis(70)).await;
        drop(probe);
        assert!(starting.await.unwrap().is_ok(), "the probe went first");
        let probe = File::open(&path).unwrap();
        probe.try_lock_shared().unwrap();
        let error = lock(dir.path()).await.unwrap_err();
        assert!(error.contains("another TUI is listening"), "{error}");
    }
}
