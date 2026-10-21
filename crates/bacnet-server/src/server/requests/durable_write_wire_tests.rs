//! A Notification Forwarder's list saves through the server's request
//! dispatch (#1270): each runs on the forwarder's writer thread while the
//! database guard is dropped, so other requests read and write meanwhile,
//! and a list that cannot be saved is still refused on the wire.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::mutation_list_wire_tests::{change_list_error, list_request, wire, ADD};
use super::mutation_tests::{oid, Fixture};
use super::*;
use bacnet_encoding::constructed::{
    encode_destination_list, encode_event_notification_subscription_list,
};
use bacnet_objects::notification_forwarder::{
    ForwarderSnapshot, NotificationForwarderObject, NotificationForwarderPersistence,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetDestination, BACnetEventNotificationSubscription, BACnetRecipient,
};
use bacnet_types::primitives::Time;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex as StdMutex};
use std::time::Duration;

pub(super) const WAIT: Duration = Duration::from_secs(10);
pub(super) const SIMPLE_ACK_ADD: [u8; 3] = [0x20, 5, 8];
pub(super) const SIMPLE_ACK_WRITE: [u8; 3] = [0x20, 5, 15];
pub(super) const SIMPLE_ACK_WPM: [u8; 3] = [0x20, 5, 16];
pub(super) const SIMPLE_ACK_DELETE: [u8; 3] = [0x20, 5, 11];
/// The first octet of an Error PDU.
pub(super) const ERROR_PDU: u8 = 0x50;

/// Storage of snapshots `S` whose saves can fail, or wait until the test
/// lets each one go: a forwarder's here, a Notification Class's in
/// `notification_class_durable_tests`.
#[derive(Default)]
pub(super) struct HeldStorage<S = ForwarderSnapshot> {
    pub(super) saved: StdMutex<Option<S>>,
    pub(super) saves: AtomicUsize,
    pub(super) fail: AtomicBool,
    hold: StdMutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl<S: Clone> HeldStorage<S> {
    /// Make every later save report on the first channel and wait for a
    /// message on the second.
    pub(super) fn hold(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started, started_rx) = mpsc::channel();
        let (go, go_rx) = mpsc::channel();
        *self.hold.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    }

    /// What was saved last.
    pub(super) fn load_saved(&self) -> Option<S> {
        self.saved.lock().unwrap().clone()
    }

    /// Save `snapshot`, waiting first if the test holds saves.
    pub(super) fn store(&self, snapshot: &S) -> Result<(), Error> {
        if let Some((started, go)) = &*self.hold.lock().unwrap() {
            let _ = started.send(());
            let _ = go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl NotificationForwarderPersistence for HeldStorage {
    fn load(&self, _forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        Ok(self.load_saved())
    }

    fn save(
        &self,
        _forwarder: ObjectIdentifier,
        snapshot: &ForwarderSnapshot,
    ) -> Result<(), Error> {
        self.store(snapshot)
    }
}

fn subscription(instance: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, instance)),
        process_identifier: 3,
        issue_confirmed_notifications: false,
        time_remaining: 10,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    buf.to_vec()
}

pub(super) fn destination(instance: u32) -> BACnetDestination {
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, instance)),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

/// A fixture holding forwarder 1, kept in `storage`.
async fn fixture(storage: &Arc<HeldStorage>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let forwarder = NotificationForwarderObject::with_persistence(
        1,
        "NF",
        Arc::clone(storage) as Arc<dyn NotificationForwarderPersistence>,
    )
    .unwrap();
    let nf = forwarder.object_identifier();
    fixture.db.write().await.add(Box::new(forwarder)).unwrap();
    (Arc::new(fixture), nf)
}

/// Send `request`, hold its save, and check that the database answers a
/// reader and a writer while the save runs. Returns the response and whether
/// the database answered both, each `within` the given time.
pub(super) async fn while_saving<S: Clone>(
    fixture: &Arc<Fixture>,
    storage: &HeldStorage<S>,
    service: ConfirmedServiceChoice,
    request: Bytes,
    within: Duration,
) -> (Vec<u8>, bool) {
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let fixture = Arc::clone(fixture);
        async move { wire(&fixture, service, request).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    let readable = tokio::time::timeout(within, fixture.db.read())
        .await
        .is_ok();
    let writable = tokio::time::timeout(within, fixture.db.write())
        .await
        .is_ok();
    go.send(()).unwrap();
    (sending.await.unwrap(), readable && writable)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_add_list_element_save_runs_while_the_database_stays_available() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let request = list_request(
        nf,
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        None,
        &framed(&[subscription(7)]),
    );
    let (response, available) = while_saving(&fixture, &storage, ADD, request, WAIT).await;
    assert_eq!(response, SIMPLE_ACK_ADD);
    assert!(available, "the database was held while the forwarder saved");
    assert_eq!(
        fixture
            .read(nf, PropertyIdentifier::SUBSCRIBED_RECIPIENTS)
            .await,
        PropertyValue::ApplicationData(framed(&[subscription(7)]))
    );
    // The write took the staged save; it did not save again.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recipient_list_write_property_save_runs_while_the_database_stays_available() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let mut list = BytesMut::new();
    encode_destination_list(&mut list, &[destination(4)]).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: nf,
        property_identifier: PropertyIdentifier::RECIPIENT_LIST,
        property_array_index: None,
        property_value: list.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    let (response, available) = while_saving(
        &fixture,
        &storage,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        request.freeze(),
        WAIT,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
    assert!(available, "the database was held while the forwarder saved");
    assert_eq!(
        storage
            .saved
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .recipient_list,
        Some(vec![destination(4)])
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
}

fn subscriptions_attempt() -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        property_array_index: None,
        value: framed(&[subscription(8)]),
        priority: None,
    }
}

fn wpm(nf: ObjectIdentifier, attempts: Vec<BACnetPropertyValue>) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: nf,
            list_of_properties: attempts,
        }],
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

fn wpm_request(nf: ObjectIdentifier) -> Bytes {
    wpm(nf, vec![subscriptions_attempt()])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_stages_a_list_write_that_follows_another_attempt() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let mut description = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut description, "relay").unwrap();
    let request = wpm(
        nf,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: description.to_vec(),
                priority: None,
            },
            subscriptions_attempt(),
        ],
    );
    let (response, available) = while_saving(
        &fixture,
        &storage,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
        WAIT,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert!(available, "the database was held while the forwarder saved");
    assert_eq!(
        fixture.read(nf, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("relay".into())
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_save_runs_while_the_database_stays_available() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let (response, available) = while_saving(
        &fixture,
        &storage,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm_request(nf),
        WAIT,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert!(available, "the database was held while the forwarder saved");
    assert_eq!(
        storage
            .saved
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .subscribed_recipients,
        [subscription(8)]
    );
}

/// A fixture holding forwarder 1, kept in `storage`, under an authorizer
/// that denies Subscribed_Recipients writes and counts its calls.
async fn authorized(
    storage: &Arc<HeldStorage>,
) -> (Arc<Fixture>, ObjectIdentifier, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let fixture = Fixture::new(Some(Arc::new(move |context| {
        counted.fetch_add(1, Ordering::SeqCst);
        let crate::mutation::MutationTarget::WritePropertyMultiple(attempt) = &context.target
        else {
            return true;
        };
        attempt.reference.property_identifier != PropertyIdentifier::SUBSCRIBED_RECIPIENTS.to_raw()
    })));
    let forwarder = NotificationForwarderObject::with_persistence(
        1,
        "NF",
        Arc::clone(storage) as Arc<dyn NotificationForwarderPersistence>,
    )
    .unwrap();
    let nf = forwarder.object_identifier();
    fixture.db.write().await.add(Box::new(forwarder)).unwrap();
    (Arc::new(fixture), nf, calls)
}

fn recipient_list_attempt() -> BACnetPropertyValue {
    let mut list = BytesMut::new();
    encode_destination_list(&mut list, &[destination(4)]).unwrap();
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::RECIPIENT_LIST,
        property_array_index: None,
        value: list.to_vec(),
        priority: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_under_a_policy_stages_its_save() {
    // The server asks the authorizer about the attempt before staging it
    // (#1321), so the save runs with the guard dropped.
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf, calls) = authorized(&storage).await;
    let (response, available) = while_saving(
        &fixture,
        &storage,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm(nf, vec![recipient_list_attempt()]),
        WAIT,
    )
    .await;
    assert_eq!(response, SIMPLE_ACK_WPM);
    assert!(available, "the database was held while the forwarder saved");
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    // Asked once: the handler took the decision made before staging.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_denied_write_property_multiple_attempt_never_reaches_storage() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf, calls) = authorized(&storage).await;
    // The denied attempt alone stages nothing and saves nothing.
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let denied = wire(&fixture, service, wpm_request(nf)).await;
    // A WritePropertyMultiple-Error opens with SERVICES (5) and
    // SERVICE_REQUEST_DENIED (29).
    assert_eq!(denied[..8], [ERROR_PDU, 5, 16, 0x0E, 0x91, 5, 0x91, 29]);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // After an allowed list attempt, only that one is staged: the one save
    // holds the Recipient_List, and the denied list never reaches storage.
    let request = wpm(nf, vec![recipient_list_attempt(), subscriptions_attempt()]);
    let response = wire(&fixture, service, request).await;
    assert_eq!(response[..8], denied[..8]);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(
        storage.load_saved(),
        Some(ForwarderSnapshot {
            recipient_list: Some(vec![destination(4)]),
            subscribed_recipients: Vec::new(),
        })
    );
    assert_eq!(
        fixture
            .read(nf, PropertyIdentifier::SUBSCRIBED_RECIPIENTS)
            .await,
        PropertyValue::ApplicationData(framed(&[]))
    );
}

#[tokio::test]
async fn a_list_that_cannot_be_saved_is_refused_on_the_wire_and_the_old_list_stays() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let added = list_request(
        nf,
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        None,
        &framed(&[subscription(7)]),
    );
    assert_eq!(wire(&fixture, ADD, added).await, SIMPLE_ACK_ADD);
    storage.fail.store(true, Ordering::SeqCst);
    let refused = list_request(
        nf,
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        None,
        &framed(&[subscription(8)]),
    );
    assert_eq!(
        wire(&fixture, ADD, refused).await,
        change_list_error(ADD, 0, 25, 0)
    );
    assert_eq!(
        fixture
            .read(nf, PropertyIdentifier::SUBSCRIBED_RECIPIENTS)
            .await,
        PropertyValue::ApplicationData(framed(&[subscription(7)]))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_stopped_before_its_list_write_puts_the_served_list_back() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    // The first attempt fails, since Description takes a character string,
    // so the list write after it, staged and already saved, never reaches
    // the forwarder.
    let mut unsigned = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut unsigned, 1);
    let request = wpm(
        nf,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: unsigned.to_vec(),
                priority: None,
            },
            subscriptions_attempt(),
        ],
    );
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response[0], ERROR_PDU, "the first attempt was refused");
    // Storage goes back to the lists the forwarder serves at once: this
    // fixture runs no operation task to do it later.
    let restored = tokio::time::timeout(WAIT, async {
        while storage.saves.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        restored.is_ok(),
        "storage kept a list the forwarder never served"
    );
    assert_eq!(
        storage.saved.lock().unwrap().clone().unwrap(),
        ForwarderSnapshot::default()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_forwarder_while_its_save_is_held_leaves_the_database_available() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    // A list write stages, and its save is held on the forwarder's writer.
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        let request = list_request(
            nf,
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
            None,
            &framed(&[subscription(7)]),
        );
        async move { wire(&fixture, ADD, request).await }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // Dropping the removed forwarder waits for that save, so DeleteObject
    // drops it off the guard: it is answered, and the database stays
    // available, while the save is still held.
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: nf,
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
    // The list write then finds its forwarder gone.
    assert_eq!(writing.await.unwrap()[0], ERROR_PDU);
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_staged_save_runs() {
    let storage = Arc::new(HeldStorage::default());
    let (fixture, nf) = fixture(&storage).await;
    let (started, go) = storage.hold();
    // The save takes real time on the forwarder's writer thread, as on a
    // slow disk, while a timer like a request's APDU timeout is pending.
    let releasing = std::thread::spawn(move || {
        started.recv_timeout(WAIT).expect("the save started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    let request = list_request(
        nf,
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        None,
        &framed(&[subscription(7)]),
    );
    assert_eq!(wire(&fixture, ADD, request).await, SIMPLE_ACK_ADD);
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
