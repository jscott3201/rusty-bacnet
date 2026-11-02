//! The run's nonce (#2237): a new name each run, the file published
//! private and replaced whole, withdrawn at shutdown, and the files
//! refused. These run on every platform; on Unix a 0600 mode stands in
//! for the owner-only descriptor. Its owner rule is the salt's
//! (`salt_tests.rs`, `win32_tests.rs`): both files are read by one
//! function.

use std::cell::Cell;
use std::fs::File;
use std::io::Write as _;

use super::*;
use crate::attach::salt::{SALT_LEN, Salt, pipe_name, platform};

/// The files in `dir`, by name.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The refusal `load` gives for `dir`.
fn refusal(dir: &Path) -> String {
    load(dir).unwrap_err().to_string()
}

/// Write `bytes` to a new private file at `path`, as the nonce is.
fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = platform::create_private(path).unwrap();
    file.write_all(bytes).unwrap();
}

/// Acceptance: two runs of a listener on one workspace, with its one
/// salt, give two names; the nonce is never shown.
#[test]
fn each_run_names_the_pipe_anew() {
    let dir = tempfile::tempdir().unwrap();
    let salt = Salt([1; SALT_LEN]);
    let (one, two) = (fresh().unwrap(), fresh().unwrap());
    assert_ne!(one.0, two.0);
    let name = pipe_name(&salt, &one, dir.path());
    assert_eq!(name, pipe_name(&salt, &Nonce(one.0), dir.path()));
    assert_ne!(name, pipe_name(&salt, &two, dir.path()), "the nonce counts");
    assert_eq!(format!("{one:?}"), "Nonce(..)", "never shown");
}

/// The nonce is published private, read back as it was, replaced whole
/// by the next run's with nothing temporary left, and withdrawn at
/// shutdown, after which `connect` hears that no TUI is listening.
#[test]
fn the_nonce_is_published_replaced_and_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(NONCE_FILE);
    let first = fresh().unwrap();
    publish(dir.path(), &first).unwrap();
    assert_eq!(load(dir.path()).unwrap().0, first.0);
    platform::check_private(&File::open(&path).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
    }
    let next = fresh().unwrap();
    publish(dir.path(), &next).unwrap();
    assert_eq!(load(dir.path()).unwrap().0, next.0, "the next run's");
    assert_eq!(files(dir.path()), [NONCE_FILE]);

    withdraw(dir.path());
    assert!(!path.exists());
    let error = refusal(dir.path());
    assert!(error.contains("no TUI is listening"), "{error}");
    assert!(error.contains(NONCE_FILE), "{error}");
    withdraw(dir.path());
}

/// A nonce of the wrong length is refused as corrupt.
#[test]
fn a_corrupt_nonce_is_refused() {
    for length in [0, NONCE_LEN - 1, NONCE_LEN + 1] {
        let dir = tempfile::tempdir().unwrap();
        write_private(&dir.path().join(NONCE_FILE), &vec![9; length]);
        let error = refusal(dir.path());
        assert!(error.contains("is corrupt"), "{length}: {error}");
    }
}

/// A directory in the nonce's place is refused, and can't be published
/// over.
#[test]
fn a_nonce_that_is_not_a_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(NONCE_FILE);
    std::fs::create_dir(&path).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(publish(dir.path(), &fresh().unwrap()).is_err());
    assert!(path.is_dir());
    assert_eq!(files(dir.path()), [NONCE_FILE], "nothing temporary left");
}

/// A nonce file another user may read is refused.
#[test]
fn a_nonce_others_may_read_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    open_to_everyone(&dir.path().join(NONCE_FILE), &[5; NONCE_LEN]);
    let error = refusal(dir.path());
    assert!(error.contains("not this user's alone"), "{error}");
}

/// A nonce file at `path` holding `bytes` that everyone may read.
#[cfg(unix)]
fn open_to_everyone(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt as _;
    write_private(path, bytes);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// A nonce file at `path` holding `bytes` that everyone may read.
#[cfg(windows)]
fn open_to_everyone(path: &Path, bytes: &[u8]) {
    use super::super::win32::{self, Descriptor};
    let own = win32::own_sid().unwrap();
    let descriptor = Descriptor::from_sddl(&format!("O:{own}D:P(A;;FA;;;{own})(A;;FR;;;WD)"));
    let mut file = win32::create_file(path, &descriptor.unwrap()).unwrap();
    file.write_all(bytes).unwrap();
}

/// A nonce symbolic link is refused, never followed; publishing replaces
/// the link itself and writes nothing through it.
#[cfg(unix)]
#[test]
fn a_nonce_link_is_refused_and_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    write_private(&real, &[3; NONCE_LEN]);
    let path = dir.path().join(NONCE_FILE);
    std::os::unix::fs::symlink(&real, &path).unwrap();
    let error = refusal(dir.path());
    assert!(error.contains("cannot open"), "{error}");
    let nonce = fresh().unwrap();
    publish(dir.path(), &nonce).unwrap();
    assert!(!std::fs::symlink_metadata(&path).unwrap().is_symlink());
    assert_eq!(std::fs::read(&real).unwrap(), [3; NONCE_LEN]);
    assert_eq!(load(dir.path()).unwrap().0, nonce.0);
}

/// A rename that fails for a moment is tried again, at most
/// [`PUBLISH_TRIES`] times with a pause between; one that keeps failing
/// says the TUI isn't listening and leaves nothing temporary; any other
/// failure isn't retried.
#[test]
fn a_failing_rename_is_retried_a_few_times() {
    let dir = tempfile::tempdir().unwrap();
    let (tries, pauses) = (Cell::new(0), Cell::new(0));
    let flaky = |from: &Path, to: &Path| {
        tries.set(tries.get() + 1);
        if tries.get() < PUBLISH_TRIES {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            std::fs::rename(from, to)
        }
    };
    let nonce = fresh().unwrap();
    publish_with(dir.path(), &nonce, flaky, || pauses.set(pauses.get() + 1)).unwrap();
    assert_eq!(
        (tries.get(), pauses.get()),
        (PUBLISH_TRIES, PUBLISH_TRIES - 1)
    );
    assert_eq!(load(dir.path()).unwrap().0, nonce.0);

    tries.set(0);
    let denied = |_: &Path, _: &Path| {
        tries.set(tries.get() + 1);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    };
    let error = publish_with(dir.path(), &fresh().unwrap(), denied, || ()).unwrap_err();
    assert!(error.to_string().contains("try again"), "{error}");
    assert_eq!(tries.get(), PUBLISH_TRIES);
    assert_eq!(load(dir.path()).unwrap().0, nonce.0, "the last one kept");
    assert_eq!(files(dir.path()), [NONCE_FILE], "nothing temporary left");

    tries.set(0);
    let gone = |_: &Path, _: &Path| {
        tries.set(tries.get() + 1);
        Err(io::Error::from(io::ErrorKind::NotFound))
    };
    publish_with(dir.path(), &fresh().unwrap(), gone, || ()).unwrap_err();
    assert_eq!(tries.get(), 1, "not retried");
}

/// Publishing clears the nonce's temporary files a crashed TUI left, and
/// nothing else.
#[test]
fn publishing_sweeps_stale_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir
        .path()
        .join(format!("{NONCE_FILE}.0123456789abcdef.tmp"));
    write_private(&stale, &[1; NONCE_LEN]);
    let other = dir.path().join("other.tmp");
    write_private(&other, b"keep");
    publish(dir.path(), &fresh().unwrap()).unwrap();
    assert!(!stale.exists());
    assert_eq!(files(dir.path()), [NONCE_FILE, "other.tmp"]);
}
