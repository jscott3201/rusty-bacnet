//! The socket's filesystem checks: a private workspace, no symbolic
//! links, and no removing a socket someone is still listening on.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::Path;
use std::time::Duration;

use tokio::net::UnixStream;

/// How long a connection to an existing socket may take before it counts
/// as live: a listener with a full backlog answers late, not never.
const PROBE_WAIT: Duration = Duration::from_secs(1);

/// The socket's mode: its owner reads and writes, nobody else.
const SOCKET_MODE: u32 = 0o600;

/// Check that `dir` is a directory of user `uid` (not a symbolic link to
/// one) that nobody else can enter: mode 0700 or narrower.
pub(super) fn check_workspace(dir: &Path, uid: u32) -> Result<(), String> {
    let shown = dir.display();
    let meta = fs::symlink_metadata(dir)
        .map_err(|e| format!("cannot inspect the workspace {shown}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "the workspace {shown} is a symbolic link; the MCP socket is only opened in a real \
             directory"
        ));
    }
    if !meta.is_dir() {
        return Err(format!("the workspace {shown} is not a directory"));
    }
    if meta.uid() != uid {
        return Err(format!(
            "the workspace {shown} belongs to uid {}, not this user (uid {uid}), so the MCP \
             socket is not opened there",
            meta.uid()
        ));
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "the workspace {shown} has mode {mode:03o}; the MCP socket needs it to be 0700, so \
             only you can reach it (run: chmod 700 '{shown}')"
        ));
    }
    Ok(())
}

/// Make way for a socket at `path`: nothing there, or a socket nobody
/// listens on, which is removed. A live socket, a symbolic link (never
/// followed) or any other file is left alone and refused.
pub(super) async fn clear_stale(path: &Path) -> Result<(), String> {
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
    match tokio::time::timeout(PROBE_WAIT, UnixStream::connect(path)).await {
        Ok(Ok(_live)) => Err(format!(
            "another process is listening on {shown} (is this profile open in another TUI?); \
             it is left alone"
        )),
        Err(_) => Err(format!(
            "{shown} did not answer within {} s and may be live; it is left alone",
            PROBE_WAIT.as_secs()
        )),
        Ok(Err(e)) if e.kind() == ErrorKind::NotFound => Ok(()),
        Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => {
            // Nobody listens: the socket of a TUI that ended without
            // removing it. Remove that very file, not one put in its place.
            remove_own(path, (meta.dev(), meta.ino()));
            match fs::symlink_metadata(path) {
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
                _ => Err(format!("cannot remove the stale socket {shown}")),
            }
        }
        Ok(Err(e)) => Err(format!(
            "cannot tell whether {shown} is live ({e}); it is left alone"
        )),
    }
}

/// Narrow the freshly bound socket at `path` to its owner, and return its
/// device and inode.
pub(super) fn own_socket(path: &Path) -> Result<(u64, u64), String> {
    let shown = path.display();
    fs::set_permissions(path, fs::Permissions::from_mode(SOCKET_MODE))
        .map_err(|e| format!("cannot make {shown} private: {e}"))?;
    let meta = fs::symlink_metadata(path).map_err(|e| format!("cannot inspect {shown}: {e}"))?;
    Ok((meta.dev(), meta.ino()))
}

/// Remove `path` if it is still the socket with device and inode
/// `identity`, never following a symbolic link.
pub(super) fn remove_own(path: &Path, identity: (u64, u64)) {
    let ours = fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.file_type().is_socket() && (meta.dev(), meta.ino()) == identity);
    if ours && let Err(e) = fs::remove_file(path) {
        tracing::warn!("cannot remove the MCP socket {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace owned by another user is refused even at mode 0700
    /// (a test can't own a directory as someone else, so it asks as
    /// another uid instead).
    #[test]
    fn another_users_workspace_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let uid = super::super::own_uid();
        check_workspace(dir.path(), uid).unwrap();
        let error = check_workspace(dir.path(), uid.wrapping_add(1)).unwrap_err();
        assert!(error.contains(&format!("belongs to uid {uid}")), "{error}");
    }
}
