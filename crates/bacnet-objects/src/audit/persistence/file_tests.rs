//! The Audit Log file backend: slot selection and the I/O failures a commit
//! or a load reports (#2183).

use super::*;
use bacnet_types::enums::ObjectType;
use std::sync::atomic::{AtomicU64, Ordering};

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap()
}

fn snapshot() -> AuditLogSnapshot {
    AuditLogSnapshot {
        object_identifier: oid(),
        generation: 1,
        capacity: 1,
        log_enable: true,
        total_record_count: 0,
        records: Vec::new(),
        completed_receipts: Vec::new(),
    }
}

type SlotRead = Result<Option<Vec<u8>>, SlotReadFailure>;

fn io_failure() -> SlotRead {
    Err(SlotReadFailure::Fatal(Error::Transport(
        std::io::Error::other("injected slot read failure"),
    )))
}

/// A slot read that found `snapshot`.
fn valid(snapshot: AuditLogSnapshot) -> SlotRead {
    Ok(Some(audit_log::encode(&snapshot).unwrap()))
}

/// A fresh, empty directory of this test's own.
fn temp_dir(label: &str) -> PathBuf {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rusty-bacnet-audit-file-{label}-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A base name longer than any file system takes for one path component.
fn overlong_name() -> String {
    "a".repeat(300)
}

#[test]
fn audit_slot_selection_never_masks_io_failure_with_valid_peer() {
    assert!(matches!(
        choose_slot([io_failure(), valid(snapshot())], oid()),
        Err(Error::Transport(_))
    ));
    assert!(matches!(
        choose_slot([valid(snapshot()), io_failure()], oid()),
        Err(Error::Transport(_))
    ));
}

/// Slot order decides which refusal a load reports: slot 0 refused as
/// another object's goes ahead of slot 1's read failure, and slot 0's read
/// failure ahead of slot 1's refusal.
#[test]
fn a_refused_slot_0_is_reported_ahead_of_slot_1s_read_failure() {
    let mut elsewhere = snapshot();
    elsewhere.object_identifier = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 2).unwrap();
    assert!(matches!(
        choose_slot([valid(elsewhere.clone()), io_failure()], oid()),
        Err(Error::Encoding(message)) if message.contains("object identity")
    ));
    assert!(matches!(
        choose_slot([io_failure(), valid(elsewhere)], oid()),
        Err(Error::Transport(_))
    ));
}

#[test]
fn audit_slot_selection_requires_identical_same_generation_snapshots() {
    let expected = snapshot();
    let selected = choose_slot([valid(expected.clone()), valid(expected.clone())], oid())
        .unwrap()
        .unwrap();
    assert_eq!(selected, expected);

    let mut divergent = snapshot();
    divergent.log_enable = false;
    assert!(matches!(
        choose_slot([valid(snapshot()), valid(divergent)], oid()),
        Err(Error::Encoding(message)) if message.contains("same generation")
    ));
}

#[test]
fn a_commit_whose_directory_cannot_be_made_fails_and_writes_nothing() {
    let dir = temp_dir("blocked-dir");
    // A regular file where the slots' directory should go.
    std::fs::write(dir.join("blocker"), b"not a directory").unwrap();
    let storage = FileAuditLogPersistence::new(dir.join("blocker").join("state")).unwrap();
    assert!(matches!(
        storage.commit(&snapshot()),
        Err(Error::Transport(_))
    ));
    assert_eq!(
        std::fs::read(dir.join("blocker")).unwrap(),
        b"not a directory"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_slot_that_cannot_be_opened_fails_the_commit_and_the_load() {
    let dir = temp_dir("overlong");
    let storage = FileAuditLogPersistence::new(dir.join(overlong_name())).unwrap();
    assert!(matches!(
        storage.commit(&snapshot()),
        Err(Error::Transport(_))
    ));
    // Not a missing slot: a load that cannot open one is fatal. Unix
    // answers ENAMETOOLONG; Windows may answer an overlong name as a path
    // not found, a missing slot, so there the next test shows the rule.
    let loaded = storage.load(oid());
    if cfg!(unix) {
        assert!(matches!(loaded, Err(Error::Transport(_))), "{loaded:?}");
    } else {
        assert!(
            matches!(loaded, Err(Error::Transport(_)) | Ok(None)),
            "{loaded:?}"
        );
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_slot_that_is_a_directory_fails_the_commit_and_the_load() {
    let dir = temp_dir("directory-slot");
    let storage = FileAuditLogPersistence::new(dir.join("state")).unwrap();
    let [slot0, slot1] = storage.slot_paths();
    std::fs::create_dir(&slot1).unwrap();
    // Generation 1 goes to slot 1, which can't be written as a file.
    assert!(matches!(
        storage.commit(&snapshot()),
        Err(Error::Transport(_))
    ));
    assert!(!slot0.exists() && slot1.is_dir());
    // Opening a directory fails on Windows and reading one fails on Unix:
    // either way the load is fatal, not a missing slot.
    assert!(matches!(storage.load(oid()), Err(Error::Transport(_))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_slot_past_the_size_cap_is_refused_and_its_valid_peer_still_loads() {
    let dir = temp_dir("oversized");
    let storage = FileAuditLogPersistence::new(dir.join("state")).unwrap();
    let [slot0, slot1] = storage.slot_paths();
    // Sparse, so the file costs no disk space.
    let oversized = File::create(&slot1).unwrap();
    oversized.set_len(MAX_SNAPSHOT_BYTES as u64 + 1).unwrap();
    drop(oversized);
    assert!(matches!(
        storage.load(oid()),
        Err(Error::Encoding(message)) if message.contains("exceeds")
    ));

    // Generation 2 lands in slot 0; slot 1 stays oversized and is passed over.
    let mut newer = snapshot();
    newer.generation = 2;
    storage.commit(&newer).unwrap();
    assert!(slot0.exists());
    assert_eq!(storage.load(oid()).unwrap(), Some(newer));
    let _ = std::fs::remove_dir_all(dir);
}
