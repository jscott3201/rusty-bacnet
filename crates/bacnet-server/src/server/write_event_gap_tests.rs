//! A write and the intrinsic transition it causes commit as one step
//! (#2530): a ReadProperty that waits for the database behind the write
//! reads the object before both or after both, never with the new
//! Reliability beside the old Event_State.
//!
//! Each test holds the database, queues the write, queues a second peer's
//! ReadProperty of Status_Flags behind it (Tokio grants the lock in turn),
//! then lets both go. With the two steps under separate guards, the read
//! landed between them.

use super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, TestTransport};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::read_property::{ReadPropertyAckRef, ReadPropertyRequest};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{EngineeringUnits, EventState, Reliability};
use std::sync::atomic::AtomicU8;
use tokio::sync::mpsc;

const IN_ALARM: u8 = 0x80;
const FAULT: u8 = 0x40;
const OUT_OF_SERVICE: u8 = 0x10;
const WRITER: [u8; 6] = [10, 0, 0, 7, 0xBA, 0xC0];
const READER: [u8; 6] = [10, 0, 0, 8, 0xBA, 0xC0];

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn enumerated(value: u32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &PropertyValue::Enumerated(value)).unwrap();
    buf.to_vec()
}

struct Gap {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    invoke_id: AtomicU8,
}

impl Gap {
    /// A Device, and AI-1 and AI-2, out of service when `out_of_service`
    /// so a peer may write their Reliability.
    async fn start(out_of_service: bool) -> Self {
        let mut db = clocked_test_database();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        for instance in [1, 2] {
            let name = format!("AI-{instance}");
            let mut object =
                AnalogInputObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
            let oos = PropertyValue::Boolean(out_of_service);
            object
                .write_property(PropertyIdentifier::OUT_OF_SERVICE, None, oos, None)
                .unwrap();
            db.add(Box::new(object)).unwrap();
        }
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .inbound(rx)
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

    async fn acknowledged(&self, invoke_id: u8) {
        let answer = self.response(&WRITER, invoke_id).await;
        assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    }

    /// The writer's WriteProperty of `oid`'s Reliability.
    async fn send_reliability(&self, oid: ObjectIdentifier, reliability: Reliability) -> u8 {
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::RELIABILITY,
            property_array_index: None,
            property_value: enumerated(reliability.to_raw()),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        self.request(&WRITER, ConfirmedServiceChoice::WRITE_PROPERTY, body)
            .await
    }

    /// The writer's WritePropertyMultiple of each object's Reliability.
    async fn send_reliabilities(&self, oids: &[ObjectIdentifier], reliability: Reliability) -> u8 {
        let specs = oids
            .iter()
            .map(|oid| WriteAccessSpecification {
                object_identifier: *oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::RELIABILITY,
                    property_array_index: None,
                    value: enumerated(reliability.to_raw()),
                    priority: None,
                }],
            })
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
            let mut body = BytesMut::new();
            ReadPropertyRequest {
                object_identifier: read,
                property_identifier: PropertyIdentifier::STATUS_FLAGS,
                property_array_index: None,
            }
            .encode(&mut body);
            let service = ConfirmedServiceChoice::READ_PROPERTY;
            let id = self.request(&READER, service, body).await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            drop(held);
            let Apdu::ComplexAck(ack) = self.response(&READER, id).await else {
                panic!("the read is answered with a ComplexACK");
            };
            let value = ReadPropertyAckRef::decode(&ack.service_ack).unwrap();
            let value = value.property_value;
            assert_eq!(&value[..2], &[0x82, 0x04], "four-bit Status_Flags");
            value[2]
        })
    }

    async fn event_state(&self, oid: ObjectIdentifier) -> u32 {
        let db = self.server.database().read().await;
        let object = db.get(&oid).unwrap();
        match object.read_property(PropertyIdentifier::EVENT_STATE, None) {
            Ok(PropertyValue::Enumerated(state)) => state,
            other => panic!("Event_State read as {other:?}"),
        }
    }
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
    gap.acknowledged(id).await;
    assert_eq!(
        flags,
        IN_ALARM | FAULT | OUT_OF_SERVICE,
        "the fault and its transition"
    );
    assert_eq!(gap.event_state(oid).await, EventState::FAULT.to_raw());

    let clear = gap.send_reliability(oid, Reliability::NO_FAULT_DETECTED);
    let (id, flags) = gap.read_behind(oid, clear).await;
    gap.acknowledged(id).await;
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
    gap.acknowledged(id).await;
    for oid in oids {
        assert_eq!(gap.event_state(oid).await, EventState::FAULT.to_raw());
    }

    let clear = gap.send_reliabilities(&oids, Reliability::NO_FAULT_DETECTED);
    let (id, flags) = gap.read_behind(oids[1], clear).await;
    gap.acknowledged(id).await;
    assert_eq!(flags, OUT_OF_SERVICE, "the clear and its transition");
    for oid in oids {
        assert_eq!(gap.event_state(oid).await, EventState::NORMAL.to_raw());
    }
}
