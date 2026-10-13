//! A server stopped mid-request leaves no staged write behind (#1363).
//!
//! `stop()` aborts every request, so a Recipient_List write staged for one
//! is never made or released, and its save may already have put a list in
//! storage that the class never served. Once it has joined its requests,
//! `stop()` drops such a write and waits until storage holds the list the
//! class serves again, so a restart serves that list.

use super::durable_write_wire_tests::{destination, HeldStorage, WAIT};
use super::mutation_tests::wpm;
use super::*;
use crate::server::durable_writes::{slow_save_warnings, SLOW_SAVE_REPEAT, SLOW_SAVE_WARNING};
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::{
    NotificationClass, NotificationClassPersistence, NotificationClassSnapshot,
};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::BACnetDestination;
use std::time::Duration;
use tokio::sync::mpsc;

type ClassStorage = HeldStorage<NotificationClassSnapshot>;

const RECIPIENT_LIST: PropertyIdentifier = PropertyIdentifier::RECIPIENT_LIST;

fn snapshot(list: &[BACnetDestination]) -> NotificationClassSnapshot {
    NotificationClassSnapshot {
        recipient_list: Some(list.to_vec()),
    }
}

/// Storage that already holds `list` as a written Recipient_List.
fn holding(list: &[BACnetDestination]) -> Arc<ClassStorage> {
    let storage = Arc::new(ClassStorage::default());
    *storage.saved.lock().unwrap() = Some(snapshot(list));
    storage
}

fn nc(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, instance).unwrap()
}

/// Notification Class `instance`, kept in `storage`.
fn class(instance: u32, storage: &Arc<ClassStorage>) -> NotificationClass {
    NotificationClass::with_persistence(
        instance,
        format!("NC-{instance}"),
        Arc::clone(storage) as Arc<dyn NotificationClassPersistence>,
    )
    .unwrap()
}

/// A started server holding a Device and one class per storage, numbered
/// from 1, and the channel that feeds it requests.
async fn server(
    storages: &[&Arc<ClassStorage>],
) -> (BACnetServer<TestTransport>, mpsc::Sender<ReceivedNpdu>) {
    let (transport, inbound) = TestTransport::inbound(4);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 100,
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for (instance, storage) in (1..).zip(storages) {
        db.add(Box::new(class(instance, storage))).unwrap();
    }
    let server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    (server, inbound)
}

/// Feed the server a confirmed `service` request carrying `request`.
pub(super) async fn send(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    service: ConfirmedServiceChoice,
    request: Bytes,
) {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: request,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    inbound
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(&[1]),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

fn encoded(list: &[BACnetDestination]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, list).unwrap();
    buf.to_vec()
}

fn write_property(instance: u32, list: &[BACnetDestination]) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: nc(instance),
        property_identifier: RECIPIENT_LIST,
        property_array_index: None,
        property_value: encoded(list),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

fn list_write(instance: u32, list: &[BACnetDestination]) -> WriteAccessSpecification {
    WriteAccessSpecification {
        object_identifier: nc(instance),
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: RECIPIENT_LIST,
            property_array_index: None,
            value: encoded(list),
            priority: None,
        }],
    }
}

/// Wait off the runtime, as the request's own wait does, until a save
/// reports on `started`.
async fn save_started(started: std::sync::mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
}

/// Whether storage comes to hold `expected` within [`WAIT`].
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

/// Stop `server` while storage holds a request's staged save, and let the
/// save through, by dropping `go`, only once stop() has joined every request.
///
/// stop() aborts its requests on its first poll, but an abort lands only
/// when the request's task next yields. A request still running on another
/// worker can find its save done and make its write before then (#1457):
/// stop() has sealed responses, so nobody hears of it, but the object serves
/// that write and storage holds it. A request whose save storage still holds
/// can't get past its wait for it, so once stop() has joined the request, its
/// staged write is never made, and stop() puts storage back.
pub(super) async fn stop_then_release(
    mut server: BACnetServer<TestTransport>,
    go: std::sync::mpsc::Sender<()>,
) -> BACnetServer<TestTransport> {
    let requests = Arc::clone(&server.request_tasks);
    let stopping = tokio::spawn(async move {
        server.stop().await.unwrap();
        server
    });
    tokio::time::timeout(WAIT, async {
        while !requests.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stop() joined the request");
    // It goes on to wait for the save storage holds.
    assert!(
        !stopping.is_finished(),
        "stop() returned before the save ran"
    );
    drop(go);
    stopping.await.unwrap()
}

/// The Recipient_List class `instance` serves in `db`.
async fn served(db: &Arc<RwLock<ObjectDatabase>>, instance: u32) -> PropertyValue {
    db.read()
        .await
        .get(&nc(instance))
        .unwrap()
        .read_property(RECIPIENT_LIST, None)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_stopped_mid_request_leaves_storage_with_the_served_lists() {
    let landed = holding(&[destination(1)]);
    let held = holding(&[destination(2)]);
    let (server, inbound) = server(&[&landed, &held]).await;
    let (started, go) = held.hold();
    // One WritePropertyMultiple writes both classes and stages them in
    // object order: class 1's save lands, then the request waits for class
    // 2's, which the test holds. Neither write is made yet.
    let request = wpm(vec![
        list_write(1, &[destination(11)]),
        list_write(2, &[destination(12)]),
    ]);
    send(
        &inbound,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    save_started(started).await;
    assert_eq!(landed.load_saved(), Some(snapshot(&[destination(11)])));
    // stop() aborts the request, so neither staged write is ever made or
    // released; class 2's save lands only after the request has gone.
    let server = stop_then_release(server, go).await;
    // Storage holds the lists the classes serve once stop returns, though
    // the database outlives the stop.
    assert_eq!(landed.load_saved(), Some(snapshot(&[destination(1)])));
    assert_eq!(held.load_saved(), Some(snapshot(&[destination(2)])));
    let db = Arc::clone(server.database());
    let framed = |list: &[BACnetDestination]| PropertyValue::ApplicationData(encoded(list));
    assert_eq!(served(&db, 1).await, framed(&[destination(1)]));
    assert_eq!(served(&db, 2).await, framed(&[destination(2)]));
    // A restart serves them too.
    drop(db);
    drop(server);
    assert_eq!(class(1, &landed).recipient_list(), [destination(1)]);
    assert_eq!(class(2, &held).recipient_list(), [destination(2)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_while_the_application_holds_the_database_puts_storage_back_once_it_lets_go() {
    let storage = holding(&[destination(1)]);
    let (mut server, inbound) = server(&[&storage]).await;
    let (started, go) = storage.hold();
    send(
        &inbound,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        write_property(1, &[destination(11)]),
    )
    .await;
    save_started(started).await;
    // The application reads the database while the save runs. The save
    // lands, and the request waits for the guard to make its write.
    let db = Arc::clone(server.database());
    let reading = db.read().await;
    drop(go);
    assert!(comes_to_hold(&storage, snapshot(&[destination(11)])).await);
    // stop() aborts the request without waiting for the application, so it
    // returns with the staged list still in storage.
    server.stop().await.unwrap();
    assert_eq!(storage.load_saved(), Some(snapshot(&[destination(11)])));
    // Once the application lets go, storage goes back to the served list,
    // while the server and its database live on.
    drop(reading);
    assert!(
        comes_to_hold(&storage, snapshot(&[destination(1)])).await,
        "storage kept a list the class never served"
    );
    assert_eq!(
        served(&db, 1).await,
        PropertyValue::ApplicationData(encoded(&[destination(1)]))
    );
}

/// Move a paused clock a second at a time, at most a minute, until stop()
/// has warned `count` times on `db`; how long that took.
async fn until_warned(db: &Arc<RwLock<ObjectDatabase>>, count: usize) -> Duration {
    let mut waited = Duration::ZERO;
    while slow_save_warnings(db) < count {
        assert!(waited < Duration::from_secs(60), "stop() never warned");
        tokio::time::advance(Duration::from_secs(1)).await;
        waited += Duration::from_secs(1);
    }
    waited
}

#[tokio::test(start_paused = true)]
async fn stop_warns_while_storage_holds_a_save_and_waits_for_it() {
    let storage = holding(&[destination(1)]);
    let (server, inbound) = server(&[&storage]).await;
    let (started, go) = storage.hold();
    send(
        &inbound,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        write_property(1, &[destination(11)]),
    )
    .await;
    save_started(started).await;
    let db = Arc::clone(server.database());
    let stopping = tokio::spawn(async move {
        let mut server = server;
        server.stop().await.unwrap();
        server
    });
    // Storage holds the staged save, so stop() waits for it. The wait runs
    // on the blocking pool, so only the test moves the paused clock.
    assert!(until_warned(&db, 1).await >= SLOW_SAVE_WARNING);
    // It warns again a while later, and still waits.
    let again = until_warned(&db, 2).await;
    assert!(again >= SLOW_SAVE_REPEAT - Duration::from_secs(1));
    assert!(!stopping.is_finished());
    // Once storage lets the saves through, stop() returns with the served
    // list put back.
    drop(go);
    let server = stopping.await.unwrap();
    assert_eq!(storage.load_saved(), Some(snapshot(&[destination(1)])));
    drop(server);
}

#[path = "durable_drop_tests.rs"]
mod drop_tests;
