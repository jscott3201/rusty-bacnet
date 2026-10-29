//! An Audit Log's Buffer_Size on the wire (#1238). A write is refused while
//! Log_Enable is TRUE; once logging is off it resizes the log, its commit
//! running with the database guard dropped, and a commit that fails is
//! refused with DEVICE / OPERATIONAL_PROBLEM. Record_Count stays read-only,
//! so no write purges the log.

use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, apdu, assert_denied, oid, value, wpm};
use super::*;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::primitives::{Date, Time};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex as StdMutex, mpsc};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);
const SIMPLE_ACK_WRITE: [u8; 3] = [0x20, 5, 15];
const SIMPLE_ACK_WPM: [u8; 3] = [0x20, 5, 16];

/// The WriteProperty Error PDU for invoke ID 5: class, then code, each an
/// application-tagged Enumerated.
fn write_error(class: ErrorClass, code: ErrorCode) -> Vec<u8> {
    vec![
        0x50,
        5,
        15,
        0x91,
        class.to_raw() as u8,
        0x91,
        code.to_raw() as u8,
    ]
}

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

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 10,
                day: 3,
                day_of_week: 6,
            },
            local_time: Time {
                hour: 9,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn audit_log() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_LOG, 7)
}

/// `fixture` holding Audit Log 7, sized 10, kept in `storage`, with three
/// status records.
async fn install(fixture: &Fixture, storage: &Arc<HeldStorage>) {
    let mut db = fixture.db.write().await;
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db.add(Box::new(
        AuditLogObject::new(7, "audit", 10, Arc::clone(storage) as _).unwrap(),
    ))
    .unwrap();
    // Off, on and off again: three log-status records.
    for on in [false, true, false] {
        db.get_mut(&audit_log())
            .unwrap()
            .write_property(
                PropertyIdentifier::LOG_ENABLE,
                None,
                PropertyValue::Boolean(on),
                None,
            )
            .unwrap();
    }
}

async fn fixture(storage: &Arc<HeldStorage>) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    install(&fixture, storage).await;
    Arc::new(fixture)
}

fn write_request(property: PropertyIdentifier, written: PropertyValue) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: audit_log(),
        property_identifier: property,
        property_array_index: None,
        property_value: value(written),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

async fn write(fixture: &Fixture, property: PropertyIdentifier, written: PropertyValue) -> Vec<u8> {
    wire(
        fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        write_request(property, written),
    )
    .await
}

fn attempt(property: PropertyIdentifier, written: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: value(written),
        priority: None,
    }
}

/// The sequence numbers of the records the log holds, oldest first.
async fn held(fixture: &Fixture) -> Vec<u64> {
    fixture
        .db
        .read()
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

#[tokio::test]
async fn buffer_size_is_written_only_while_logging_is_off() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(true)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    let before = storage.committed();
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(2)
        )
        .await,
        write_error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(held(&fixture).await, [1, 2, 3, 4]);
    assert_eq!(storage.committed(), before);

    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    let commits = storage.commits.load(Ordering::SeqCst);
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(2)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    // The newest records that fit stay, and the write took its staged
    // commit without committing again.
    assert_eq!(held(&fixture).await, [4, 5]);
    assert_eq!(
        fixture
            .read(audit_log(), PropertyIdentifier::BUFFER_SIZE)
            .await,
        PropertyValue::Unsigned(2)
    );
    assert_eq!(storage.committed().capacity, 2);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 1);
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(u64::from(u32::MAX))
        )
        .await,
        write_error(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
}

#[tokio::test]
async fn record_count_stays_read_only_so_no_write_purges_the_log() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let before = storage.committed();
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::RECORD_COUNT,
            PropertyValue::Unsigned(0)
        )
        .await,
        write_error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(held(&fixture).await, [1, 2, 3]);
    assert_eq!(storage.committed(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_buffer_size_commit_runs_while_the_database_stays_available() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        async move {
            write(
                &fixture,
                PropertyIdentifier::BUFFER_SIZE,
                PropertyValue::Unsigned(1),
            )
            .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the commit started");
    // While the commit runs the database answers readers and writers, and
    // the log still serves its old size.
    let size = tokio::time::timeout(
        WAIT,
        fixture.read(audit_log(), PropertyIdentifier::BUFFER_SIZE),
    )
    .await;
    let writable = tokio::time::timeout(WAIT, fixture.db.write()).await.is_ok();
    let answered_early = sending.is_finished();
    go.send(()).unwrap();
    assert_eq!(sending.await.unwrap(), SIMPLE_ACK_WRITE);
    assert_eq!(size.ok(), Some(PropertyValue::Unsigned(10)));
    assert!(writable, "the database was held while the log committed");
    assert!(!answered_early);
    assert_eq!(held(&fixture).await, [3]);
    assert_eq!(storage.committed().capacity, 1);
}

#[tokio::test]
async fn a_buffer_size_write_that_cannot_be_committed_is_refused_and_changes_nothing() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let before = storage.committed();
    storage.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(1)
        )
        .await,
        write_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(held(&fixture).await, [1, 2, 3]);
    assert_eq!(
        fixture
            .read(audit_log(), PropertyIdentifier::BUFFER_SIZE)
            .await,
        PropertyValue::Unsigned(10)
    );
    assert_eq!(storage.committed(), before);
}

fn log_attempts(first: BACnetPropertyValue, second: BACnetPropertyValue) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: audit_log(),
        list_of_properties: vec![first, second],
    }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_turns_logging_off_then_resizes_in_one_commit_off_the_guard() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(true)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    // Resizing first fails at that attempt, and the request stages nothing.
    let before = storage.committed();
    let commits = storage.commits.load(Ordering::SeqCst);
    let request = log_attempts(
        attempt(PropertyIdentifier::BUFFER_SIZE, PropertyValue::Unsigned(2)),
        attempt(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        ),
    );
    let response = fixture
        .dispatch(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, request, 5)
        .await
        .unwrap();
    let Apdu::Error(error) = apdu(response) else {
        panic!("expected a WritePropertyMultiple error");
    };
    assert_eq!(error.error_class, ErrorClass::PROPERTY);
    assert_eq!(error.error_code, ErrorCode::WRITE_ACCESS_DENIED);
    let detailed = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    assert_eq!(
        detailed.first_failed_write_attempt.property_identifier,
        PropertyIdentifier::BUFFER_SIZE.to_raw()
    );
    assert_eq!(
        fixture
            .read(audit_log(), PropertyIdentifier::LOG_ENABLE)
            .await,
        PropertyValue::Boolean(true)
    );
    assert_eq!(held(&fixture).await, [1, 2, 3, 4]);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
    assert_eq!(storage.committed(), before);

    // Turning logging off first lets the resize through in the same request,
    // both in one commit made with the database guard dropped.
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        let request = log_attempts(
            attempt(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            ),
            attempt(PropertyIdentifier::BUFFER_SIZE, PropertyValue::Unsigned(2)),
        );
        async move {
            wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                request,
            )
            .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the commit started");
    let enabled = tokio::time::timeout(
        WAIT,
        fixture.read(audit_log(), PropertyIdentifier::LOG_ENABLE),
    )
    .await;
    let writable = tokio::time::timeout(WAIT, fixture.db.write()).await.is_ok();
    let answered_early = sending.is_finished();
    go.send(()).unwrap();
    assert_eq!(sending.await.unwrap(), SIMPLE_ACK_WPM);
    assert_eq!(enabled.ok(), Some(PropertyValue::Boolean(true)));
    assert!(writable, "the database was held while the log committed");
    assert!(!answered_early);
    assert_eq!(held(&fixture).await, [4, 5]);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 1);
    assert_eq!(storage.committed().capacity, 2);
    assert!(!storage.committed().log_enable);
}

#[tokio::test]
async fn a_log_enable_write_that_cannot_be_committed_is_refused_and_changes_nothing() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let before = storage.committed();
    storage.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(true)
        )
        .await,
        write_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(
        fixture
            .read(audit_log(), PropertyIdentifier::LOG_ENABLE)
            .await,
        PropertyValue::Boolean(false)
    );
    assert_eq!(held(&fixture).await, [1, 2, 3]);
    assert_eq!(storage.committed(), before);
}

#[tokio::test]
async fn a_denied_buffer_size_write_stages_nothing() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = Fixture::new(Some(Arc::new(|_| false)));
    install(&fixture, &storage).await;
    let commits = storage.commits.load(Ordering::SeqCst);
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_request(PropertyIdentifier::BUFFER_SIZE, PropertyValue::Unsigned(1)),
            5,
        )
        .await
        .unwrap();
    assert_denied(response, ConfirmedServiceChoice::WRITE_PROPERTY, 5);
    assert_eq!(held(&fixture).await, [1, 2, 3]);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_buffer_size_commit_runs() {
    let storage = Arc::new(HeldStorage::default());
    let fixture = fixture(&storage).await;
    let (started, go) = storage.hold();
    let releasing = std::thread::spawn(move || {
        started.recv_timeout(WAIT).expect("the commit started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(1)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    releasing.join().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "virtual time jumped {:?} while the commit ran",
        start.elapsed()
    );
    assert!(!timer.is_finished());
    timer.abort();
}
