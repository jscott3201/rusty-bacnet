//! A write and the intrinsic transition it causes commit as one step
//! (#2530): a ReadProperty that waits for the database behind the write
//! reads the object before both or after both, never with the new
//! Reliability or value beside the old Event_State.
//!
//! Each read test holds the database, queues the write, queues a second
//! peer's ReadProperty of Status_Flags behind it (Tokio grants the lock in
//! turn), then lets both go. With the two steps under separate guards, the
//! read landed between them. The order tests check that committed
//! transitions reach the wire in the order they committed.

use super::*;
use crate::server::event_notifications_tests::notification_class_0_broadcasting;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::read_property::{ReadPropertyAckRef, ReadPropertyRequest};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{EngineeringUnits, EventState, Reliability};
use std::sync::atomic::{AtomicBool, AtomicU8};
use tokio::sync::{Semaphore, mpsc};

const IN_ALARM: u8 = 0x80;
const FAULT: u8 = 0x40;
const OUT_OF_SERVICE: u8 = 0x10;
const WRITER: [u8; 6] = [10, 0, 0, 7, 0xBA, 0xC0];
const READER: [u8; 6] = [10, 0, 0, 8, 0xBA, 0xC0];
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn bv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap()
}

fn stg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
}

fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

/// Every transition, for Event_Enable.
fn all_transitions() -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0xE0],
    }
}

/// Holds the next SimpleACK to the writer once armed, until released.
struct AckHold {
    armed: AtomicBool,
    held: Semaphore,
    release: Semaphore,
}

impl AckHold {
    async fn on_send(&self, frame: SentFrame) {
        if !frame.broadcast
            && frame.mac.as_slice() == WRITER
            && matches!(frame.apdu(), Apdu::SimpleAck(_))
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.held.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
    }
}

struct Gap {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    hold: Arc<AckHold>,
    invoke_id: AtomicU8,
}

impl Gap {
    /// AI-1 and AI-2, reporting every transition, out of service when
    /// `out_of_service` so a peer may write their Reliability.
    async fn start(out_of_service: bool) -> Self {
        Self::start_with(|db| {
            for instance in [1, 2] {
                let name = format!("AI-{instance}");
                let units = EngineeringUnits::DEGREES_CELSIUS;
                let mut object = AnalogInputObject::new(instance, &name, units).unwrap();
                let oos = PropertyValue::Boolean(out_of_service);
                for (property, value) in [
                    (PropertyIdentifier::OUT_OF_SERVICE, oos),
                    (PropertyIdentifier::EVENT_ENABLE, all_transitions()),
                ] {
                    object.write_property(property, None, value, None).unwrap();
                }
                db.add(Box::new(object)).unwrap();
            }
        })
        .await
    }

    /// A Device, Notification Class 0 broadcasting every transition, and
    /// what `extend` adds.
    async fn start_with(extend: impl FnOnce(&mut ObjectDatabase)) -> Self {
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
        let hold = Arc::new(AckHold {
            armed: AtomicBool::new(false),
            held: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        let on_send = Arc::clone(&hold);
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .inbound(rx)
            .on_send(move |frame| {
                let hold = Arc::clone(&on_send);
                async move {
                    hold.on_send(frame).await;
                    Ok(())
                }
            })
            .build();
        let sent = transport.sent();
        let server = BACnetServer::start(ServerConfig::default(), db, transport)
            .await
            .expect("server should start");
        // Clear of the sweeps at whole seconds.
        tokio::time::sleep(Duration::from_millis(200)).await;
        Self {
            server,
            tx,
            sent,
            hold,
            invoke_id: AtomicU8::new(0),
        }
    }

    /// Deliver a confirmed request from `peer`; its invoke ID.
    async fn request(
        &self,
        peer: &[u8],
        service_choice: ConfirmedServiceChoice,
        body: BytesMut,
    ) -> u8 {
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
                source_mac: MacAddr::from_slice(peer),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        invoke_id
    }

    /// The answer `peer` got to its request `invoke_id`, waited for on the
    /// paused clock.
    async fn response(&self, peer: &[u8], invoke_id: u8) -> Apdu {
        for _ in 0..1_000 {
            let answer = self.sent.unicasts().into_iter().find_map(|frame| {
                if frame.mac.as_slice() != peer {
                    return None;
                }
                let apdu = frame.apdu();
                let id = match &apdu {
                    Apdu::SimpleAck(ack) => ack.invoke_id,
                    Apdu::ComplexAck(ack) => ack.invoke_id,
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
        panic!("no answer to request {invoke_id}");
    }

    async fn acknowledged(&self, peer: &[u8], invoke_id: u8) {
        let answer = self.response(peer, invoke_id).await;
        assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    }

    /// `peer`'s WriteProperty of `oid`'s `property`.
    async fn send_write(
        &self,
        peer: &[u8],
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
        self.request(peer, ConfirmedServiceChoice::WRITE_PROPERTY, body)
            .await
    }

    /// The writer's WriteProperty of `oid`'s Reliability.
    async fn send_reliability(&self, oid: ObjectIdentifier, reliability: Reliability) -> u8 {
        let value = PropertyValue::Enumerated(reliability.to_raw());
        self.send_write(&WRITER, oid, PropertyIdentifier::RELIABILITY, value)
            .await
    }

    /// The writer's WritePropertyMultiple of `writes`, one specification
    /// each: object, property, value and priority.
    async fn send_multiple(
        &self,
        writes: &[(
            ObjectIdentifier,
            PropertyIdentifier,
            PropertyValue,
            Option<u8>,
        )],
    ) -> u8 {
        let specs = writes
            .iter()
            .map(
                |(oid, property, value, priority)| WriteAccessSpecification {
                    object_identifier: *oid,
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: *property,
                        property_array_index: None,
                        value: encoded(value.clone()),
                        priority: *priority,
                    }],
                },
            )
            .collect();
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        self.request(&WRITER, service, body).await
    }

    /// The writer's WritePropertyMultiple of each object's Reliability.
    async fn send_reliabilities(&self, oids: &[ObjectIdentifier], reliability: Reliability) -> u8 {
        let value = PropertyValue::Enumerated(reliability.to_raw());
        let writes: Vec<_> = oids
            .iter()
            .map(|oid| (*oid, PropertyIdentifier::RELIABILITY, value.clone(), None))
            .collect();
        self.send_multiple(&writes).await
    }

    /// The reader's ReadProperty of `oid`'s Status_Flags; its invoke ID.
    async fn send_read_flags(&self, oid: ObjectIdentifier) -> u8 {
        let mut body = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::STATUS_FLAGS,
            property_array_index: None,
        }
        .encode(&mut body);
        let service = ConfirmedServiceChoice::READ_PROPERTY;
        self.request(&READER, service, body).await
    }

    /// The Status_Flags the reader's request `invoke_id` was answered with.
    async fn read_flags(&self, invoke_id: u8) -> u8 {
        let Apdu::ComplexAck(ack) = self.response(&READER, invoke_id).await else {
            panic!("the read is answered with a ComplexACK");
        };
        let value = ReadPropertyAckRef::decode(&ack.service_ack).unwrap();
        let value = value.property_value;
        assert_eq!(&value[..2], &[0x82, 0x04], "four-bit Status_Flags");
        value[2]
    }

    /// Run `write` with the database held, queue the reader's ReadProperty
    /// of `read`'s Status_Flags behind it, then let both go. What `write`
    /// returned, and the flags the reader got.
    async fn read_behind<F: std::future::Future>(
        &self,
        read: ObjectIdentifier,
        write: F,
    ) -> (F::Output, u8) {
        let held = self.server.database().read().await;
        tokio::join!(write, async {
            // The write queues for the database, then the read behind it.
            tokio::time::sleep(Duration::from_millis(1)).await;
            let id = self.send_read_flags(read).await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            drop(held);
            self.read_flags(id).await
        })
    }

    /// [`Self::read_behind`] for a request that takes the database twice,
    /// a first guard (a durable stage, or the write that starts a Staging
    /// plan) and then the one under test: the test takes the database back
    /// between the two, so the second queues behind it and the read behind
    /// that.
    async fn read_behind_second<F: std::future::Future>(
        &self,
        read: ObjectIdentifier,
        write: F,
    ) -> (F::Output, u8) {
        let database = self.server.database();
        let first = database.read().await;
        let (second, written) = tokio::join!(
            async {
                // Queued behind the request's first guard.
                tokio::time::sleep(Duration::from_millis(1)).await;
                database.read().await
            },
            async {
                let written = write.await;
                tokio::time::sleep(Duration::from_millis(2)).await;
                drop(first);
                written
            }
        );
        // The request's second guard queues behind the test, the read
        // behind it.
        tokio::time::sleep(Duration::from_millis(1)).await;
        let id = self.send_read_flags(read).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        drop(second);
        (written, self.read_flags(id).await)
    }

    async fn event_state(&self, oid: ObjectIdentifier) -> u32 {
        let db = self.server.database().read().await;
        let object = db.get(&oid).unwrap();
        match object.read_property(PropertyIdentifier::EVENT_STATE, None) {
            Ok(PropertyValue::Enumerated(state)) => state,
            other => panic!("Event_State read as {other:?}"),
        }
    }

    /// The object and to-state of each EventNotification broadcast, in
    /// order.
    fn events(&self) -> Vec<(ObjectIdentifier, EventState)> {
        self.sent
            .broadcasts()
            .iter()
            .filter_map(|frame| match frame.apdu() {
                Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION =>
                {
                    let notification = decode_event_notification(&request.service_request);
                    let notification = notification.unwrap();
                    Some((notification.event_object_identifier, notification.to_state))
                }
                _ => None,
            })
            .collect()
    }
}

/// BV-1, reporting every transition, and STG-1, whose three stages command
/// it at priority 8: off below 30, on from 50.
fn staging(db: &mut ObjectDatabase) {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        all_transitions(),
        None,
    )
    .unwrap();
    db.add(Box::new(bv)).unwrap();
    let stage = |limit, active| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value: 5.0,
        min_present_value: 0.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(30.0, false), stage(50.0, true)],
        target_references: vec![BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: bv1(),
        }],
        stage_names: None,
    };
    db.add(Box::new(StagingObject::new(1, "STG-1", config).unwrap()))
        .unwrap();
}

/// The application clears a fault: the read sees no IN_ALARM without FAULT.
#[tokio::test(start_paused = true)]
async fn a_read_behind_a_local_clear_sees_no_half_cleared_flags() {
    let gap = Gap::start(false).await;
    let oid = ai(1);
    let server = &gap.server;
    server
        .set_reliability_local(&oid, Reliability::NO_SENSOR)
        .await
        .unwrap();
    assert_eq!(gap.event_state(oid).await, EventState::FAULT.to_raw());

    let clear = server.set_reliability_local(&oid, Reliability::NO_FAULT_DETECTED);
    let (cleared, flags) = gap.read_behind(oid, clear).await;
    cleared.unwrap();
    assert_eq!(flags, 0, "read after the clear and its transition both");
    assert_eq!(gap.event_state(oid).await, EventState::NORMAL.to_raw());
}

/// A peer simulates a fault on AI-1, its Out_Of_Service TRUE, and clears it
/// with WriteProperty: the read sees FAULT and IN_ALARM together, then
/// neither.
#[tokio::test(start_paused = true)]
async fn a_read_behind_a_peer_write_property_sees_no_half_updated_flags() {
    let gap = Gap::start(true).await;
    let oid = ai(1);
    let fault = gap.send_reliability(oid, Reliability::NO_SENSOR);
    let (id, flags) = gap.read_behind(oid, fault).await;
    gap.acknowledged(&WRITER, id).await;
    assert_eq!(
        flags,
        IN_ALARM | FAULT | OUT_OF_SERVICE,
        "the fault and its transition"
    );
    assert_eq!(gap.event_state(oid).await, EventState::FAULT.to_raw());

    let clear = gap.send_reliability(oid, Reliability::NO_FAULT_DETECTED);
    let (id, flags) = gap.read_behind(oid, clear).await;
    gap.acknowledged(&WRITER, id).await;
    assert_eq!(flags, OUT_OF_SERVICE, "the clear and its transition");
    assert_eq!(gap.event_state(oid).await, EventState::NORMAL.to_raw());
}

/// One WritePropertyMultiple clears AI-1 and AI-2: the read of AI-2, the
/// second written, sees both steps of its clear or neither.
#[tokio::test(start_paused = true)]
async fn a_read_behind_a_write_property_multiple_sees_no_half_cleared_flags() {
    let gap = Gap::start(true).await;
    let oids = [ai(1), ai(2)];
    let id = gap.send_reliabilities(&oids, Reliability::NO_SENSOR).await;
    gap.acknowledged(&WRITER, id).await;
    for oid in oids {
        assert_eq!(gap.event_state(oid).await, EventState::FAULT.to_raw());
    }

    let clear = gap.send_reliabilities(&oids, Reliability::NO_FAULT_DETECTED);
    let (id, flags) = gap.read_behind(oids[1], clear).await;
    gap.acknowledged(&WRITER, id).await;
    assert_eq!(flags, OUT_OF_SERVICE, "the clear and its transition");
    for oid in oids {
        assert_eq!(gap.event_state(oid).await, EventState::NORMAL.to_raw());
    }
}

/// An AddListElement puts MSI-1's state 2 in Alarm_Values: the read sees
/// IN_ALARM with the edit, or neither.
#[tokio::test(start_paused = true)]
async fn a_read_behind_a_list_edit_sees_no_half_moved_flags() {
    let gap = Gap::start_with(|db| {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_present_value(2);
        db.add(Box::new(msi)).unwrap();
    })
    .await;
    let msi1 = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    let mut body = BytesMut::new();
    ListElementRequest {
        object_identifier: msi1,
        property_identifier: PropertyIdentifier::ALARM_VALUES,
        property_array_index: None,
        list_of_elements: vec![0x21, 2],
    }
    .encode(&mut body)
    .unwrap();
    // Alarm_Values is kept durably, so the edit stages under a guard first.
    let edit = gap.request(&WRITER, ConfirmedServiceChoice::ADD_LIST_ELEMENT, body);
    let (id, flags) = gap.read_behind_second(msi1, edit).await;
    gap.acknowledged(&WRITER, id).await;
    assert_eq!(flags, IN_ALARM, "the edit and its transition");
    assert_eq!(gap.event_state(msi1).await, EventState::OFFNORMAL.to_raw());
}

/// A peer's write of STG-1 moves it to the stage that turns BV-1 on, and the
/// plan writes BV-1 under a guard of its own: the read queued behind that
/// guard sees BV-1 on and in alarm together.
#[tokio::test(start_paused = true)]
async fn a_read_behind_a_staging_target_write_sees_no_half_moved_flags() {
    let gap = Gap::start_with(staging).await;
    let write = gap.send_write(&WRITER, stg1(), PV, PropertyValue::Real(60.0));
    let (id, flags) = gap.read_behind_second(bv1(), write).await;
    gap.acknowledged(&WRITER, id).await;
    assert_eq!(flags, IN_ALARM, "the target write and its transition");
    assert_eq!(gap.event_state(bv1()).await, EventState::OFFNORMAL.to_raw());
}

/// One WritePropertyMultiple turns BV-1 on at priority 8, then moves STG-1
/// to a stage whose plan turns it off at the same priority. BV-1 goes into
/// alarm and back, and the wire carries the two transitions in that order,
/// so the last to-state sent is the state BV-1 is in.
#[tokio::test(start_paused = true)]
async fn a_write_and_the_staging_plan_it_starts_send_their_transitions_in_order() {
    let gap = Gap::start_with(staging).await;
    let before = gap.events().len();
    let id = gap
        .send_multiple(&[
            (bv1(), PV, PropertyValue::Enumerated(1), Some(8)),
            (stg1(), PV, PropertyValue::Real(20.0), None),
        ])
        .await;
    gap.acknowledged(&WRITER, id).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        gap.events()[before..],
        [(bv1(), EventState::OFFNORMAL), (bv1(), EventState::NORMAL)],
        "the write's transition, then the plan's"
    );
    assert_eq!(gap.event_state(bv1()).await, EventState::NORMAL.to_raw());
}

/// Two peers fault and clear AI-1 while the first one's SimpleACK is held
/// on its way out: the first one's FAULT is on the wire already, before its
/// SimpleACK and before the second's NORMAL, so the last to-state sent is
/// the state AI-1 is in.
#[tokio::test(start_paused = true)]
async fn two_peers_writes_send_their_transitions_in_commit_order() {
    let gap = Gap::start(true).await;
    let oid = ai(1);
    gap.hold.armed.store(true, Ordering::SeqCst);
    let first = gap.send_reliability(oid, Reliability::NO_SENSOR).await;
    let held = tokio::time::timeout(Duration::from_secs(5), gap.hold.held.acquire());
    held.await
        .expect("the first SimpleACK is held")
        .unwrap()
        .forget();
    assert_eq!(
        gap.events(),
        [(oid, EventState::FAULT)],
        "sent before its ACK"
    );

    let value = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    let second = gap
        .send_write(&READER, oid, PropertyIdentifier::RELIABILITY, value)
        .await;
    gap.acknowledged(&READER, second).await;
    gap.hold.release.add_permits(1);
    gap.acknowledged(&WRITER, first).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        gap.events(),
        [(oid, EventState::FAULT), (oid, EventState::NORMAL)],
        "the transitions in the order they committed"
    );
    assert_eq!(gap.event_state(oid).await, EventState::NORMAL.to_raw());
}
