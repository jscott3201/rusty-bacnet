//! The application's purge of an Audit Log (#1238): the commit runs while
//! the database stays available, the log serves the purge only once storage
//! holds it, and a notification batch lands whole on one side of it.

use bacnet_types::error::RemoteError;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, mpsc};
use std::time::Duration;

use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::audit::{
    AuditLogQueryAck, AuditLogQueryRequest, BACnetAuditLogQueryParameters,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::BACnetAuditLogDatum;
use bacnet_types::enums::{AuditOperation, BACnetSuccessFilter};

use super::super::audit_notification_tests::{
    dispatch, dispatch_confirmed, notification, oid, request_bytes,
};
use super::super::clock::clocked_test_database;
use super::super::test_transport::TestTransport;
use super::super::*;

const WAIT: Duration = Duration::from_secs(10);

/// Audit Log storage whose commits can fail, or wait until the test lets
/// each one go.
#[derive(Default)]
struct HeldStorage {
    snapshot: StdMutex<Option<AuditLogSnapshot>>,
    commits: AtomicUsize,
    fail: AtomicBool,
    hold: StdMutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl HeldStorage {
    /// Make every later commit report on the first channel and wait for a
    /// message on the second.
    fn hold(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started, started_rx) = mpsc::channel();
        let (go, go_rx) = mpsc::channel();
        *self.hold.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    }

    fn committed(&self) -> AuditLogSnapshot {
        self.snapshot.lock().unwrap().clone().unwrap()
    }
}

impl AuditLogPersistence for HeldStorage {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        if let Some((started, go)) = &*self.hold.lock().unwrap() {
            let _ = started.send(());
            let _ = go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Transport(std::io::Error::other(
                "injected commit failure",
            )));
        }
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn audit_log() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_LOG, 7)
}

async fn server(storage: &Arc<HeldStorage>) -> Arc<BACnetServer<TestTransport>> {
    let mut db = clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AuditLogObject::new(7, "audit", 16, Arc::clone(storage) as _).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    Arc::new(
        BACnetServer::generic_builder()
            .transport(TestTransport::new())
            .database(db)
            .enable_event_enrollment(false)
            .build()
            .await
            .unwrap(),
    )
}

async fn stop(server: Arc<BACnetServer<TestTransport>>) {
    let mut server = Arc::into_inner(server).expect("no task still holds the server");
    server.stop().await.unwrap();
}

/// The server configuration that lets the log take notifications.
fn receiving() -> ServerConfig {
    ServerConfig {
        audit_notification_sink: Some(audit_log()),
        policy: Some(Arc::new(
            crate::FnPolicy::new().audit_notification(|_| true),
        )),
        ..ServerConfig::default()
    }
}

/// Send one confirmed notification, with its own invoke ID, to the log.
async fn notify(db: &Arc<RwLock<ObjectDatabase>>, invoke_id: u8) -> Apdu {
    let mut report = notification(AuditOperation::WRITE);
    report.invoke_id = Some(invoke_id);
    dispatch(
        db,
        &receiving(),
        &Arc::new(ConfirmedRequestTracker::default()),
        invoke_id,
        &[0x10],
        None,
        request_bytes(vec![report]),
    )
    .await
    .unwrap()
}

/// The sequence numbers of the records the log holds, oldest first.
async fn held(db: &Arc<RwLock<ObjectDatabase>>) -> Vec<u64> {
    db.read()
        .await
        .get(&audit_log())
        .unwrap()
        .audit_log_storage_internal()
        .unwrap()
        .retained_records()
        .iter()
        .map(|record| record.sequence_number)
        .collect()
}

/// The log-status flags of the oldest record the log holds.
async fn first_status(db: &Arc<RwLock<ObjectDatabase>>) -> LogStatus {
    let db = db.read().await;
    let records = db
        .get(&audit_log())
        .unwrap()
        .audit_log_storage_internal()
        .unwrap()
        .retained_records();
    match &records.front().expect("a record is held").record.datum {
        BACnetAuditLogDatum::LogStatus(status) => *status,
        other => panic!("the first record is not a log-status record: {other:?}"),
    }
}

/// The sequence numbers an AuditLogQuery on the wire returns for every
/// report on target Device 2, newest first.
async fn queried(db: &Arc<RwLock<ObjectDatabase>>) -> Vec<u64> {
    let request = AuditLogQueryRequest {
        audit_log: audit_log(),
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 2),
            target_device_address: None,
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::ALL,
        },
        start_at_sequence_number: None,
        requested_count: 100,
    };
    let mut service_request = BytesMut::new();
    request.try_encode(&mut service_request).unwrap();
    let response = dispatch_confirmed(
        db,
        &ServerConfig::default(),
        &Arc::new(ConfirmedRequestTracker::default()),
        &[0x11],
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::AUDIT_LOG_QUERY,
            service_request: service_request.freeze(),
        },
    )
    .await
    .unwrap();
    let Apdu::ComplexAck(ack) = response else {
        panic!("expected an AuditLogQuery ComplexAck, got {response:?}");
    };
    AuditLogQueryAck::decode(&ack.service_ack)
        .unwrap()
        .records
        .iter()
        .map(|record| record.sequence_number)
        .collect()
}

async fn started(started: mpsc::Receiver<()>) -> mpsc::Receiver<()> {
    tokio::task::spawn_blocking(move || {
        started.recv_timeout(WAIT).expect("a commit started");
        started
    })
    .await
    .unwrap()
}

/// Spawn `future`, a request on `db`, and return once it has found the log
/// busy: it then waits for the staged commit held ahead of it.
async fn spawn_until_busy<F>(db: &Arc<RwLock<ObjectDatabase>>, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let before = super::super::durable_writes::busy_waits(db);
    let task = tokio::spawn(future);
    tokio::time::timeout(WAIT, async {
        while super::super::durable_writes::busy_waits(db) == before {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the request found the log busy");
    assert!(!task.is_finished(), "the request did not wait for the log");
    task
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_purge_commits_while_the_database_stays_available_and_lands_once_durable() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    let db = Arc::clone(server.database());
    for invoke_id in 1..=3 {
        assert!(matches!(notify(&db, invoke_id).await, Apdu::SimpleAck(_)));
    }
    assert_eq!(queried(&db).await, [3, 2, 1]);
    let generation = storage.committed().generation;
    let commits = storage.commits.load(Ordering::SeqCst);

    let (started_rx, go) = storage.hold();
    let purging = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.purge_audit_log(&audit_log()).await }
    });
    let _started = started(started_rx).await;
    // While the commit runs the database answers readers and writers, and
    // the log still serves its committed records.
    let records = tokio::time::timeout(WAIT, held(&db)).await;
    let writable = tokio::time::timeout(WAIT, db.write()).await.is_ok();
    let purged_early = purging.is_finished();
    go.send(()).unwrap();
    purging.await.unwrap().unwrap();

    assert_eq!(records.ok(), Some(vec![1, 2, 3]), "the database was held");
    assert!(writable, "the database was held while the purge committed");
    assert!(!purged_early);
    assert_eq!(held(&db).await, [4]);
    assert_eq!(first_status(&db).await, LogStatus::BUFFER_PURGED);
    assert!(queried(&db).await.is_empty());
    let committed = storage.committed();
    assert_eq!(committed.generation, generation + 1);
    assert_eq!(committed.total_record_count, 4);
    // The purge took its staged commit and committed nothing more.
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 1);
    drop(go);
    stop(server).await;
}

#[tokio::test]
async fn a_purge_that_cannot_be_committed_leaves_the_log_as_it_was() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    let db = Arc::clone(server.database());
    assert!(matches!(notify(&db, 1).await, Apdu::SimpleAck(_)));
    let before = storage.committed();
    storage.fail.store(true, Ordering::SeqCst);
    assert_error(
        server.purge_audit_log(&audit_log()).await.unwrap_err(),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(held(&db).await, [1]);
    assert_eq!(queried(&db).await, [1]);
    assert_eq!(storage.committed(), before);
    // Once storage is back the purge goes through.
    storage.fail.store(false, Ordering::SeqCst);
    server.purge_audit_log(&audit_log()).await.unwrap();
    assert_eq!(held(&db).await, [2]);
    stop(server).await;
}

#[tokio::test]
async fn only_a_running_server_purges_and_only_an_audit_log() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    assert_error(
        server
            .purge_audit_log(&oid(ObjectType::AUDIT_LOG, 8))
            .await
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    );
    assert_error(
        server
            .purge_audit_log(&oid(ObjectType::BINARY_VALUE, 1))
            .await
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    let mut server = Arc::into_inner(server).unwrap();
    server.stop().await.unwrap();
    assert!(server.purge_audit_log(&audit_log()).await.is_err());
    assert!(held(server.database()).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_batch_committing_when_a_purge_is_asked_for_lands_first_and_is_purged() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    let db = Arc::clone(server.database());
    let (started_rx, go) = storage.hold();
    let sending = tokio::spawn({
        let db = Arc::clone(&db);
        async move { notify(&db, 1).await }
    });
    let started_rx = started(started_rx).await;
    // The batch is staged and its commit held; the purge asked for now
    // finds the log busy, waits for the batch, and then builds on it.
    let purging = spawn_until_busy(&db, {
        let server = Arc::clone(&server);
        async move { server.purge_audit_log(&audit_log()).await }
    })
    .await;
    go.send(()).unwrap();
    let _started = started(started_rx).await;
    go.send(()).unwrap();
    assert!(matches!(sending.await.unwrap(), Apdu::SimpleAck(_)));
    purging.await.unwrap().unwrap();

    assert_eq!(held(&db).await, [2]);
    assert_eq!(first_status(&db).await, LogStatus::BUFFER_PURGED);
    assert!(queried(&db).await.is_empty());
    assert_eq!(storage.committed().total_record_count, 2);
    drop(go);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_batch_that_arrives_while_a_purge_commits_follows_the_purge_record() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    let db = Arc::clone(server.database());
    assert!(matches!(notify(&db, 1).await, Apdu::SimpleAck(_)));
    let (started_rx, go) = storage.hold();
    let purging = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.purge_audit_log(&audit_log()).await }
    });
    let started_rx = started(started_rx).await;
    // The purge is staged and its commit held; the batch sent now finds the
    // log busy, waits until the purge lands, then commits after it.
    let sending = spawn_until_busy(&db, {
        let db = Arc::clone(&db);
        async move { notify(&db, 2).await }
    })
    .await;
    go.send(()).unwrap();
    let _started = started(started_rx).await;
    go.send(()).unwrap();
    purging.await.unwrap().unwrap();
    assert!(matches!(sending.await.unwrap(), Apdu::SimpleAck(_)));

    assert_eq!(held(&db).await, [2, 3]);
    assert_eq!(first_status(&db).await, LogStatus::BUFFER_PURGED);
    assert_eq!(queried(&db).await, [3]);
    drop(go);
    stop(server).await;
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_purge_commits() {
    let storage = Arc::new(HeldStorage::default());
    let server = server(&storage).await;
    let (started_rx, go) = storage.hold();
    // The commit takes real time on the log's writer thread, as on a slow
    // disk, while a timer like a request's APDU timeout is pending.
    let releasing = std::thread::spawn(move || {
        started_rx.recv_timeout(WAIT).expect("the commit started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    server.purge_audit_log(&audit_log()).await.unwrap();
    releasing.join().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "virtual time jumped {:?} while the purge committed",
        start.elapsed()
    );
    assert!(!timer.is_finished());
    timer.abort();
    assert_eq!(held(server.database()).await, [1]);
    stop(server).await;
}
