//! The named pipe's guards on their own: its instances' options, and the
//! descriptor the kernel ends up holding. Its name is tested with the
//! salt (`salt_tests.rs`).

use super::*;
use crate::attach::salt::{Salt, pipe_name};

/// The access every allowed entry grants: all file access.
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

/// The label the descriptor carries: medium integrity (S-1-16-8192), and
/// no write or read up (a mandatory label entry, type 0x11, mask 0x3).
fn medium_label() -> win32::Entry {
    let medium = Sid(vec![1, 1, 0, 0, 0, 0, 0, 16, 0x00, 0x20, 0, 0]);
    (0x11, 0x3, medium)
}

/// A pipe name of this test's own.
fn unique(tag: &str) -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let mut salt = [0; 32];
    getrandom::fill(&mut salt).unwrap();
    let name = pipe_name(&Salt(salt), dir.path());
    PathBuf::from(format!("{}-{tag}", name.display()))
}

/// A SID shows in its usual form.
#[test]
fn a_sid_shows_as_usual() {
    let system = Sid(vec![1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0]);
    assert_eq!(system.to_string(), "S-1-5-18");
    let user = Sid(vec![1, 2, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 0xE9, 0x03, 0, 0]);
    assert_eq!(user.to_string(), "S-1-5-21-1001");
    let own = win32::own_sid().unwrap().to_string();
    assert!(own.starts_with("S-1-"), "{own}");
}

/// Every instance refuses remote clients, and only the first claims the
/// name. Tokio's options say what they will ask of the system; a test
/// can't make a remote client.
#[test]
fn every_instance_refuses_remote_clients() {
    let first = format!("{:?}", options(true));
    assert!(first.contains("reject_remote_clients: true"), "{first}");
    assert!(first.contains("first_pipe_instance: true"), "{first}");
    let later = format!("{:?}", options(false));
    assert!(later.contains("reject_remote_clients: true"), "{later}");
    assert!(later.contains("first_pipe_instance: false"), "{later}");
}

/// The kernel holds the descriptor as made, on the first instance and on
/// a later one: owned by this user, one entry, allowing this user, and a
/// medium label that keeps lower-integrity processes out.
#[tokio::test]
async fn the_descriptor_grants_only_its_owner() {
    let own = win32::own_sid().unwrap();
    let descriptor = Descriptor::owner_only(&own).unwrap();
    let name = unique("descriptor");
    let first = descriptor.create(&options(true), &name).unwrap();
    let later = descriptor.create(&options(false), &name).unwrap();
    for instance in [&first, &later] {
        let security = win32::security_of(instance).unwrap();
        assert_eq!(security.owner, own);
        assert_eq!(security.access, [(0, FILE_ALL_ACCESS, own.clone())]);
        assert_eq!(security.label, [medium_label()], "{security:?}");
    }
    // The first instance claims the name: a second first is refused.
    let error = descriptor.create(&options(true), &name).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
}
