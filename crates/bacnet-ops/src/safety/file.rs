//! Writing a workspace file atomically: a complete, synced temporary file
//! published over the old one, so a crash leaves either the old file or
//! the new one, never half of either.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

/// Write `text` to `temp` (created new, 0600 on Unix, synced), then publish
/// it as `path`: a rename, or with `create` a hard link, so a concurrent
/// creation never overwrites. Profiles, the session ledger and the MCP
/// plan grant's write count are saved this way. Only files this attempt
/// created are cleaned up: a stale temporary name, or another writer's
/// collision, survives an unsuccessful `create_new`.
///
/// On Unix the directory is synced after the publish too, so the new name
/// survives a power loss. A filesystem that can't sync a directory (some
/// network and FUSE mounts) counts as synced, with one warning a process.
/// Any other failure of that sync is an error that says the new file is
/// already in place ([`in_place`] tells it apart): it is there now, but
/// may not survive a power loss. Every earlier failure leaves the previous
/// file as it was.
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
    sync_parent(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            InPlace(format!(
                "the new file is in place, but its directory could not be synced, so it may \
                 not survive a power loss: {error}"
            )),
        )
    })
}

/// Whether `error` came from [`publish`] after the new file was in place:
/// only the directory sync failed.
pub fn in_place(error: &std::io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<InPlace>())
}

/// A failed directory sync after the publish.
#[derive(Debug)]
struct InPlace(String);

impl std::fmt::Display for InPlace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InPlace {}

/// Sync the directory holding `path`, so a rename or link into it is
/// durable: what [`publish`] does after it publishes, for a caller that
/// finds the file already in place (a retried save). A filesystem that
/// can't sync a directory counts as synced, with one warning a process.
/// Windows has no directory handle to sync this way (NTFS journals the
/// rename itself), so there it does nothing.
#[cfg(unix)]
pub fn sync_parent(path: &Path) -> std::io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(test)]
    let synced = tests::injected().map_or_else(|| sync(directory), Err);
    #[cfg(not(test))]
    let synced = sync(directory);
    match synced {
        Ok(()) => Ok(()),
        Err(error) if unsupported(&error) => {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    "{} can't sync a directory ({error}); saves there are atomic but may not \
                     survive a power loss",
                    directory.display()
                );
            });
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync(directory: &Path) -> std::io::Result<()> {
    sync_all(&fs::File::open(directory)?)
}

/// `fsync` the open directory, counted for the tests.
#[cfg(unix)]
fn sync_all(directory: &fs::File) -> std::io::Result<()> {
    directory.sync_all()?;
    #[cfg(test)]
    tests::DIRECTORY_SYNCS.with(|n| n.set(n.get() + 1));
    Ok(())
}

/// The filesystem doesn't sync directories: matched on the errno, since
/// `ErrorKind` doesn't name them alike on every platform (macOS reports
/// ENOTSUP as uncategorized).
#[cfg(unix)]
fn unsupported(error: &std::io::Error) -> bool {
    error.raw_os_error().is_some_and(|errno| {
        errno == libc::ENOTSUP
            || errno == libc::EOPNOTSUPP
            || errno == libc::ENOSYS
            || errno == libc::EINVAL
    })
}

/// Sync the directory holding `path`: nothing to do on this platform
/// (see the Unix version).
#[cfg(not(unix))]
pub fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        /// Directory syncs made on this thread, for the tests below.
        pub(super) static DIRECTORY_SYNCS: Cell<usize> = const { Cell::new(0) };
        /// An errno this thread's directory syncs fail with instead, 0 for
        /// none.
        #[cfg(unix)]
        static INJECTED: Cell<i32> = const { Cell::new(0) };
    }

    #[cfg(unix)]
    pub(super) fn injected() -> Option<std::io::Error> {
        let errno = INJECTED.with(Cell::get);
        (errno != 0).then(|| std::io::Error::from_raw_os_error(errno))
    }

    #[cfg(unix)]
    fn inject(errno: i32) {
        INJECTED.with(|n| n.set(errno));
    }

    /// A publish replaces the file, leaves no temporary file, and syncs
    /// the directory after the rename and after a link.
    #[test]
    fn a_publish_replaces_the_file_and_syncs_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let before = DIRECTORY_SYNCS.with(Cell::get);
        publish(&dir.path().join(".a.tmp"), &path, "one", true).unwrap();
        publish(&dir.path().join(".b.tmp"), &path, "two", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        assert_eq!(DIRECTORY_SYNCS.with(Cell::get), before + 2);
        #[cfg(not(unix))]
        let _ = before;
    }

    /// A filesystem that can't sync a directory counts as synced; any
    /// other failure of the sync is an error saying the new file is in
    /// place, and it is.
    #[cfg(unix)]
    #[test]
    fn an_unsupported_directory_sync_counts_as_synced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        for errno in [libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS, libc::EINVAL] {
            inject(errno);
            let saved = publish(&dir.path().join(".a.tmp"), &path, "one", false);
            inject(0);
            saved.unwrap_or_else(|e| panic!("errno {errno}: {e}"));
        }
        inject(libc::EIO);
        let failed = publish(&dir.path().join(".b.tmp"), &path, "two", false);
        inject(0);
        let error = failed.unwrap_err();
        assert!(in_place(&error), "{error}");
        assert!(error.to_string().contains("in place"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert!(!in_place(&std::io::Error::other("before the publish")));
    }
}
