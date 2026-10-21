//! A Notification Class's Recipient_List saves through the server (#1315):
//! WriteProperty, WritePropertyMultiple, AddListElement, RemoveListElement
//! and `write_local` each stage the save, which runs on the class's writer
//! thread while the database guard is dropped. A list that cannot be saved
//! is refused, and a saved list is what a rebuilt server serves.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    destination, while_saving, HeldStorage, ERROR_PDU, SIMPLE_ACK_ADD, SIMPLE_ACK_DELETE,
    SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT,
};
use super::mutation_list_wire_tests::{change_list_error, list_request, wire, ADD, REMOVE};
use super::mutation_tests::{wpm, Fixture};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::{
    NotificationClass, NotificationClassPersistence, NotificationClassSnapshot,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetDestination;
use std::sync::atomic::Ordering;
use std::time::Duration;

type ClassStorage = HeldStorage<NotificationClassSnapshot>;

impl NotificationClassPersistence for ClassStorage {
    fn load(&self, _class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        Ok(self.load_saved())
    }

    fn save(
        &self,
        _class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        self.store(snapshot)
    }
}

const RECIPIENT_LIST: PropertyIdentifier = PropertyIdentifier::RECIPIENT_LIST;

/// Notification Class 1 kept in `storage`, with `configured` added the way
/// an application configures it.
fn class(storage: &Arc<ClassStorage>, configured: &[BACnetDestination]) -> NotificationClass {
    let mut class = NotificationClass::with_persistence(
        1,
        "NC",
        Arc::clone(storage) as Arc<dyn NotificationClassPersistence>,
    )
    .unwrap();
    for destination in configured {
        class.add_destination(destination.clone()).unwrap();
    }
    class
}

/// A request fixture whose database holds [`class`].
async fn served_by(
    storage: &Arc<ClassStorage>,
    configured: &[BACnetDestination],
) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let class = class(storage, configured);
    let nc = class.object_identifier();
    fixture.db.write().await.add(Box::new(class)).unwrap();
    (Arc::new(fixture), nc)
}

fn encoded(list: &[BACnetDestination]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, list).unwrap();
    buf.to_vec()
}

fn framed(list: &[BACnetDestination]) -> PropertyValue {
    PropertyValue::ApplicationData(encoded(list))
}

/// The Recipient_List a server rebuilt on `storage` serves.
async fn after_a_rebuild(storage: &Arc<ClassStorage>) -> PropertyValue {
    let (fixture, nc) = served_by(storage, &[]).await;
    fixture.read(nc, RECIPIENT_LIST).await
}

fn write_property(nc: ObjectIdentifier, list: &[BACnetDestination]) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: nc,
        property_identifier: RECIPIENT_LIST,
        property_array_index: None,
        property_value: encoded(list),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

fn write_property_multiple(nc: ObjectIdentifier, attempts: Vec<BACnetPropertyValue>) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: nc,
        list_of_properties: attempts,
    }])
}

fn list_attempt(list: &[BACnetDestination]) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: RECIPIENT_LIST,
        property_array_index: None,
        value: encoded(list),
        priority: None,
    }
}

/// Send `request` while holding its save, check the database stayed
/// available and the class answered with `ack`, then rebuild on the same
/// storage and check the rebuilt class serves `expected`.
async fn saves_off_the_lock_and_survives_a_rebuild(
    configured: &[BACnetDestination],
    service: ConfirmedServiceChoice,
    request: impl FnOnce(ObjectIdentifier) -> Bytes,
    ack: &[u8],
    expected: &[BACnetDestination],
) {
    let storage = Arc::new(ClassStorage::default());
    let (fixture, nc) = served_by(&storage, configured).await;
    let (response, available) = while_saving(&fixture, &storage, service, request(nc), WAIT).await;
    assert_eq!(response, ack);
    assert!(available, "the database was held while the class saved");
    assert_eq!(fixture.read(nc, RECIPIENT_LIST).await, framed(expected));
    // The write took the staged save; it did not save again.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    drop(fixture);
    assert_eq!(after_a_rebuild(&storage).await, framed(expected));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_saves_off_the_lock_and_survives_a_rebuild() {
    let list = [destination(4), destination(5)];
    saves_off_the_lock_and_survives_a_rebuild(
        &[],
        ConfirmedServiceChoice::WRITE_PROPERTY,
        |nc| write_property(nc, &list),
        &SIMPLE_ACK_WRITE,
        &list,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_saves_off_the_lock_and_survives_a_rebuild() {
    let list = [destination(4)];
    let mut description = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut description, "alarms").unwrap();
    saves_off_the_lock_and_survives_a_rebuild(
        &[],
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        |nc| {
            write_property_multiple(
                nc,
                vec![
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::DESCRIPTION,
                        property_array_index: None,
                        value: description.to_vec(),
                        priority: None,
                    },
                    list_attempt(&list),
                ],
            )
        },
        &SIMPLE_ACK_WPM,
        &list,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_add_list_element_saves_off_the_lock_and_survives_a_rebuild() {
    // The configured destination is in the list the request edits, so the
    // saved list holds it too.
    saves_off_the_lock_and_survives_a_rebuild(
        &[destination(1)],
        ADD,
        |nc| list_request(nc, RECIPIENT_LIST, None, &encoded(&[destination(6)])),
        &SIMPLE_ACK_ADD,
        &[destination(1), destination(6)],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remove_list_element_saves_off_the_lock_and_survives_a_rebuild() {
    saves_off_the_lock_and_survives_a_rebuild(
        &[destination(1), destination(2)],
        REMOVE,
        |nc| list_request(nc, RECIPIENT_LIST, None, &encoded(&[destination(1)])),
        &[0x20, 5, REMOVE.to_raw()],
        &[destination(2)],
    )
    .await;
}

/// A started server holding [`class`] and a Device, for `write_local`.
async fn local_server(
    storage: &Arc<ClassStorage>,
) -> (Arc<BACnetServer<TestTransport>>, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(100, 0)
                .expect("valid Device identity")
                .with_name("Notification Class device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let class = class(storage, &[]);
    let nc = class.object_identifier();
    db.add(Box::new(class)).unwrap();
    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
        .transport(TestTransport::new())
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    (Arc::new(server), nc)
}

async fn write_local(
    server: &BACnetServer<TestTransport>,
    nc: ObjectIdentifier,
    list: &[BACnetDestination],
) -> Result<(), Error> {
    server
        .write_local(
            &nc,
            RECIPIENT_LIST,
            None,
            framed(list),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

async fn stop(server: Arc<BACnetServer<TestTransport>>) {
    let mut server = Arc::into_inner(server).expect("no other handle on the server");
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_local_saves_off_the_lock_and_survives_a_rebuild() {
    let storage = Arc::new(ClassStorage::default());
    let (server, nc) = local_server(&storage).await;
    let list = [destination(7)];
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let server = Arc::clone(&server);
        let list = list.clone();
        async move { write_local(&server, nc, &list).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    let db = server.database();
    let readable = tokio::time::timeout(WAIT, db.read()).await.is_ok();
    let writable = tokio::time::timeout(WAIT, db.write()).await.is_ok();
    go.send(()).unwrap();
    writing.await.unwrap().unwrap();
    assert!(
        readable && writable,
        "the database was held while the class saved"
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    stop(server).await;
    assert_eq!(after_a_rebuild(&storage).await, framed(&list));
}

/// The Error PDU a WriteProperty refused with DEVICE / OPERATIONAL_PROBLEM
/// gets: invoke ID 5, the service, then the class and code.
const WRITE_REFUSED: [u8; 7] = [0x50, 5, 15, 0x91, 0, 0x91, 25];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_that_cannot_be_saved_is_refused_on_every_path_and_the_old_list_stays() {
    let storage = Arc::new(ClassStorage::default());
    let kept = [destination(1)];
    let (fixture, nc) = served_by(&storage, &[]).await;
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property(nc, &kept)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    storage.fail.store(true, Ordering::SeqCst);
    let refused = [destination(2)];
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property(nc, &refused)
        )
        .await,
        WRITE_REFUSED
    );
    let wpm_response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        write_property_multiple(nc, vec![list_attempt(&refused)]),
    )
    .await;
    // A WritePropertyMultiple-Error opens with the class and code.
    assert_eq!(wpm_response[..8], [0x50, 5, 16, 0x0E, 0x91, 0, 0x91, 25]);
    // Adding the second destination, or removing the first.
    for (service, elements) in [(ADD, &refused), (REMOVE, &kept)] {
        let request = list_request(nc, RECIPIENT_LIST, None, &encoded(elements));
        assert_eq!(
            wire(&fixture, service, request).await,
            change_list_error(service, 0, 25, 0)
        );
    }
    assert_eq!(fixture.read(nc, RECIPIENT_LIST).await, framed(&kept));
    assert_eq!(
        storage.load_saved().unwrap().recipient_list,
        Some(kept.to_vec())
    );
    drop(fixture);

    let (server, nc) = local_server(&storage).await;
    match write_local(&server, nc, &refused).await {
        Err(Error::Protocol { class, code }) => assert_eq!(
            (class, code),
            (ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
        ),
        other => panic!("expected DEVICE / OPERATIONAL_PROBLEM, got {other:?}"),
    }
    stop(server).await;
    storage.fail.store(false, Ordering::SeqCst);
    assert_eq!(after_a_rebuild(&storage).await, framed(&kept));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_destinations_are_never_saved_and_a_written_list_wins_on_restart() {
    let storage = Arc::new(ClassStorage::default());
    let configured = [destination(1)];
    let (fixture, nc) = served_by(&storage, &configured).await;
    // A write to another property saves nothing, so a restart applies the
    // configuration again.
    let mut description = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut description, "alarms").unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: nc,
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: description.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request.freeze()
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    drop(fixture);
    let (fixture, nc) = served_by(&storage, &[destination(2)]).await;
    assert_eq!(
        fixture.read(nc, RECIPIENT_LIST).await,
        framed(&[destination(2)])
    );

    // A list write over the network is saved, and wins over the
    // configuration at the next start.
    let written = [destination(3)];
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property(nc, &written)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    drop(fixture);
    let (fixture, nc) = served_by(&storage, &configured).await;
    assert_eq!(fixture.read(nc, RECIPIENT_LIST).await, framed(&written));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_stopped_before_its_list_write_saves_no_list() {
    let storage = Arc::new(ClassStorage::default());
    let (fixture, nc) = served_by(&storage, &[destination(1)]).await;
    // The first attempt fails, since Description takes a character string,
    // so the list write after it, staged and already saved, never reaches
    // the class.
    let mut unsigned = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut unsigned, 1);
    let request = write_property_multiple(
        nc,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: unsigned.to_vec(),
                priority: None,
            },
            list_attempt(&[destination(9)]),
        ],
    );
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response[0], ERROR_PDU, "the first attempt was refused");
    // Storage goes back at once to holding no written list, the state the
    // class serves: its list is still the configured one.
    let restored = tokio::time::timeout(WAIT, async {
        while storage.saves.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        restored.is_ok(),
        "storage kept a list the class never served"
    );
    assert_eq!(
        storage.load_saved().unwrap(),
        NotificationClassSnapshot::default()
    );
    assert_eq!(
        fixture.read(nc, RECIPIENT_LIST).await,
        framed(&[destination(1)])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_class_while_its_save_is_held_leaves_the_database_available() {
    let storage = Arc::new(ClassStorage::default());
    let (fixture, nc) = served_by(&storage, &[]).await;
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        let request = write_property(nc, &[destination(7)]);
        async move { wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // Dropping the removed class waits for that save, so DeleteObject drops
    // it off the guard: it is answered, and the database stays available,
    // while the save is still held.
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: nc,
    }
    .encode(&mut request);
    let deleting = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        async move {
            wire(
                &fixture,
                ConfirmedServiceChoice::DELETE_OBJECT,
                request.freeze(),
            )
            .await
        }
    });
    let deleted = tokio::time::timeout(WAIT, deleting).await;
    let readable = tokio::time::timeout(WAIT, fixture.db.read()).await.is_ok();
    go.send(()).unwrap();
    assert_eq!(
        deleted
            .expect("DeleteObject was answered while the save was held")
            .unwrap(),
        SIMPLE_ACK_DELETE
    );
    assert!(readable, "the database was held while the save ran");
    // The list write then finds its class gone.
    assert_eq!(writing.await.unwrap()[0], ERROR_PDU);
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_class_save_runs() {
    let storage = Arc::new(ClassStorage::default());
    let (fixture, nc) = served_by(&storage, &[]).await;
    let (started, go) = storage.hold();
    // The save takes real time on the class's writer thread, as on a slow
    // disk, while a timer like a request's APDU timeout is pending.
    let releasing = std::thread::spawn(move || {
        started.recv_timeout(WAIT).expect("the save started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_property(nc, &[destination(7)])
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    releasing.join().unwrap();
    // Waiting for the save kept the runtime busy, so the paused clock did
    // not jump to the timer.
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "virtual time jumped {:?} while the save ran",
        start.elapsed()
    );
    assert!(!timer.is_finished());
    timer.abort();
}

/// How long a staged write waits for its request once its save has run, in
/// a build of bacnet-objects outside its own tests.
const STAGED_WRITE_LIFETIME: Duration = Duration::from_secs(10);
/// The operation task's period.
const TICK: Duration = Duration::from_secs(1);

fn snapshot(list: &[BACnetDestination]) -> NotificationClassSnapshot {
    NotificationClassSnapshot {
        recipient_list: Some(list.to_vec()),
    }
}

/// Whether storage comes to hold `expected` within [`WAIT`] of real time.
/// The wait runs on the blocking pool, so a paused clock stands still.
async fn comes_to_hold(storage: &Arc<ClassStorage>, expected: NotificationClassSnapshot) -> bool {
    let storage = Arc::clone(storage);
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + WAIT;
        while std::time::Instant::now() < deadline {
            if storage.load_saved().as_ref() == Some(&expected) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    })
    .await
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_staged_write_whose_request_vanished_is_put_back_within_its_lifetime_and_a_tick() {
    let storage = Arc::new(ClassStorage::default());
    let (server, nc) = local_server(&storage).await;
    let served = [destination(1)];
    write_local(&server, nc, &served).await.unwrap();
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let server = Arc::clone(&server);
        async move { write_local(&server, nc, &[destination(2)]).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // The request goes while its save is held, as when stop() aborts it or
    // an application drops the future. Its staged write stays behind.
    writing.abort();
    assert!(writing.await.unwrap_err().is_cancelled());
    // The save lands, so storage holds a list the class never served. The
    // paused clock stands still until the save is done: the request's wait
    // on the blocking pool outlives the request. Dropping `go` lets this
    // save, and every later one, through.
    drop(go);
    let unserved = snapshot(&[destination(2)]);
    assert!(comes_to_hold(&storage, unserved.clone()).await);
    let start = tokio::time::Instant::now();
    // Within the lifetime, storage keeps it.
    tokio::time::sleep(STAGED_WRITE_LIFETIME - TICK).await;
    assert_eq!(storage.load_saved(), Some(unserved));
    // By a lifetime and a tick, the operation task has dropped the staged
    // write and queued a save of the served list.
    tokio::time::sleep_until(start + STAGED_WRITE_LIFETIME + TICK + Duration::from_millis(500))
        .await;
    assert!(
        comes_to_hold(&storage, snapshot(&served)).await,
        "storage kept a list the class never served"
    );
    let db = server.database().read().await;
    let class = db.get(&nc).unwrap();
    assert_eq!(
        class.read_property(RECIPIENT_LIST, None).unwrap(),
        framed(&served)
    );
    drop(db);
    stop(server).await;
}

/// Recipient_List isn't commandable and has no NULL in its datatype, so a
/// NULL written to it succeeds and leaves the list as it was (#1396). The
/// class stages and saves nothing for it, over WriteProperty,
/// WritePropertyMultiple or `write_local_encoded`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_null_recipient_list_succeeds_unchanged_and_saves_nothing() {
    let storage = Arc::new(ClassStorage::default());
    let kept = [destination(1)];
    let (fixture, nc) = served_by(&storage, &[]).await;
    let write = ConfirmedServiceChoice::WRITE_PROPERTY;
    assert_eq!(
        wire(&fixture, write, write_property(nc, &kept)).await,
        SIMPLE_ACK_WRITE
    );
    let saves = storage.saves.load(Ordering::SeqCst);
    let mut null = BytesMut::new();
    WritePropertyRequest {
        object_identifier: nc,
        property_identifier: RECIPIENT_LIST,
        property_array_index: None,
        property_value: vec![0x00],
        priority: None,
    }
    .encode(&mut null)
    .unwrap();
    assert_eq!(wire(&fixture, write, null.freeze()).await, SIMPLE_ACK_WRITE);
    let null_attempt = BACnetPropertyValue {
        property_identifier: RECIPIENT_LIST,
        property_array_index: None,
        value: vec![0x00],
        priority: None,
    };
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            write_property_multiple(nc, vec![null_attempt]),
        )
        .await,
        SIMPLE_ACK_WPM
    );
    assert_eq!(fixture.read(nc, RECIPIENT_LIST).await, framed(&kept));
    assert_eq!(storage.saves.load(Ordering::SeqCst), saves);
    drop(fixture);

    // A server rebuilt on the same storage serves the list saved above.
    let (server, nc) = local_server(&storage).await;
    server
        .write_local_encoded(
            &nc,
            RECIPIENT_LIST,
            None,
            &[0x00],
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let served = server
        .database()
        .read()
        .await
        .get(&nc)
        .unwrap()
        .read_property(RECIPIENT_LIST, None)
        .unwrap();
    assert_eq!(served, framed(&kept));
    assert_eq!(storage.saves.load(Ordering::SeqCst), saves);
    stop(server).await;
}
