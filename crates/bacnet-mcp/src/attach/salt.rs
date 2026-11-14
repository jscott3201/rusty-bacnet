//! The secret in the Windows pipe's name (#2219). Every user on a machine
//! shares one namespace of pipe names, so a name computed from the
//! workspace's path alone can be computed, and taken first, by anyone who
//! can guess the path. The name mixes in 32 random bytes kept in a file
//! of the workspace that only this user may read: without that file the
//! name can't be found.
//!
//! The file is made once, by the first listener of the workspace, and
//! never replaced: a running listener's name depends on it. It is written
//! whole under a temporary name and then linked into place, which fails
//! when the file exists already, so two TUIs starting at once agree on
//! the one that got there first. A file that is short, long, not a plain
//! file, or open to anyone but this user (and, on Windows, SYSTEM and the
//! Administrators group, who may read anything anyway) is refused, never
//! made anew.
//!
//! The pipe is Windows only; the file's handling builds on Unix too, for
//! its tests, with a 0600 mode standing in for the owner-only descriptor.

use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::AttachError;
use super::nonce::Nonce;

/// The salt's file name in the profile's workspace.
pub(crate) const SALT_FILE: &str = "mcp-pipe.salt";

/// How many random bytes the salt holds.
pub(crate) const SALT_LEN: usize = 32;

/// The salt's bytes. Never shown: they are the name's secret.
pub(crate) struct Salt(pub(crate) [u8; SALT_LEN]);

impl fmt::Debug for Salt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Salt(..)")
    }
}

/// Why the salt couldn't be had.
#[derive(Debug)]
pub(crate) enum SaltError {
    /// The workspace has no salt file: no listener has run there.
    Missing(PathBuf),
    /// The file is there but isn't safe to use, or can't be read or made.
    Refused(String),
}

impl From<SaltError> for AttachError {
    fn from(error: SaltError) -> Self {
        match error {
            SaltError::Missing(path) => AttachError(format!(
                "no TUI is listening for MCP on this profile: none has listened on it yet \
                 ({} is missing); open the profile in `bacnet tui` first",
                path.display()
            )),
            SaltError::Refused(message) => AttachError(message),
        }
    }
}

/// The pipe's name for the workspace `dir` in one run of its listener: a
/// hash of the salt, the run's nonce and then the workspace's canonical
/// path, so each profile has its own, nobody who can't read the salt and
/// the nonce can work it out, and each run has a new one (#2237). The
/// salt's and the nonce's fixed lengths keep the parts apart.
pub(crate) fn pipe_name(salt: &Salt, nonce: &Nonce, dir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(dir)
        .or_else(|_| std::path::absolute(dir))
        .unwrap_or_else(|_| dir.to_path_buf());
    let digest = Sha256::new()
        .chain_update(salt.0)
        .chain_update(nonce.0)
        .chain_update(canonical.as_os_str().as_encoded_bytes())
        .finalize();
    PathBuf::from(format!(
        r"\\.\pipe\rusty-bacnet-mcp-{}",
        bacnet_ops::octets::hex(&digest[..16])
    ))
}

/// The workspace `dir`'s salt, checked as [`read_private`] does.
pub(crate) fn load(dir: &Path) -> Result<Salt, SaltError> {
    read_private(&dir.join(SALT_FILE)).map(Salt)
}

/// The `N` bytes of the private file at `path`, checked: it must be a
/// plain file, owned by this user and open to nobody else (see the
/// module), and hold exactly `N` bytes. The salt and the nonce are both
/// read here.
pub(crate) fn read_private<const N: usize>(path: &Path) -> Result<[u8; N], SaltError> {
    let shown = path.display();
    let mut file = match platform::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(SaltError::Missing(path.to_path_buf()));
        }
        Err(e) => return Err(SaltError::Refused(format!("cannot open {shown}: {e}"))),
    };
    let plain = file
        .metadata()
        .map_err(|e| SaltError::Refused(format!("cannot inspect {shown}: {e}")))?;
    if !plain.is_file() || plain.is_symlink() {
        return Err(SaltError::Refused(format!(
            "{shown} is not a plain file; not using it"
        )));
    }
    platform::check_private(&file).map_err(|e| {
        SaltError::Refused(format!(
            "{shown} is not this user's alone ({e}); not using it: remove it with no TUI open \
             on this profile, and the next TUI makes a new one"
        ))
    })?;
    let mut bytes = Vec::with_capacity(N + 1);
    (&mut file)
        .take(N as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SaltError::Refused(format!("cannot read {shown}: {e}")))?;
    <[u8; N]>::try_from(bytes.as_slice()).map_err(|_| {
        SaltError::Refused(format!(
            "{shown} is corrupt: it doesn't hold exactly {N} bytes; remove it with no \
             TUI open on this profile, and the next TUI makes a new one"
        ))
    })
}

/// Write `bytes` whole and synced to a new private file of `dir` under a
/// random temporary name beside `file`, and give that name: the salt is
/// linked into place from it, the nonce renamed.
pub(crate) fn write_temporary(dir: &Path, file: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut tag = [0u8; 8];
    getrandom::fill(&mut tag).map_err(io::Error::other)?;
    let tag = bacnet_ops::octets::hex(&tag);
    let temporary = dir.join(format!("{file}.{tag}.tmp"));
    // A file already at the name isn't this call's: left alone.
    let mut made = platform::create_private(&temporary)?;
    let written = made.write_all(bytes).and_then(|()| made.sync_all());
    drop(made);
    match written {
        Ok(()) => Ok(temporary),
        Err(e) => {
            remove_temporary(&temporary);
            Err(e)
        }
    }
}

/// Remove the temporary file `path`, if it is there still.
pub(crate) fn remove_temporary(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {e}", path.display());
    }
}

/// The workspace `dir`'s salt, made now if it has none.
pub(crate) fn load_or_create(dir: &Path) -> Result<Salt, SaltError> {
    match load(dir) {
        Err(SaltError::Missing(_)) => create(dir),
        loaded => loaded,
    }
}

/// Make the salt file: written whole and synced under a name of its own,
/// then linked to [`SALT_FILE`], which fails if that exists. A listener
/// that loses the race takes the winner's salt.
fn create(dir: &Path) -> Result<Salt, SaltError> {
    let path = dir.join(SALT_FILE);
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt)
        .map_err(|e| SaltError::Refused(format!("no random bytes for the MCP pipe's salt: {e}")))?;
    let linked = write_temporary(dir, SALT_FILE, &salt).and_then(|temporary| {
        let linked = std::fs::hard_link(&temporary, &path);
        // Made or not, the temporary name goes; the file stays under the
        // other name when linked.
        remove_temporary(&temporary);
        linked
    });
    match linked {
        Ok(()) => Ok(Salt(salt)),
        // The race's loser takes the winner's salt; that one gone again
        // means another process removed it.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match load(dir) {
            Err(SaltError::Missing(path)) => Err(SaltError::Refused(format!(
                "{} was removed as this TUI started, by another process; not listening",
                path.display()
            ))),
            loaded => loaded,
        },
        Err(e) => Err(SaltError::Refused(format!(
            "cannot make {}: {e}",
            path.display()
        ))),
    }
}

/// How the salt and nonce files are made, opened and checked: on Windows
/// with the owner-only descriptor the pipe carries, their reparse points
/// not followed.
#[cfg(windows)]
pub(crate) mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    use super::super::win32::{self, Descriptor};

    /// A new file at `path`, owned by this user and open to nobody else.
    pub(crate) fn create_private(path: &Path) -> io::Result<File> {
        let descriptor = Descriptor::owner_only(&win32::own_sid()?)?;
        win32::create_file(path, &descriptor)
    }

    /// The file at `path` for reading; a link is opened as itself.
    pub(crate) fn open(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    /// Whether `file` is owned by this user and its access list grants
    /// nobody else but SYSTEM and the Administrators group.
    pub(crate) fn check_private(file: &File) -> io::Result<()> {
        win32::check_private_file(file, &win32::own_sid()?)
    }
}

/// How the salt and nonce files are made, opened and checked in Unix test
/// builds:
/// mode 0600, never through a link.
#[cfg(unix)]
pub(crate) mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::Path;

    /// A new file at `path`, mode 0600.
    pub(crate) fn create_private(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// The file at `path` for reading, not through a link.
    pub(crate) fn open(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// Whether `file` is this user's and nobody else may read or write it.
    pub(crate) fn check_private(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        private(
            metadata.uid(),
            metadata.mode(),
            super::super::socket::own_uid(),
        )
    }

    /// Whether a file owned by `uid` with `mode` is the user `own`'s
    /// alone.
    pub(crate) fn private(uid: u32, mode: u32, own: u32) -> io::Result<()> {
        if uid != own {
            return Err(io::Error::other(format!("owned by uid {uid}")));
        }
        if mode & 0o077 != 0 {
            return Err(io::Error::other(format!("mode {:o}", mode & 0o777)));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "salt_tests.rs"]
mod tests;
