//! The salt file's and the workspace's access rules on their own, over
//! made-up entries: who may own them, which entries count and which
//! rights the workspace keeps from others (#2219).

use super::*;

/// A user's SID, S-1-5-21-1001: the test's "own" user.
fn own() -> Sid {
    Sid(vec![1, 2, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 0xE9, 0x03, 0, 0])
}

/// Everyone (S-1-1-0).
fn everyone() -> Sid {
    Sid(vec![1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0])
}

/// The local Users group (S-1-5-32-545).
fn users() -> Sid {
    Sid(vec![1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x21, 0x02, 0, 0])
}

/// An entry of `kind` granting or denying `mask` to `sid`.
fn ace(kind: u8, flags: u8, mask: u32, sid: Sid) -> Ace {
    Ace {
        kind,
        flags,
        mask: Some(mask),
        sid: Some(sid),
    }
}

/// An allow entry of `mask` for `sid`.
fn allow(mask: u32, sid: Sid) -> Ace {
    ace(ACCESS_ALLOWED, 0, mask, sid)
}

/// An entry of a kind with another layout: a callback entry, say.
fn unknown() -> Ace {
    Ace {
        kind: 9,
        flags: 0,
        mask: None,
        sid: None,
    }
}

/// The owner, SYSTEM and Administrators may hold the workspace; nobody
/// else may own it.
#[test]
fn the_workspace_owner_must_be_trusted() {
    let list = [allow(GENERIC_ALL, own())];
    private_dir(&own(), &list, &own()).unwrap();
    private_dir(&Sid(SYSTEM.to_vec()), &list, &own()).unwrap();
    private_dir(&Sid(ADMINISTRATORS.to_vec()), &list, &own()).unwrap();
    let error = private_dir(&users(), &list, &own()).unwrap_err();
    assert!(error.contains("owned by Users (S-1-5-32-545)"), "{error}");
}

/// Reading, writing alone, or any right in the forbidden set is refused
/// for another user; rights outside it, deny entries and entries that
/// only pass to what the directory holds are not.
#[test]
fn the_workspace_keeps_others_from_reading_and_changing_it() {
    let check = |entry: Ace| private_dir(&own(), &[allow(GENERIC_ALL, own()), entry], &own());
    for mask in [
        GENERIC_READ,
        GENERIC_WRITE,
        FILE_LIST_DIRECTORY,
        FILE_ADD_FILE,
        FILE_ADD_SUBDIRECTORY,
        FILE_DELETE_CHILD,
        DELETE,
        WRITE_DAC,
        WRITE_OWNER,
    ] {
        let error = check(allow(mask, users())).unwrap_err();
        assert!(error.contains("Users (S-1-5-32-545)"), "{mask:#x}: {error}");
    }
    // Traversing and reading attributes give nothing away.
    check(allow(0x20 | 0x80 | 0x0010_0000, everyone())).unwrap();
    check(ace(ACCESS_DENIED, 0, GENERIC_ALL, everyone())).unwrap();
    check(ace(
        ACCESS_ALLOWED,
        INHERIT_ONLY_ACE as u8,
        GENERIC_ALL,
        everyone(),
    ))
    .unwrap();
    check(allow(GENERIC_ALL, Sid(SYSTEM.to_vec()))).unwrap();
    check(allow(GENERIC_ALL, Sid(ADMINISTRATORS.to_vec()))).unwrap();
    let error = check(unknown()).unwrap_err();
    assert!(error.contains("an entry of kind 9"), "{error}");
}

/// The salt file must be the user's own, and may grant only the user,
/// SYSTEM and Administrators, as the workspace; any other grant, of any
/// right, or an entry of a kind not read is refused.
#[test]
fn the_salt_file_admits_only_trusted_entries() {
    let mine = allow(GENERIC_ALL, own());
    private_file(&own(), &[mine], &own()).unwrap();
    let error = private_file(&users(), &[allow(GENERIC_ALL, own())], &own()).unwrap_err();
    assert!(error.contains("owned by Users"), "{error}");
    let trusted = [
        allow(GENERIC_ALL, own()),
        allow(GENERIC_ALL, Sid(SYSTEM.to_vec())),
        allow(GENERIC_ALL, Sid(ADMINISTRATORS.to_vec())),
        ace(ACCESS_DENIED, 0, GENERIC_ALL, everyone()),
    ];
    private_file(&own(), &trusted, &own()).unwrap();
    for entry in [
        allow(0x0010_0000, everyone()),
        allow(GENERIC_READ, users()),
        unknown(),
    ] {
        let list = [allow(GENERIC_ALL, own()), entry];
        assert!(private_file(&own(), &list, &own()).is_err(), "{list:?}");
    }
}

/// Well-known SIDs are named in messages, others shown as they are.
#[test]
fn well_known_sids_are_named() {
    assert_eq!(shown(&everyone()), "Everyone (S-1-1-0)");
    assert_eq!(shown(&Sid(SYSTEM.to_vec())), "SYSTEM (S-1-5-18)");
    assert_eq!(shown(&own()), "S-1-5-21-1001");
}

/// A workspace that is a junction is refused, not followed.
#[test]
fn a_workspace_junction_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = root.path().join("link");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&real)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let own = own_sid().unwrap();
    check_private_dir(&real, &own).unwrap();
    let error = check_private_dir(&link, &own).unwrap_err();
    assert!(error.contains("junction"), "{error}");
}
