//! The filesystem guards on their own: ownership, narrowing on one
//! handle, symbolic links, the re-check after bind, and the lock.

use super::*;
use crate::attach::own_uid;

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o777
}

fn dir_at(mode: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(mode)).unwrap();
    dir
}

/// A workspace owned by another user is refused even at mode 0700 (a test
/// can't own a directory as someone else, so it asks as another uid).
#[test]
fn another_users_workspace_is_refused() {
    let dir = dir_at(0o700);
    let uid = own_uid();
    open_workspace(dir.path(), uid).unwrap();
    let error = open_workspace(dir.path(), uid.wrapping_add(1))
        .err()
        .unwrap();
    assert!(error.contains(&format!("belongs to uid {uid}")), "{error}");
}

/// A looser workspace of this user's is narrowed to 0700; through a
/// symbolic link nothing is opened or narrowed; a file is refused.
#[test]
fn a_loose_workspace_is_narrowed_but_never_through_a_link() {
    for loose in [0o755, 0o750, 0o711, 0o701, 0o770] {
        let dir = dir_at(loose);
        open_workspace(dir.path(), own_uid()).unwrap();
        assert_eq!(mode(dir.path()), 0o700, "{loose:o}");
    }
    let root = dir_at(0o700);
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let error = open_workspace(&link, own_uid()).err().unwrap();
    assert!(error.contains("symbolic link"), "{error}");
    assert_eq!(mode(&real), 0o755, "the target is untouched");
    let file = root.path().join("file");
    fs::write(&file, "").unwrap();
    let error = open_workspace(&file, own_uid()).err().unwrap();
    assert!(error.contains("not a directory"), "{error}");
}

/// The re-check after bind notices a workspace swapped for another
/// directory, or loosened.
#[test]
fn the_recheck_notices_a_swap_or_a_looser_mode() {
    let root = dir_at(0o700);
    let dir = root.path().join("site");
    fs::create_dir(&dir).unwrap();
    let held = open_workspace(&dir, own_uid()).unwrap();
    held.recheck(&dir, own_uid()).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(held.recheck(&dir, own_uid()).is_err(), "loosened");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(&dir, root.path().join("moved")).unwrap();
    fs::create_dir(&dir).unwrap();
    let error = held.recheck(&dir, own_uid()).unwrap_err();
    assert!(error.contains("replaced"), "{error}");
}

/// A socket just bound is made 0600; a symbolic link the bind followed is
/// refused and the socket made at its target removed, the link kept.
#[test]
fn only_the_socket_just_bound_is_kept() {
    let dir = dir_at(0o700);
    let path = dir.path().join("mcp.sock");
    let _bound = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let identity = own_socket(&path).unwrap();
    assert_eq!(mode(&path), 0o600);
    let meta = fs::symlink_metadata(&path).unwrap();
    assert_eq!(identity, (meta.dev(), meta.ino()));

    let elsewhere = dir.path().join("elsewhere.sock");
    let _followed = std::os::unix::net::UnixListener::bind(&elsewhere).unwrap();
    let link = dir.path().join("link.sock");
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    let error = own_socket(&link).unwrap_err();
    assert!(error.contains("not the socket just bound"), "{error}");
    assert!(!elsewhere.exists(), "what the bind made is removed");
    assert!(fs::symlink_metadata(&link).is_ok(), "the link is left");
}

/// One listener's lock at a time; a probe shows whether one is held,
/// without refusing another probe.
#[tokio::test(start_paused = true)]
async fn the_lock_admits_one_listener() {
    let dir = dir_at(0o700);
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

/// A listener starting while a probe holds its shared lock waits for it
/// and takes the lock once it goes; a lock held throughout refuses it.
#[tokio::test(start_paused = true)]
async fn a_listener_waits_out_a_probe() {
    let dir = dir_at(0o700);
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

/// A path longer than a socket takes is refused before any bind.
#[test]
fn a_path_too_long_for_a_socket_is_refused() {
    let short = Path::new("/tmp/mcp.sock");
    check_length(short).unwrap();
    let long = Path::new("/tmp")
        .join("a".repeat(MAX_PATH))
        .join("mcp.sock");
    let error = check_length(&long).unwrap_err();
    assert!(error.contains(&format!("the {MAX_PATH}")), "{error}");
}
