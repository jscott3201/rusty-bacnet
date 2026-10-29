//! Inbound Audit notifications commit off the database lock (#1270): the
//! commit runs on the Audit Log's writer thread while other requests read
//! and write the database, and a confirmed notification is acknowledged only
//! once its records and receipt are durable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, mpsc};
use std::time::Duration;

use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::AuditOperation;

use super::super::*;
use super::{FixedClock, count, dispatch, notification, oid, request_bytes};

/// Long enough for any wait here, including a reader that expects the
/// database while a commit runs: a loaded runner cannot fail it.
const WAIT: Duration = Duration::from_secs(10);

/// Audit Log storage whose commits can fail, or wait until the test lets
/// each one go.
#[derive(Default)]
struct HeldStorage {
    snapshot: StdMutex<Option<AuditLogSnapshot>>,
    fail: AtomicBool,
    hold: StdMutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl HeldStorage {
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
        Ok(())
    }
}

fn database(storage: Arc<HeldStorage>) -> Arc<RwLock<ObjectDatabase>> {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AuditLogObject::new(7, "audit", 16, storage).unwrap(),
    ))
    .unwrap();
    Arc::new(RwLock::new(db))
}

fn config(sink: ObjectIdentifier) -> ServerConfig {
    ServerConfig {
        audit_notification_sink: Some(sink),
        policy: Some(Arc::new(
            crate::FnPolicy::new().audit_notification(|_| true),
        )),
        ..ServerConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_confirmed_notification_commits_while_the_database_stays_available() {
    let storage = Arc::new(HeldStorage::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&storage));
    let generation = storage.committed().generation;
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let db = Arc::clone(&db);
        async move {
            dispatch(
                &db,
                &config(sink),
                &Arc::new(ConfirmedRequestTracker::default()),
                9,
                &[0x10],
                None,
                request_bytes(vec![notification(AuditOperation::WRITE)]),
            )
            .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the commit started");

    // While the commit runs the database answers readers and writers, the log
    // still serves its committed records, and nothing is acknowledged.
    let records = tokio::time::timeout(WAIT, count(&db, sink)).await;
    let writable = tokio::time::timeout(WAIT, db.write()).await.is_ok();
    let acknowledged_early = sending.is_finished();
    go.send(()).unwrap();
    let response = sending.await.unwrap().unwrap();

    assert_eq!(records.ok(), Some((0, 0)), "the database was held");
    assert!(writable, "the database was held while the log committed");
    assert!(!acknowledged_early);
    assert!(matches!(response, Apdu::SimpleAck(_)));
    // The records and the receipt were durable before the acknowledgment.
    let committed = storage.committed();
    assert_eq!(committed.generation, generation + 1);
    assert_eq!(committed.records.len(), 1);
    assert_eq!(committed.completed_receipts.len(), 1);
    assert_eq!(count(&db, sink).await, (1, 1));
}

#[tokio::test]
async fn a_confirmed_notification_whose_commit_fails_is_refused_and_stores_nothing() {
    let storage = Arc::new(HeldStorage::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&storage));
    storage.fail.store(true, Ordering::SeqCst);
    let response = dispatch(
        &db,
        &config(sink),
        &Arc::new(ConfirmedRequestTracker::default()),
        9,
        &[0x10],
        None,
        request_bytes(vec![notification(AuditOperation::WRITE)]),
    )
    .await
    .unwrap();
    // The storage error is the server's own trouble, not the sender's
    // (#1366): DEVICE / OPERATIONAL_PROBLEM, as a failed Log_Enable commit.
    let Apdu::Error(error) = response else {
        panic!("expected an Error, got {response:?}");
    };
    assert_eq!(error.invoke_id, 9);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
    );
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(count(&db, sink).await, (0, 0));
    // The retransmission is stored once storage is back.
    storage.fail.store(false, Ordering::SeqCst);
    let response = dispatch(
        &db,
        &config(sink),
        &Arc::new(ConfirmedRequestTracker::default()),
        9,
        &[0x10],
        None,
        request_bytes(vec![notification(AuditOperation::WRITE)]),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    assert_eq!(count(&db, sink).await, (1, 1));
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_commit_runs() {
    let storage = Arc::new(HeldStorage::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&storage));
    let (started, go) = storage.hold();
    // The commit takes real time on the log's writer thread, as on a slow
    // disk, while a timer like a request's APDU timeout is pending.
    let releasing = std::thread::spawn(move || {
        started.recv_timeout(WAIT).expect("the commit started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    let response = dispatch(
        &db,
        &config(sink),
        &Arc::new(ConfirmedRequestTracker::default()),
        9,
        &[0x10],
        None,
        request_bytes(vec![notification(AuditOperation::WRITE)]),
    )
    .await
    .unwrap();
    releasing.join().unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    // Waiting for the commit kept the runtime busy, so the paused clock did
    // not jump to the timer.
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "virtual time jumped {:?} while the commit ran",
        start.elapsed()
    );
    assert!(!timer.is_finished());
    timer.abort();
}
