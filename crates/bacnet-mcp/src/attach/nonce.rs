//! The run's nonce in the Windows pipe's name (#2237). Any local user can
//! list pipe names, so the salt (`salt.rs`) keeps the name from being
//! worked out in advance but not from being seen while the TUI listens;
//! a name that never changed could then be taken while the TUI is closed.
//! Each listener therefore picks 32 fresh random bytes and mixes them into
//! the name, and a name seen during one run is useless for the next.
//!
//! The nonce is published in [`NONCE_FILE`] of the workspace, under the
//! salt's rules: written whole and synced to a new file owned by this user
//! and open to nobody else from its creation, then renamed over the last
//! run's, so a reader finds the old file or the new one, never part of
//! one. Only the holder of the workspace's lock writes it, after its
//! pipe's first instance has claimed the name, and removes it as it stops.
//! `bacnet mcp connect` reads it on every connect, with the salt's checks:
//! not through a link, a plain file of this user's alone, exactly
//! [`NONCE_LEN`] bytes, refused otherwise. A nonce left by a TUI that
//! crashed names a pipe nobody holds, and the bridge says no TUI is
//! listening.

use std::fmt;
use std::io;
use std::path::Path;
use std::time::Duration;

use super::AttachError;
use super::salt::{self, SaltError};

/// The nonce's file name in the profile's workspace.
pub(crate) const NONCE_FILE: &str = "mcp-pipe.nonce";

/// How many random bytes the nonce holds.
pub(crate) const NONCE_LEN: usize = 32;

/// The nonce's bytes. Never shown: with the salt, they are the name's
/// secret.
pub(crate) struct Nonce(pub(crate) [u8; NONCE_LEN]);

impl fmt::Debug for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nonce(..)")
    }
}

/// A new nonce, for a listener starting.
pub(crate) fn fresh() -> Result<Nonce, AttachError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce)
        .map_err(|e| AttachError(format!("no random bytes for the MCP pipe's nonce: {e}")))?;
    Ok(Nonce(nonce))
}

/// How many times the rename into place is tried while it fails for a
/// moment (a reader holding the old file, say), and how long apart.
const PUBLISH_TRIES: u32 = 3;
const PUBLISH_RETRY: Duration = Duration::from_millis(20);

/// Publish `nonce` as the workspace `dir`'s, replacing the last run's in
/// one step, and clear temporary files a crashed TUI left. Only the
/// holder of the workspace's lock may.
pub(crate) fn publish(dir: &Path, nonce: &Nonce) -> Result<(), AttachError> {
    publish_with(
        dir,
        nonce,
        |from, to| std::fs::rename(from, to),
        || {
            std::thread::sleep(PUBLISH_RETRY);
        },
    )
}

/// [`publish`], renaming with `rename` and resting with `pause` between
/// tries.
fn publish_with(
    dir: &Path,
    nonce: &Nonce,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    pause: impl Fn(),
) -> Result<(), AttachError> {
    sweep(dir);
    let path = dir.join(NONCE_FILE);
    let renamed = salt::write_temporary(dir, NONCE_FILE, &nonce.0).and_then(|temporary| {
        let mut tries = 1;
        let renamed = loop {
            match rename(&temporary, &path) {
                Err(e) if transient(&e) && tries < PUBLISH_TRIES => {
                    tries += 1;
                    pause();
                }
                renamed => break renamed,
            }
        };
        if renamed.is_err() {
            salt::remove_temporary(&temporary);
        }
        renamed
    });
    renamed.map_err(|e| {
        AttachError(format!(
            "cannot publish {}: {e}; not listening for MCP, try again",
            path.display()
        ))
    })
}

/// Whether a failed rename may pass if tried again: access denied, or on
/// Windows a sharing violation.
fn transient(error: &io::Error) -> bool {
    /// `ERROR_SHARING_VIOLATION`.
    #[cfg(windows)]
    const SHARING_VIOLATION: Option<i32> = Some(32);
    #[cfg(not(windows))]
    const SHARING_VIOLATION: Option<i32> = None;
    error.kind() == io::ErrorKind::PermissionDenied
        || (SHARING_VIOLATION.is_some() && error.raw_os_error() == SHARING_VIOLATION)
}

/// Remove the nonce's temporary files of earlier runs from `dir`: only a
/// TUI that stopped mid-publish leaves one, and the lock's holder alone
/// writes them.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{NONCE_FILE}.");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            salt::remove_temporary(&entry.path());
        }
    }
}

/// The workspace `dir`'s current nonce, checked as the salt is.
pub(crate) fn load(dir: &Path) -> Result<Nonce, AttachError> {
    match salt::read_private(&dir.join(NONCE_FILE)) {
        Ok(nonce) => Ok(Nonce(nonce)),
        Err(SaltError::Missing(path)) => Err(AttachError(format!(
            "no TUI is listening for MCP on this profile now ({} is missing: the last TUI \
             to listen on it has stopped); open the profile in `bacnet tui` first",
            path.display()
        ))),
        Err(SaltError::Refused(message)) => Err(AttachError(message)),
    }
}

/// Remove the workspace `dir`'s nonce as its listener stops, with the
/// lock still held: the name it gave goes with the pipe.
pub(crate) fn withdraw(dir: &Path) {
    let path = dir.join(NONCE_FILE);
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {e}", path.display());
    }
}

#[cfg(test)]
#[path = "nonce_tests.rs"]
mod tests;
