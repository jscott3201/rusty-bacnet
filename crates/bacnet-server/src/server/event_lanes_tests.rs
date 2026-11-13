//! The event notification lanes (#2537): one destination's notifications go
//! out in commit order whichever task committed them, a stalled destination
//! holds up its own lane only, the bounds drop the newest entry and count
//! it, a change's COV fanout waits for its own event notifications (capped),
//! and `stop()` sends nothing more.
//!
//! The wire here is the order in which the transport's sends finish, after
//! any hold, not the order they were logged in.

use super::*;
use crate::server::clocked_test_database;
use crate::server::event_notifications_tests::{
    local_broadcast_destination, notification_class_0_broadcasting,
};
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::constructed::{decode_event_log_record, decode_event_notification};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetAddress, BACnetDestination, EventLogDatum,
};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType, NotifyType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicU8;
use tokio::sync::{Semaphore, mpsc};

#[path = "event_lanes_fault_tests.rs"]
mod fault;
#[path = "event_lanes_hold_time_tests.rs"]
mod hold_time;
#[path = "event_lanes_wire_tests.rs"]
mod wire;

const WRITER: [u8; 6] = [10, 0, 0, 7, 0xBA, 0xC0];
const OTHER: [u8; 6] = [10, 0, 0, 8, 0xBA, 0xC0];
const SUBSCRIBER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const FAULT: u32 = 1;
const NORMAL: u32 = 0;
const OFFNORMAL: u32 = 2;

fn all_transitions() -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0xE0],
    }
}

fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

/// A notification as the wire carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sent {
    /// An event notification: broadcast or to [`OTHER`], its object, its
    /// To State, and whether it is an acknowledgment.
    Event {
        broadcast: bool,
        object: ObjectIdentifier,
        to: u32,
        ack: bool,
    },
    /// A COV notification of the object.
    Cov(ObjectIdentifier),
}

/// A broadcast transition of `object` to `to`.
fn ev(object: ObjectIdentifier, to: u32) -> Sent {
    Sent::Event {
        broadcast: true,
        object,
        to,
        ack: false,
    }
}

struct Probe {
    /// The next transition to hold in the transport: broadcast or not, and
    /// its To State.
    hold: StdMutex<Option<(bool, u32)>>,
    held: Semaphore,
    release: Semaphore,
    /// The next transition whose send panics, by its To State.
    panic_on: StdMutex<Option<u32>>,
    /// Every transition whose send panics, by its To State.
    panic_always: StdMutex<Option<u32>>,
    /// The SimpleACK to hold in the transport, by invoke ID, and its release.
    hold_ack: StdMutex<Option<u8>>,
    release_ack: Semaphore,
    wire: StdMutex<Vec<Sent>>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            hold: StdMutex::new(None),
            held: Semaphore::new(0),
            release: Semaphore::new(0),
            panic_on: StdMutex::new(None),
            panic_always: StdMutex::new(None),
            hold_ack: StdMutex::new(None),
            release_ack: Semaphore::new(0),
            wire: StdMutex::new(Vec::new()),
        }
    }
}

impl Probe {
    async fn on_send(&self, frame: SentFrame) {
        if let Apdu::SimpleAck(ack) = frame.apdu() {
            let hold = {
                let mut slot = self.hold_ack.lock().unwrap();
                let hold = *slot == Some(ack.invoke_id);
                if hold {
                    *slot = None;
                }
                hold
            };
            if hold {
                self.release_ack.acquire().await.unwrap().forget();
            }
            return;
        }
        let Apdu::UnconfirmedRequest(request) = frame.apdu() else {
            return;
        };
        let sent = if request.service_choice
            == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
        {
            let n = decode_event_notification(&request.service_request).unwrap();
            Sent::Event {
                broadcast: frame.broadcast,
                object: n.event_object_identifier,
                to: n.to_state.to_raw(),
                ack: n.notify_type == NotifyType::ACK_NOTIFICATION,
            }
        } else if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION {
            let n = COVNotificationRequest::decode(&request.service_request).unwrap();
            Sent::Cov(n.monitored_object_identifier)
        } else {
            return;
        };
        if let Sent::Event { to, ack: false, .. } = &sent {
            let mut slot = self.panic_on.lock().unwrap();
            let once = *slot == Some(*to);
            if once {
                *slot = None;
            }
            drop(slot);
            if once || *self.panic_always.lock().unwrap() == Some(*to) {
                panic!("a test transport's send panics");
            }
        }
        let hold = match &sent {
            Sent::Event {
                broadcast,
                to,
                ack: false,
                ..
            } => {
                let mut slot = self.hold.lock().unwrap();
                let hold = *slot == Some((*broadcast, *to));
                if hold {
                    *slot = None;
                }
                hold
            }
            _ => false,
        };
        if hold {
            self.held.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        self.wire.lock().unwrap().push(sent);
    }

    fn wire(&self) -> Vec<Sent> {
        self.wire.lock().unwrap().clone()
    }
}

struct Rig {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    probe: Arc<Probe>,
    invoke_id: AtomicU8,
}

impl Rig {
    async fn start(extend: impl FnOnce(&mut ObjectDatabase)) -> Self {
        Self::start_with(ServerConfig::default(), extend).await
    }

    async fn start_with(config: ServerConfig, extend: impl FnOnce(&mut ObjectDatabase)) -> Self {
        Self::start_bound(config, Vec::new(), extend).await
    }

    /// [`Self::start_with`], the server holding `bindings` as configured.
    async fn start_bound(
        config: ServerConfig,
        bindings: Vec<crate::server::device_bindings::DeviceBinding>,
        extend: impl FnOnce(&mut ObjectDatabase),
    ) -> Self {
        let mut db = clocked_test_database();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        db.add(Box::new(notification_class_0_broadcasting()))
            .unwrap();
        extend(&mut db);
        let probe = Arc::new(Probe::default());
        let on_send = Arc::clone(&probe);
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .inbound(rx)
            .on_send(move |frame| {
                let probe = Arc::clone(&on_send);
                async move {
                    probe.on_send(frame).await;
                    Ok(())
                }
            })
            .build();
        let sent = transport.sent();
        let clock = Some(crate::server::ClockConfig::default());
        let start = BACnetServer::start_with_clock_mode_and_bindings;
        let server = start(config, db, transport, clock, bindings)
            .await
            .expect("server should start");
        tokio::time::sleep(Duration::from_millis(200)).await;
        Self {
            server,
            tx,
            sent,
            probe,
            invoke_id: AtomicU8::new(0),
        }
    }

    fn hold(&self, broadcast: bool, to: u32) {
        *self.probe.hold.lock().unwrap() = Some((broadcast, to));
    }

    /// Wait until the transport holds a send.
    async fn holding(&self) {
        let held = tokio::time::timeout(Duration::from_secs(1), self.probe.held.acquire());
        held.await.expect("a send held").unwrap().forget();
    }

    fn release(&self) {
        self.probe.release.add_permits(1);
    }

    async fn request(&self, service_choice: ConfirmedServiceChoice, body: BytesMut) -> u8 {
        let invoke_id = self.invoke_id.fetch_add(1, Ordering::SeqCst) + 1;
        let mut payload = BytesMut::new();
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
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
        });
        encode_apdu(&mut payload, &pdu).unwrap();
        let mut npdu = BytesMut::new();
        let fields = Npdu {
            expecting_reply: true,
            payload: payload.freeze(),
            ..Npdu::default()
        };
        encode_npdu(&mut npdu, &fields).unwrap();
        self.tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: MacAddr::from_slice(&WRITER),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        invoke_id
    }

    async fn acked(&self, invoke_id: u8) {
        for _ in 0..5_000 {
            let found = self.sent.unicasts().into_iter().any(|frame| {
                frame.mac.as_slice() == WRITER
                    && matches!(frame.apdu(), Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id)
            });
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("no SimpleACK to {invoke_id}");
    }

    async fn write(
        &self,
        oid: ObjectIdentifier,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> u8 {
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: property,
            property_array_index: None,
            property_value: encoded(value),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
            .await
    }

    /// AI-1's Reliability, by WriteProperty, its SimpleACK awaited.
    async fn reliability(&self, reliability: Reliability) {
        let value = PropertyValue::Enumerated(reliability.to_raw());
        let id = self
            .write(ai1(), PropertyIdentifier::RELIABILITY, value)
            .await;
        self.acked(id).await;
    }

    async fn wpm(&self, writes: &[(ObjectIdentifier, PropertyValue, Option<u8>)]) -> u8 {
        let specs = writes
            .iter()
            .map(|(oid, value, priority)| WriteAccessSpecification {
                object_identifier: *oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PV,
                    property_array_index: None,
                    value: encoded(value.clone()),
                    priority: *priority,
                }],
            })
            .collect();
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        }
        .encode(&mut body)
        .unwrap();
        self.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
            .await
    }

    async fn event_state(&self, oid: ObjectIdentifier) -> u32 {
        let db = self.server.database().read().await;
        match db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
        {
            Ok(PropertyValue::Enumerated(state)) => state,
            other => panic!("{other:?}"),
        }
    }

    async fn wait_wire(&self, count: usize) -> Vec<Sent> {
        for _ in 0..2_000 {
            let wire = self.probe.wire();
            if wire.len() >= count {
                return wire;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        self.probe.wire()
    }
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}
fn bv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap()
}
fn cmd1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COMMAND, 1).unwrap()
}

/// AI-1, out of service so a peer may write its Reliability, reporting every
/// transition through Notification Class 0.
fn ai_objects(db: &mut ObjectDatabase) {
    let mut object = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oos = PropertyValue::Boolean(true);
    object
        .write_property(PropertyIdentifier::OUT_OF_SERVICE, None, oos, None)
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            all_transitions(),
            None,
        )
        .unwrap();
    db.add(Box::new(object)).unwrap();
}

/// BV-1 reporting every transition; CMD-1 whose list 1 writes BV-1 inactive
/// at priority 8.
fn command_objects(db: &mut ObjectDatabase) {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        all_transitions(),
        None,
    )
    .unwrap();
    db.add(Box::new(bv)).unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: bv1(),
                property_identifier: PV,
                property_array_index: None,
                property_value: PropertyValue::Enumerated(0),
                priority: Some(8),
                post_delay: None,
                quit_on_failure: false,
                write_successful: true,
            }],
        }])
        .unwrap();
    db.add(Box::new(command)).unwrap();
}

/// P1 (#2537): one client, two WriteProperty requests to AI-1 one after the
/// other (fault, then clear), the first's FAULT notification held in the
/// transport. The second is sent only after the first's SimpleACK. Before the
/// lanes, the clear's NORMAL went out first.
#[tokio::test(start_paused = true)]
async fn sequential_writes_from_one_client_go_out_in_commit_order() {
    let rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), [], "the clear waits behind the fault");
    rig.release();
    let wire = rig.wait_wire(2).await;
    assert_eq!(wire, [ev(ai1(), FAULT), ev(ai1(), NORMAL)]);
    assert_eq!(rig.event_state(ai1()).await, NORMAL);
}

/// P2: one WPM writes BV-1 active at 8 (OFFNORMAL) and CMD-1 = 1, whose run
/// writes BV-1 inactive at 8 (NORMAL). The WPM's own OFFNORMAL send is held;
/// the run's NORMAL waits behind it.
#[tokio::test(start_paused = true)]
async fn a_run_a_request_starts_goes_out_after_the_requests_own() {
    let rig = Rig::start(command_objects).await;
    rig.hold(true, OFFNORMAL);
    let id = rig
        .wpm(&[
            (bv1(), PropertyValue::Enumerated(1), Some(8)),
            (cmd1(), PropertyValue::Unsigned(1), None),
        ])
        .await;
    rig.acked(id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    rig.release();
    let wire = rig.wait_wire(2).await;
    assert_eq!(wire, [ev(bv1(), OFFNORMAL), ev(bv1(), NORMAL)]);
    assert_eq!(rig.event_state(bv1()).await, NORMAL);
}

/// P2b: P2, released before its SimpleACK is looked for.
#[tokio::test(start_paused = true)]
async fn a_run_waits_behind_the_requests_own_without_waiting_for_its_ack() {
    let rig = Rig::start(command_objects).await;
    rig.hold(true, OFFNORMAL);
    let id = rig
        .wpm(&[
            (bv1(), PropertyValue::Enumerated(1), Some(8)),
            (cmd1(), PropertyValue::Unsigned(1), None),
        ])
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    rig.release();
    rig.acked(id).await;
    let wire = rig.wait_wire(2).await;
    assert_eq!(wire, [ev(bv1(), OFFNORMAL), ev(bv1(), NORMAL)]);
}

/// Transitions are logged to the Event Logs under the guard that commits
/// them, so in commit order, whatever their sends wait for.
#[tokio::test(start_paused = true)]
async fn the_event_logs_take_transitions_in_commit_order_while_a_send_is_held() {
    let el1 = ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap();
    let rig = Rig::start(|db| {
        ai_objects(db);
        db.add(Box::new(EventLogObject::new(1, "EL-1", 8).unwrap()))
            .unwrap();
    })
    .await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    let logged = {
        let db = rig.server.database().read().await;
        let records = db.get(&el1).unwrap().log_buffer().unwrap();
        let records = records.buffer_records().unwrap();
        (0..records.record_count())
            .filter_map(|at| {
                let mut buf = BytesMut::new();
                records.encode_record(at, &mut buf);
                match decode_event_log_record(&buf, 0).unwrap().0.log_datum {
                    EventLogDatum::Notification(n) => Some(n.to_state.to_raw()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(logged, [FAULT, NORMAL], "logged as committed");
    assert_eq!(rig.probe.wire(), [], "neither sent yet");
    rig.release();
    assert_eq!(
        rig.wait_wire(2).await,
        [ev(ai1(), FAULT), ev(ai1(), NORMAL)]
    );
}

/// A destination whose send is held holds up its own lane only: the same
/// transition reaches another recipient at once.
#[tokio::test(start_paused = true)]
async fn a_held_lane_does_not_hold_another_recipient() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        let mut class = NotificationClass::new(0, "NC-0").unwrap();
        class
            .add_destination(local_broadcast_destination())
            .unwrap();
        class
            .add_destination(BACnetDestination {
                recipient: BACnetRecipient::Address(BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(&OTHER),
                }),
                ..local_broadcast_destination()
            })
            .unwrap();
        // Replaces the broadcast-only class the rig adds.
        db.remove(&class.object_identifier()).unwrap();
        db.add(Box::new(class)).unwrap();
    })
    .await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    let other = Sent::Event {
        broadcast: false,
        object: ai1(),
        to: FAULT,
        ack: false,
    };
    assert_eq!(
        rig.wait_wire(1).await,
        std::slice::from_ref(&other),
        "not held"
    );
    rig.release();
    assert_eq!(rig.wait_wire(2).await, [other, ev(ai1(), FAULT)]);
}

/// The lanes' bounds drop the newest entry and say which one it hit.
#[test]
fn a_full_lane_full_lanes_and_too_many_lanes_drop_the_newest() {
    let notification = EventNotificationRequest {
        process_identifier: 0,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ai1(),
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        notification_class: 0,
        priority: 0,
        event_type: EventType::CHANGE_OF_RELIABILITY,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::FAULT,
        event_values: None,
    };
    let payload = Arc::new(LanePayload::local(notification));
    let mut settle = Settle::default();
    let mut entry = || LaneEntry::new(&payload, 0, false, Some(&mut settle));
    let device = |instance| {
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
    };
    let mut lanes = EventLanes::default();
    for _ in 0..MAX_LANE_ENTRIES {
        assert!(matches!(
            lanes.push(&device(0), entry()),
            Pushed::Opened | Pushed::Joined
        ));
    }
    assert!(matches!(
        lanes.push(&device(0), entry()),
        Pushed::Dropped(LaneDrop::OwnLane)
    ));
    for instance in 1..(MAX_LANED_ENTRIES / MAX_LANE_ENTRIES) as u32 {
        for _ in 0..MAX_LANE_ENTRIES {
            lanes.push(&device(instance), entry());
        }
    }
    assert_eq!(lanes.waiting(), MAX_LANED_ENTRIES);
    assert!(matches!(
        lanes.push(&device(999), entry()),
        Pushed::Dropped(LaneDrop::AllEntries)
    ));
    let mut lanes = EventLanes::default();
    for instance in 0..MAX_LANES as u32 {
        assert!(matches!(
            lanes.push(&device(instance), entry()),
            Pushed::Opened
        ));
    }
    assert!(matches!(
        lanes.push(&device(MAX_LANES as u32), entry()),
        Pushed::Dropped(LaneDrop::Destinations)
    ));
    assert!(matches!(lanes.push(&device(0), entry()), Pushed::Joined));
    lanes.seal();
    assert!(matches!(lanes.push(&device(0), entry()), Pushed::Sealed));
    assert_eq!(lanes.waiting(), 0);
}

/// A writer that pushed nothing has nothing to wait on; one whose entries
/// are all gone, dropped or sealed away, waits no more.
#[tokio::test(start_paused = true)]
async fn a_settle_resolves_once_its_entries_are_gone_and_none_is_no_wait() {
    assert!(Settle::default().into_wait().is_none(), "no handle");
    let mut settle = Settle::default();
    let token = settle.token();
    let waiting = settle.into_wait().expect("a pushed entry");
    drop(token);
    assert!(settled_within(Some(waiting), 1).await);
}
