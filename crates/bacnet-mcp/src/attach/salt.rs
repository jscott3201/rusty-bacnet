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
//! file or open to anyone else is refused, never made anew.
//!
//! The pipe is Windows only; the file's handling builds on Unix too, for
//! its tests, with a 0600 mode standing in for the owner-only descriptor.

use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::AttachError;

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

/// The pipe's name for the workspace `dir`: a hash of the salt and then
/// the workspace's canonical path, so each profile has its own and nobody
/// who can't read the salt can work it out. The salt's fixed length
/// keeps the two parts apart.
pub(crate) fn pipe_name(salt: &Salt, dir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(dir)
        .or_else(|_| std::path::absolute(dir))
        .unwrap_or_else(|_| dir.to_path_buf());
    let digest = Sha256::new()
        .chain_update(salt.0)
        .chain_update(canonical.as_os_str().as_encoded_bytes())
        .finalize();
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    PathBuf::from(format!(r"\\.\pipe\rusty-bacnet-mcp-{hex}"))
}

/// The workspace `dir`'s salt, checked: the file must be a plain file,
/// owned by this user and readable by nobody else, and hold exactly
/// [`SALT_LEN`] bytes.
pub(crate) fn load(dir: &Path) -> Result<Salt, SaltError> {
    let path = dir.join(SALT_FILE);
    let shown = path.display();
    let mut file = match platform::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(SaltError::Missing(path)),
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
    let mut bytes = Vec::with_capacity(SALT_LEN + 1);
    (&mut file)
        .take(SALT_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SaltError::Refused(format!("cannot read {shown}: {e}")))?;
    let salt = <[u8; SALT_LEN]>::try_from(bytes.as_slice()).map_err(|_| {
        SaltError::Refused(format!(
            "{shown} is corrupt: it doesn't hold exactly {SALT_LEN} bytes; remove it with no \
             TUI open on this profile, and the next TUI makes a new one"
        ))
    })?;
    Ok(Salt(salt))
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
    let mut tag = [0u8; 8];
    getrandom::fill(&mut salt)
        .and_then(|()| getrandom::fill(&mut tag))
        .map_err(|e| SaltError::Refused(format!("no random bytes for the MCP pipe's salt: {e}")))?;
    let tag: String = tag.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = dir.join(format!("{SALT_FILE}.{tag}.tmp"));
    let written = platform::create_private(&temporary).and_then(|mut file| {
        file.write_all(&salt)?;
        file.sync_all()
    });
    let linked = written.and_then(|()| std::fs::hard_link(&temporary, &path));
    // Made or not, the temporary name goes; the file stays under the
    // other name when linked.
    if let Err(e) = std::fs::remove_file(&temporary)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {e}", temporary.display());
    }
    match linked {
        Ok(()) => Ok(Salt(salt)),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => load(dir),
        Err(e) => Err(SaltError::Refused(format!(
            "cannot make {}: {e}",
            path.display()
        ))),
    }
}

/// How the salt file is made, opened and checked: on Windows with the
/// owner-only descriptor the pipe carries, its reparse points not
/// followed.
#[cfg(windows)]
mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    use super::super::win32::{self, Descriptor};

    /// A new file at `path`, owned by this user and open to nobody else.
    pub(super) fn create_private(path: &Path) -> io::Result<File> {
        let descriptor = Descriptor::owner_only(&win32::own_sid()?)?;
        win32::create_file(path, &descriptor)
    }

    /// The file at `path` for reading; a link is opened as itself.
    pub(super) fn open(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    /// Whether `file` is owned by this user and its access list grants
    /// nobody else.
    pub(super) fn check_private(file: &File) -> io::Result<()> {
        win32::check_private_file(file, &win32::own_sid()?)
    }
}

/// How the salt file is made, opened and checked in Unix test builds:
/// mode 0600, never through a link.
#[cfg(unix)]
mod platform {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::Path;

    /// A new file at `path`, mode 0600.
    pub(super) fn create_private(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// The file at `path` for reading, not through a link.
    pub(super) fn open(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// Whether `file` is this user's and nobody else may read or write it.
    pub(super) fn check_private(file: &File) -> io::Result<()> {
        let metadata = file.metadata()?;
        if metadata.uid() != super::super::socket::own_uid() {
            return Err(io::Error::other(format!("owned by uid {}", metadata.uid())));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(io::Error::other(format!(
                "mode {:o}",
                metadata.mode() & 0o777
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "salt_tests.rs"]
mod tests;
