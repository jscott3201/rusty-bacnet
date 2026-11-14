//! The pipe's salt (#2219): the name it gives, the file made once and
//! agreed on by racing listeners, and the files refused. These run on
//! every platform; on Unix a 0600 mode stands in for the owner-only
//! descriptor.

use std::fs::File;
use std::sync::{Arc, Barrier};

use super::*;
use crate::attach::nonce::NONCE_LEN;

/// A salt of every byte `b`.
fn salt_of(b: u8) -> Salt {
    Salt([b; SALT_LEN])
}

/// The name for the workspace `dir` with the salt `salt_of(b)` and a
/// nonce of zeroes.
fn named(b: u8, dir: &Path) -> PathBuf {
    pipe_name(&salt_of(b), &Nonce([0; NONCE_LEN]), dir)
}

/// The name the pipe had before #2219: the workspace's path alone.
fn unsalted(dir: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(dir).unwrap();
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    PathBuf::from(format!(
        r"\\.\pipe\rusty-bacnet-mcp-{}",
        bacnet_ops::octets::hex(&digest[..16])
    ))
}

/// Write `bytes` to a new private file at `path`, as the salt is made.
fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = platform::create_private(path).unwrap();
    file.write_all(bytes).unwrap();
}

/// The name is the same however the workspace is spelt, differs between
/// workspaces and between salts, can't be had from the path alone, and
/// says nothing but a hash.
#[test]
fn the_name_needs_the_salt_and_the_workspace() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let name = named(1, one.path());
    assert_eq!(name, named(1, one.path()));
    assert_eq!(name, named(1, &one.path().join(".")));
    assert_ne!(name, named(1, two.path()));
    assert_ne!(name, named(2, one.path()), "the salt counts");
    assert_ne!(name, unsalted(one.path()), "the path alone doesn't give it");
    let shown = name.to_str().unwrap();
    let hash = shown.strip_prefix(r"\\.\pipe\rusty-bacnet-mcp-").unwrap();
    assert_eq!(hash.len(), 32, "{shown}");
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()), "{shown}");
    assert_eq!(format!("{:?}", salt_of(7)), "Salt(..)", "never shown");
}

/// No salt file: no listener has run there, which the bridge says.
#[test]
fn a_workspace_without_a_salt_has_no_listener() {
    let dir = tempfile::tempdir().unwrap();
    let error = load(dir.path()).unwrap_err();
    assert!(matches!(error, SaltError::Missing(_)), "{error:?}");
    let error = AttachError::from(error).to_string();
    assert!(error.contains("no TUI is listening"), "{error}");
    assert!(error.contains(SALT_FILE), "{error}");
}

/// The first listener makes the salt: 32 bytes, private; later ones
/// load the same, and nothing temporary is left behind.
#[test]
fn the_salt_is_made_once() {
    let dir = tempfile::tempdir().unwrap();
    let made = load_or_create(dir.path()).unwrap();
    let path = dir.path().join(SALT_FILE);
    assert_eq!(std::fs::read(&path).unwrap(), made.0);
    platform::check_private(&File::open(&path).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
    }
    assert_eq!(load_or_create(dir.path()).unwrap().0, made.0);
    assert_eq!(load(dir.path()).unwrap().0, made.0);
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 1, "{files:?}");
}

/// A listener that makes its salt after another's is in place takes the
/// one in place, not its own.
#[test]
fn a_late_maker_takes_the_salt_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let first = load_or_create(dir.path()).unwrap();
    let late = create(dir.path()).unwrap();
    assert_eq!(late.0, first.0);
    assert_eq!(load(dir.path()).unwrap().0, first.0);
}

/// Two listeners starting together agree on one salt, the one in the
/// file, every time.
#[test]
fn racing_listeners_agree_on_one_salt() {
    for _ in 0..50 {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let start = Arc::new(Barrier::new(2));
        let racers: Vec<_> = (0..2)
            .map(|_| {
                let (dir, start) = (Arc::clone(&dir), Arc::clone(&start));
                std::thread::spawn(move || {
                    start.wait();
                    load_or_create(dir.path()).unwrap().0
                })
            })
            .collect();
        let salts: Vec<_> = racers.into_iter().map(|r| r.join().unwrap()).collect();
        assert_eq!(salts[0], salts[1]);
        assert_eq!(load(dir.path()).unwrap().0, salts[0]);
    }
}

/// A salt file of the wrong length is refused as corrupt, and left as it
/// is: a new salt would orphan a running listener's name.
#[test]
fn a_corrupt_salt_is_refused_not_replaced() {
    for length in [0, SALT_LEN - 1, SALT_LEN + 1] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SALT_FILE);
        let bytes = vec![9; length];
        write_private(&path, &bytes);
        let error = AttachError::from(load_or_create(dir.path()).unwrap_err()).to_string();
        assert!(error.contains("is corrupt"), "{length}: {error}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "left as it was");
    }
}

/// A directory in the salt's place is refused, never followed or
/// replaced.
#[test]
fn a_salt_that_is_not_a_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SALT_FILE);
    std::fs::create_dir(&path).unwrap();
    let error = load_or_create(dir.path()).unwrap_err();
    assert!(matches!(error, SaltError::Refused(_)), "{error:?}");
    assert!(path.is_dir());
}

/// A salt file another user may read is refused, and left as it is.
#[test]
fn a_salt_others_may_read_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SALT_FILE);
    open_to_everyone(&path, &[5; SALT_LEN]);
    let error = AttachError::from(load_or_create(dir.path()).unwrap_err()).to_string();
    assert!(error.contains("not this user's alone"), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), [5; SALT_LEN]);
}

/// A salt file at `path` holding `bytes` that everyone may read.
#[cfg(unix)]
fn open_to_everyone(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt as _;
    write_private(path, bytes);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// A salt file at `path` holding `bytes` that everyone may read.
#[cfg(windows)]
fn open_to_everyone(path: &Path, bytes: &[u8]) {
    use super::super::win32::{self, Descriptor};
    let own = win32::own_sid().unwrap();
    let descriptor = Descriptor::from_sddl(&format!("O:{own}D:P(A;;FA;;;{own})(A;;FR;;;WD)"));
    let mut file = win32::create_file(path, &descriptor.unwrap()).unwrap();
    file.write_all(bytes).unwrap();
}

/// A file of another user's is refused, as is one its group or others
/// may read or write; a test can't make a file of another uid, so the
/// rule is checked on made-up owners.
#[cfg(unix)]
#[test]
fn a_salt_of_another_user_is_refused() {
    platform::private(1000, 0o100_600, 1000).unwrap();
    let error = platform::private(1001, 0o100_600, 1000).unwrap_err();
    assert!(error.to_string().contains("owned by uid 1001"), "{error}");
    for mode in [0o640, 0o604, 0o620, 0o602] {
        let error = platform::private(1000, mode, 1000).unwrap_err();
        assert!(error.to_string().contains("mode"), "{mode:o}: {error}");
    }
}

/// A salt symbolic link is refused, never followed, even to a private
/// file of the right length.
#[cfg(unix)]
#[test]
fn a_salt_link_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    write_private(&real, &[3; SALT_LEN]);
    std::os::unix::fs::symlink(&real, dir.path().join(SALT_FILE)).unwrap();
    let error = load_or_create(dir.path()).unwrap_err();
    assert!(matches!(error, SaltError::Refused(_)), "{error:?}");
}

/// The workspace check: a directory only this user, SYSTEM and the
/// Administrators group may list passes, as does one whose grant to
/// everyone only passes to what it holds; one everyone may list doesn't.
#[cfg(windows)]
#[test]
fn a_workspace_others_may_list_is_refused() {
    use super::super::win32::{self, Descriptor};
    let own = win32::own_sid().unwrap();
    let root = tempfile::tempdir().unwrap();
    win32::check_private_dir(root.path(), &own).unwrap();
    let made = |name: &str, sddl: String| {
        let path = root.path().join(name);
        win32::create_dir(&path, &Descriptor::from_sddl(&sddl).unwrap()).unwrap();
        path
    };
    let private = made(
        "private",
        format!("D:P(A;OICI;FA;;;{own})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
    );
    win32::check_private_dir(&private, &own).unwrap();
    let passed_on = made(
        "passed-on",
        format!("D:P(A;OICI;FA;;;{own})(A;OICIIO;FR;;;WD)"),
    );
    win32::check_private_dir(&passed_on, &own).unwrap();
    let open = made("open", format!("D:P(A;OICI;FA;;;{own})(A;;FR;;;WD)"));
    let error = win32::check_private_dir(&open, &own)
        .unwrap_err()
        .to_string();
    assert!(error.contains("S-1-1-0"), "{error}");
    let users = made("users", format!("D:P(A;OICI;FA;;;{own})(A;;0x1;;;BU)"));
    let error = win32::check_private_dir(&users, &own)
        .unwrap_err()
        .to_string();
    assert!(error.contains("S-1-5-32-545"), "{error}");
}
