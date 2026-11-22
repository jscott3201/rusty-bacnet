//! A peer-written Description on an Audit Log (#2407): committed with the
//! database guard dropped before the write is answered, carried by every
//! later commit, served by a log rebuilt on the same storage, refused with
//! DEVICE / OPERATIONAL_PROBLEM when its commit fails, and never committed
//! for a NULL.

use super::audit_log_buffer_wire_tests::{
    HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, attempt, audit_log, fixture, log_attempts,
    write, write_error,
};
use super::mutation_list_wire_tests::wire;
use super::*;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

fn text(text: &str) -> PropertyValue {
    PropertyValue::CharacterString(text.into())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_written_description_is_committed_off_the_guard_and_survives_a_rebuild() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        async move { write(&fixture, P::DESCRIPTION, text("lobby")).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the commit started");
    // While the commit runs the database answers readers and writers, and
    // the log still serves its old Description.
    let served = tokio::time::timeout(WAIT, fixture.read(audit_log(), P::DESCRIPTION)).await;
    let writable = tokio::time::timeout(WAIT, fixture.db.write()).await.is_ok();
    let answered_early = sending.is_finished();
    go.send(()).unwrap();
    assert_eq!(sending.await.unwrap(), SIMPLE_ACK_WRITE);
    assert_eq!(served.ok(), Some(text("")));
    assert!(writable, "the database was held while the log committed");
    assert!(!answered_early);
    assert_eq!(storage.committed().description.as_deref(), Some("lobby"));
    drop(fixture);

    // The rebuilt log serves it, and its own commits (three Enable changes)
    // keep it. With the release dropped, no commit waits any more.
    drop(go);
    let rebuilt = super::audit_log_buffer_wire_tests::fixture(&storage).await;
    assert_eq!(
        rebuilt.read(audit_log(), P::DESCRIPTION).await,
        text("lobby")
    );
    assert_eq!(storage.committed().description.as_deref(), Some("lobby"));
}

#[tokio::test]
async fn a_description_that_cannot_be_committed_is_refused_with_the_old_one_kept() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    assert_eq!(
        write(&fixture, P::DESCRIPTION, text("lobby")).await,
        SIMPLE_ACK_WRITE
    );
    storage.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        write(&fixture, P::DESCRIPTION, text("hall")).await,
        write_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(
        fixture.read(audit_log(), P::DESCRIPTION).await,
        text("lobby")
    );
    assert_eq!(storage.committed().description.as_deref(), Some("lobby"));
}

#[tokio::test]
async fn a_null_description_commits_nothing() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let commits = storage.commits.load(Ordering::SeqCst);
    assert_eq!(
        write(&fixture, P::DESCRIPTION, PropertyValue::Null).await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
    assert_eq!(storage.committed().description, None);
    // Past the cap: refused before anything is committed.
    assert_eq!(
        write(&fixture, P::DESCRIPTION, text(&"d".repeat(16 * 1024 + 1))).await,
        write_error(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
    );
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}

/// A WritePropertyMultiple of Description and Enable commits once.
#[tokio::test]
async fn a_write_property_multiple_with_description_commits_once() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let commits = storage.commits.load(Ordering::SeqCst);
    let request = log_attempts(
        attempt(P::DESCRIPTION, text("lobby")),
        attempt(P::ENABLE, PropertyValue::Boolean(true)),
    );
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WPM);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 1);
    let committed = storage.committed();
    assert_eq!(committed.description.as_deref(), Some("lobby"));
    assert!(committed.log_enable);
    assert_eq!(
        fixture.read(audit_log(), P::DESCRIPTION).await,
        text("lobby")
    );
}
