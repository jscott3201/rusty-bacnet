//! The socket's filesystem guards: a private workspace, checked and
//! narrowed on one open handle; the listener's lock; no symbolic links;
//! and the socket checked again once it is bound.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::time::Duration;

/// The lock file a listener holds in the workspace for its whole life.
pub const LOCK_FILE: &str = "mcp.lock";

/// The longest socket path the OS takes: `sun_path` less its terminator.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) const MAX_PATH: usize = 107;
/// The longest socket path the OS takes: `sun_path` less its terminator.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(super) const MAX_PATH: usize = 103;

/// The socket's mode: its owner reads and writes, nobody else.
const SOCKET_MODE: u32 = 0o600;

/// The workspace's mode once narrowed.
const WORKSPACE_MODE: u32 = 0o700;

/// The workspace directory, open, as it was checked: a re-check after the
/// socket is bound compares the path with it.
pub(super) struct WorkspaceDir {
    file: File,
    identity: (u64, u64),
}

/// Open `dir` without following a final symbolic link and check, on that
/// one handle, that it is a directory of user `uid`; narrow it to 0700
/// when it is looser, saying so.
pub(super) fn open_workspace(dir: &Path, uid: u32) -> Result<WorkspaceDir, String> {
    let shown = dir.display();
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| {
            // Linux says ELOOP for a final link, macOS ENOTDIR: look again
            // only to word the refusal.
            let link = fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink());
            match e.raw_os_error() {
                Some(libc::ELOOP | libc::ENOTDIR) if link => format!(
                    "the workspace {shown} is a symbolic link; the MCP socket is only opened in \
                     a real directory"
                ),
                Some(libc::ENOTDIR) => format!("the workspace {shown} is not a directory"),
                _ => format!("cannot open the workspace {shown}: {e}"),
            }
        })?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot inspect the workspace {shown}: {e}"))?;
    if meta.uid() != uid {
        return Err(format!(
            "the workspace {shown} belongs to uid {}, not this user (uid {uid}), so the MCP \
             socket is not opened there",
            meta.uid()
        ));
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        file.set_permissions(fs::Permissions::from_mode(WORKSPACE_MODE))
            .map_err(|e| format!("cannot make the workspace {shown} private: {e}"))?;
        tracing::info!(
            "MCP: narrowed the workspace {shown} from mode {mode:03o} to 0700, so only you can \
             reach its socket"
        );
    }
    Ok(WorkspaceDir {
        file,
        identity: (meta.dev(), meta.ino()),
    })
}

impl WorkspaceDir {
    /// Check that `dir` still names this directory, and that it is still
    /// user `uid`'s and private.
    pub(super) fn recheck(&self, dir: &Path, uid: u32) -> Result<(), String> {
        let shown = dir.display();
        let named =
            fs::symlink_metadata(dir).map_err(|e| format!("cannot inspect {shown}: {e}"))?;
        let held = self
            .file
            .metadata()
            .map_err(|e| format!("cannot inspect {shown}: {e}"))?;
        if named.file_type().is_symlink() || (named.dev(), named.ino()) != self.identity {
            return Err(format!(
                "the workspace {shown} was replaced while the MCP socket was being made"
            ));
        }
        if held.uid() != uid || held.mode() & 0o077 != 0 {
            return Err(format!(
                "the workspace {shown} changed owner or mode while the MCP socket was being made"
            ));
        }
        Ok(())
    }
}

/// Refuse a socket path longer than the OS takes.
pub(super) fn check_length(path: &Path) -> Result<(), String> {
    let length = path.as_os_str().len();
    if length > MAX_PATH {
        return Err(format!(
            "the MCP socket path {} is {length} bytes, more than the {MAX_PATH} a Unix socket \
             path may have here; use a shorter profile name, or point BACNET_CONFIG_DIR at a \
             shorter directory",
            path.display()
        ));
    }
    Ok(())
}

/// How many times a listener tries for its lock, and how long it waits
/// between tries: a `connect` probe holds a shared lock only for a moment.
const LOCK_TRIES: u32 = 5;
const LOCK_RETRY: Duration = Duration::from_millis(50);

/// Take the listener's lock in `dir`, never through a symbolic link, or
/// say that another listener holds it.
pub(super) async fn lock(dir: &Path) -> Result<File, String> {
    let path = dir.join(LOCK_FILE);
    let shown = path.display();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(SOCKET_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
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
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(LOCK_FILE))
        .is_ok_and(|file| matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock)))
}

/// Make way for the socket at `path`, with the listener's lock held, so
/// any socket there is a stale one and is removed. A symbolic link (never
/// followed) or any other file is left alone and refused.
pub(super) fn clear_stale(path: &Path) -> Result<(), String> {
    let shown = path.display();
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot inspect {shown}: {e}")),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        return Err(format!(
            "{shown} is a symbolic link; it is neither followed nor removed (remove it yourself)"
        ));
    }
    if !kind.is_socket() {
        return Err(format!(
            "{shown} exists and is not a socket; it is left alone (remove it yourself)"
        ));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove the stale socket {shown}: {e}")),
    }
}

/// Check that `path` is the socket just bound, not something `bind`
/// followed a symbolic link to, narrow it to its owner, and return its
/// device and inode. On failure, what this bind made is removed.
pub(super) fn own_socket(path: &Path) -> Result<(u64, u64), String> {
    let shown = path.display();
    let meta = fs::symlink_metadata(path).map_err(|e| format!("cannot inspect {shown}: {e}"))?;
    if !meta.file_type().is_socket() {
        // A link planted before the bind: the socket went to its target.
        if let Ok(target) = fs::read_link(path) {
            let target = path.parent().unwrap_or(Path::new("")).join(target);
            if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_socket()) {
                let _ = fs::remove_file(&target);
            }
        }
        return Err(format!(
            "{shown} is not the socket just bound (it was replaced); not listening"
        ));
    }
    let identity = (meta.dev(), meta.ino());
    let narrowed = fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE))
        .map_err(|e| format!("cannot make {shown} private: {e}"));
    let still = fs::symlink_metadata(path).is_ok_and(|again| {
        again.file_type().is_socket()
            && (again.dev(), again.ino()) == identity
            && again.mode() & 0o777 == SOCKET_MODE
    });
    if let Err(error) = narrowed.and_then(|()| {
        still
            .then_some(())
            .ok_or_else(|| format!("{shown} was replaced while it was made private"))
    }) {
        remove_own(path, identity);
        return Err(error);
    }
    Ok(identity)
}

/// Remove `path` if it is still the socket with device and inode
/// `identity`, never following a symbolic link. The listener's lock is
/// held meanwhile, so no other listener can have put its own there.
pub(super) fn remove_own(path: &Path, identity: (u64, u64)) {
    let ours = fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.file_type().is_socket() && (meta.dev(), meta.ino()) == identity);
    if ours && let Err(e) = fs::remove_file(path) {
        tracing::warn!("cannot remove the MCP socket {}: {e}", path.display());
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
