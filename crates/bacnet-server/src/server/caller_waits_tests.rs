//! A local call that its caller polls once and then leaves unpolled holds up
//! nothing the server serves (#2772). Tokio's lock hands a queued waiter the
//! guard whether or not anything polls it again, so a call that waited for
//! the database in its caller's task could own it, with every request the
//! server takes queued behind it.
//!
//! Each entry point is parked the same way: the test holds the lock the
//! call needs, polls the call once, has the peer send a WriteProperty that
//! queues behind it, lets the lock go, and has the peer read the Device's
//! Active_COV_Subscriptions, which needs the database and the COV table.
//! Both requests are answered while the call sits unpolled; polled at last,
//! the call returns what a call awaited normally returns.

use std::pin::Pin;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll, Waker};

use bacnet_encoding::apdu::{Apdu, ConfirmedRequest as ConfirmedRequestPdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, LifeSafetyOperation};
use bytes::{Bytes, BytesMut};
use tokio::sync::{OwnedRwLockWriteGuard, mpsc};

use super::*;
use crate::server::clock::{ClockConfig, clocked_test_database};
use crate::server::test_transport::{SendLog, TestTransport};

/// Long enough that only a request stuck behind a parked call reaches it;
/// the paused clock jumps there at once when nothing else can run.
const STUCK: Duration = Duration::from_secs(30);

const PEER: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];

/// A call as its caller holds it.
type Call<'a, R> = Pin<Box<dyn Future<Output = R> + Send + 'a>>;

/// Poll `call` once with a waker that wakes nothing: its caller then stops
/// polling it.
fn poll_once<R>(call: &mut Call<'_, R>) -> Poll<R> {
    call.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn device() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, 2772)
}

/// An Audit Log's storage, in memory.
#[derive(Default)]
struct Memory(StdMutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

struct Node {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    invoke_id: std::sync::atomic::AtomicU8,
}

/// A running server with AV-1, AI-1, BO-1, Life Safety Point 1 and Audit
/// Log 7 beside its Device.
async fn node() -> Node {
    let (transport, tx) = TestTransport::inbound(16);
    let sent = transport.sent();
    let identity = bacnet_objects::DeviceIdentity::new(2772, 42).unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("Parked callers"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "Setpoint", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryOutputObject::new(1, "Fan").unwrap()))
        .unwrap();
    db.add(Box::new(LifeSafetyPointObject::new(1, "Smoke").unwrap()))
        .unwrap();
    db.add(Box::new(
        AuditLogObject::new(7, "Audit", 16, Arc::new(Memory::default()) as _).unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    Node {
        server,
        tx,
        sent,
        invoke_id: std::sync::atomic::AtomicU8::new(0),
    }
}

fn confirmed(invoke_id: u8, service_choice: ConfirmedServiceChoice, body: BytesMut) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    apdu.freeze()
}

/// The peer writes AV-1's Present_Value.
fn write_av(value: f32) -> (ConfirmedServiceChoice, BytesMut) {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &PropertyValue::Real(value))
        .unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_VALUE, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: Some(10),
    }
    .encode(&mut body)
    .unwrap();
    (ConfirmedServiceChoice::WRITE_PROPERTY, body)
}

/// The peer reads the Device's Active_COV_Subscriptions: the database, then
/// the COV table.
fn read_subscriptions() -> (ConfirmedServiceChoice, BytesMut) {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        property_array_index: None,
    }
    .encode(&mut body);
    (ConfirmedServiceChoice::READ_PROPERTY, body)
}

/// The peer subscribes to AI-1's changes, unconfirmed.
fn subscribe() -> (ConfirmedServiceChoice, BytesMut) {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 9,
        monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(600),
    }
    .encode(&mut body)
    .unwrap();
    (ConfirmedServiceChoice::SUBSCRIBE_COV, body)
}

impl Node {
    /// Send the peer's request; its invoke ID.
    async fn ask(&self, (service, body): (ConfirmedServiceChoice, BytesMut)) -> u8 {
        let invoke_id = 1 + self
            .invoke_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: true,
                payload: confirmed(invoke_id, service, body),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&PEER),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        invoke_id
    }

    /// The server's answer to `invoke_id`; fails the test if none comes
    /// before [`STUCK`].
    async fn answer(&self, invoke_id: u8) -> Apdu {
        tokio::time::timeout(STUCK, async {
            loop {
                let answer = self.sent.frames().into_iter().find_map(|frame| {
                    let apdu = frame.apdu();
                    let id = match &apdu {
                        Apdu::ComplexAck(ack) => ack.invoke_id,
                        Apdu::SimpleAck(ack) => ack.invoke_id,
                        Apdu::Error(error) => error.invoke_id,
                        _ => return None,
                    };
                    (id == invoke_id).then_some(apdu)
                });
                if let Some(answer) = answer {
                    return answer;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the server never answered request {invoke_id}"))
    }

    /// The peer's requests, one queued behind the parked call while `held`
    /// is held and one after, are both answered.
    async fn served_past<G>(&self, held: G) {
        let write = self.ask(write_av(21.5)).await;
        // Let the write's handler queue behind the parked call.
        tokio::time::sleep(Duration::from_millis(5)).await;
        drop(held);
        let read = self.ask(read_subscriptions()).await;
        assert!(matches!(self.answer(write).await, Apdu::SimpleAck(_)));
        assert!(matches!(self.answer(read).await, Apdu::ComplexAck(_)));
    }

    async fn value(&self, object: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
        self.server
            .read_local(&object, property, None)
            .await
            .unwrap()
    }

    /// The server's request task set has nothing left once its finished
    /// tasks are joined.
    async fn drained(&self) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.server.request_tasks.reap_finished();
        assert_eq!(self.server.request_tasks.len(), 0, "every task ended");
    }

    /// Return once the server's request task set is empty, its finished
    /// tasks joined: an earlier local call may have waited in a task of its
    /// own, which can still be in the set, or about to end, as its caller
    /// goes on. A test that counts tasks starts from here.
    async fn settled(&self) {
        let tasks = &self.server.request_tasks;
        tokio::time::timeout(STUCK, async {
            while {
                tasks.reap_finished();
                !tasks.is_empty()
            } {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a request task never ended");
    }
}

/// The database's write guard, as the application holds it.
async fn hold(node: &Node) -> OwnedRwLockWriteGuard<ObjectDatabase> {
    Arc::clone(&node.server.db).write_owned().await
}

#[tokio::test(start_paused = true)]
async fn a_parked_write_local_holds_up_no_request() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    // The peer wrote at 10 after the parked write at 8: 8 wins.
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(5.0)
    );
    parked.await.unwrap();
    node.drained().await;
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_read_local_holds_up_no_request() {
    let mut node = node().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.read_local(
        &ai,
        PropertyIdentifier::OBJECT_NAME,
        None,
    ));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    assert_eq!(
        parked.await.unwrap(),
        node.value(ai, PropertyIdentifier::OBJECT_NAME).await
    );
    node.server.stop().await.unwrap();
}

/// A parked read of a live Device list waits for the COV table in the
/// task too, with the database held there, not in the caller.
#[tokio::test(start_paused = true)]
async fn a_parked_read_of_a_live_list_holds_up_no_request() {
    let mut node = node().await;
    let table = Arc::clone(&node.server.cov_table).write_owned().await;
    let device = device();
    let mut parked: Call<'_, _> = Box::pin(node.server.read_local(
        &device,
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        None,
    ));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(table).await;
    assert_eq!(
        parked.await.unwrap(),
        node.value(device, PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS)
            .await
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_write_local_encoded_holds_up_no_request() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut encoded, &PropertyValue::Real(7.0))
        .unwrap();
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local_encoded(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        &encoded,
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(7.0)
    );
    node.server.stop().await.unwrap();
}

/// One of the typed `*_local` inputs, through the same entry point.
#[tokio::test(start_paused = true)]
async fn a_parked_set_present_value_local_holds_up_no_request() {
    let mut node = node().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(
        node.server
            .set_present_value_local(&ai, PropertyValue::Real(19.0)),
    );
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    assert_eq!(
        node.value(ai, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(19.0)
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_life_safety_operation_holds_up_no_request() {
    let mut node = node().await;
    let point = oid(ObjectType::LIFE_SAFETY_POINT, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(
        node.server
            .set_life_safety_operation_expected_local(&point, LifeSafetyOperation::SILENCE),
    );
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    assert_eq!(
        node.value(point, PropertyIdentifier::OPERATION_EXPECTED)
            .await,
        PropertyValue::Enumerated(LifeSafetyOperation::SILENCE.to_raw())
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_clock_zone_change_holds_up_no_request() {
    let mut node = node().await;
    let zone = ClockConfig::new(-300, true).unwrap();
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.set_clock_zone(zone));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    assert_eq!(node.server.clock_zone(), Some(zone));
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_audit_log_purge_holds_up_no_request() {
    let mut node = node().await;
    let log = oid(ObjectType::AUDIT_LOG, 7);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.purge_audit_log(&log));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    // The purge leaves its one BUFFER_PURGED record.
    assert_eq!(
        node.value(log, PropertyIdentifier::RECORD_COUNT).await,
        PropertyValue::Unsigned(1)
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_reset_report_holds_up_no_request() {
    let mut node = node().await;
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.report_device_reset_local());
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await.unwrap();
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_physical_output_read_holds_up_no_request() {
    let mut node = node().await;
    let fan = oid(ObjectType::BINARY_OUTPUT, 1);
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.physical_output_local(&fan));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    assert!(!parked.await.unwrap(), "the Fan starts OFF");
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_pics_holds_up_no_request() {
    let mut node = node().await;
    let config = crate::pics::PicsConfig::default();
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.generate_pics(&config));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    let pics = parked.await;
    assert_eq!(
        format!("{pics:?}"),
        format!("{:?}", node.server.generate_pics(&config).await)
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_subscription_removal_holds_up_no_request() {
    let mut node = node().await;
    let subscribed = node.ask(subscribe()).await;
    assert!(matches!(node.answer(subscribed).await, Apdu::SimpleAck(_)));
    let table = Arc::clone(&node.server.cov_table).write_owned().await;
    let mut parked: Call<'_, _> = Box::pin(node.server.remove_peer_subscriptions(&PEER, None));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(table).await;
    assert_eq!(parked.await, 1);
    assert_eq!(node.server.remove_peer_subscriptions(&PEER, None).await, 0);
    node.server.stop().await.unwrap();
}

/// A write whose caller goes while its task waits for the database makes
/// no change, and its task ends.
#[tokio::test(start_paused = true)]
async fn a_write_dropped_while_its_task_waits_changes_nothing() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let before = node.value(av, PropertyIdentifier::PRESENT_VALUE).await;
    node.settled().await;
    let held = hold(&node).await;
    let mut parked: Call<'_, _> = Box::pin(node.server.write_local(
        &av,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(5.0),
        Some(8),
        crate::LocalCommandSource::ServerDevice,
    ));
    assert!(poll_once(&mut parked).is_pending());
    assert_eq!(node.server.request_tasks.len(), 1, "one task waits");
    drop(parked);
    node.drained().await;
    drop(held);
    assert_eq!(
        node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
        before
    );
    node.server.stop().await.unwrap();
}

/// Writes awaited one after the other land in that order, each through a
/// task of the server's while the database is held; calls made at once
/// land in the order they first waited.
#[tokio::test(start_paused = true)]
async fn local_writes_keep_their_order() {
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    {
        let server = &node.server;
        let write = |value: f32| {
            server.write_local(
                &av,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(value),
                Some(8),
                crate::LocalCommandSource::ServerDevice,
            )
        };
        let held = Arc::clone(&server.db).write_owned().await;
        let in_turn = async {
            write(1.0).await.unwrap();
            let held = Arc::clone(&server.db).write_owned().await;
            let mut second: Call<'_, _> = Box::pin(write(2.0));
            assert!(poll_once(&mut second).is_pending());
            drop(held);
            second.await.unwrap();
        };
        let release = async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            drop(held);
        };
        tokio::join!(in_turn, release);
        assert_eq!(
            node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
            PropertyValue::Real(2.0)
        );

        let held = Arc::clone(&server.db).write_owned().await;
        let mut first: Call<'_, _> = Box::pin(write(3.0));
        let mut second: Call<'_, _> = Box::pin(write(4.0));
        assert!(poll_once(&mut first).is_pending());
        assert!(poll_once(&mut second).is_pending());
        drop(held);
        second.await.unwrap();
        first.await.unwrap();
        assert_eq!(
            node.value(av, PropertyIdentifier::PRESENT_VALUE).await,
            PropertyValue::Real(4.0)
        );
    }
    node.drained().await;
    node.server.stop().await.unwrap();
}

/// A burst of parked writes makes one task each, no more, and every one of
/// them ends once the database is free, with no caller polling: the last
/// to queue is the value that stays.
#[tokio::test(start_paused = true)]
async fn a_burst_of_parked_calls_is_bounded() {
    const CALLS: u16 = 32;
    let mut node = node().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    node.settled().await;
    let held = hold(&node).await;
    let server = &node.server;
    let mut parked: Vec<Call<'_, _>> = (0..CALLS)
        .map(|n| -> Call<'_, _> {
            Box::pin(server.write_local(
                &av,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(f32::from(n)),
                Some(8),
                crate::LocalCommandSource::ServerDevice,
            ))
        })
        .collect();
    for call in &mut parked {
        assert!(poll_once(call).is_pending());
    }
    assert_eq!(server.request_tasks.len(), usize::from(CALLS));
    drop(held);
    tokio::time::sleep(Duration::from_millis(200)).await;
    server.request_tasks.reap_finished();
    assert_eq!(server.request_tasks.len(), 0, "every task ended unpolled");
    assert_eq!(
        server
            .read_local(&av, PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap(),
        PropertyValue::Real(f32::from(CALLS - 1))
    );
    for call in parked {
        call.await.unwrap();
    }
    node.server.stop().await.unwrap();
}

#[path = "caller_waits_edge_tests.rs"]
mod edge;

#[path = "tick_waits_tests.rs"]
mod ticks;
