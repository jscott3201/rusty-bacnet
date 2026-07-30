//! Wire-level proof that CHANGE_OF_STATE alarm commissioning and each
//! `Event_Enable` bit reach notification distribution.
//!
//! `event_notifications_tests.rs` pins the TO_OFFNORMAL half of this gate for
//! AnalogInput and has little headroom left under the 700-LOC cap, so the
//! Multi-state Input cases live here alongside Binary Input and Multi-state
//! Value proofs that Alarm_Value(s) can be commissioned through working
//! network routes.
//!
//! Both directions are covered on purpose. A gate written `event_enable & 0x01`
//! instead of `event_enable & transition_bit` distributes TO_OFFNORMAL correctly
//! and every other transition wrongly, so asserting only the off-normal
//! direction leaves that mistake invisible.

use super::*;
use crate::handlers::{handle_add_list_element, handle_remove_list_element};
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::binary::{BinaryInputObject, BinaryValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::{MultiStateInputObject, MultiStateValueObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{EventState, EventType, NotifyType};
use bytes::{Bytes, BytesMut};
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

/// Records every broadcast NPDU and discards unicasts.
#[derive(Clone, Default)]
struct RecordingTransport {
    sent_broadcast: StdArc<StdMutex<Vec<Bytes>>>,
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.sent_broadcast
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xBA, 0xC0]
    }
}

/// The state value the fixture treats as an alarm, and the one it does not.
const ALARM_STATE: u64 = 2;
const NORMAL_STATE: u64 = 1;

/// A Multi-state Input in a one-object database, plus what the per-write
/// notification path needs to run against it.
struct Fixture {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<RecordingTransport>>,
    comm_state: Arc<AtomicU8>,
    server_tsm: Arc<Mutex<ServerTsm>>,
    sent: StdArc<StdMutex<Vec<Bytes>>>,
    oid: ObjectIdentifier,
}

impl Fixture {
    /// Build a Multi-state Input holding `bits` in `Event_Enable`, with
    /// [`ALARM_STATE`] as its only alarm value and `Out_Of_Service` TRUE so that
    /// `Present_Value` accepts network writes.
    ///
    /// Scalar commissioning uses object dispatch. Alarm_Values uses
    /// AddListElement through its handler, the only working network route
    /// today; whole-list WriteProperty remains blocked by decoder issue #182.
    async fn new(bits: EventTransitionBits) -> Self {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        for (property, value) in [
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
            (
                PropertyIdentifier::EVENT_ENABLE,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![bits.to_bacnet()],
                },
            ),
            // EVENT, not the default ALARM: the send site falls back to ALARM
            // when it cannot read Notify_Type, so a fixture left at the default
            // would assert nothing about the property actually reaching the wire.
            (
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
            ),
        ] {
            msi.write_property(property, None, value, None)
                .unwrap_or_else(|e| panic!("{property:?} must be writable since #229: {e:?}"));
        }
        let fixture = Self::from_object(Box::new(msi)).await;
        fixture.add_alarm_value(ALARM_STATE as u8).await;
        fixture
    }

    async fn from_object(object: Box<dyn BACnetObject>) -> Self {
        let oid = object.object_identifier();

        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 1,
                name: "Dev".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();

        let sent = StdArc::new(StdMutex::new(Vec::new()));
        Self {
            db: Arc::new(RwLock::new(db)),
            network: Arc::new(NetworkLayer::new(RecordingTransport {
                sent_broadcast: StdArc::clone(&sent),
            })),
            comm_state: Arc::new(AtomicU8::new(0)), // DCC not blocking
            server_tsm: Arc::new(Mutex::new(ServerTsm::new())),
            sent,
            oid,
        }
    }

    /// Write `Present_Value` and run the per-write notification path once, the
    /// same two steps a WriteProperty request performs.
    async fn write_present_value(&self, value: u64) {
        self.write_present_value_as(PropertyValue::Unsigned(value))
            .await;
    }

    async fn write_present_value_as(&self, value: PropertyValue) {
        self.db
            .write()
            .await
            .get_mut(&self.oid)
            .expect("the MSI is in the database")
            .write_property(PropertyIdentifier::PRESENT_VALUE, None, value, None)
            .expect("Out_Of_Service is TRUE, so Present_Value must be writable");

        BACnetServer::<RecordingTransport>::fire_event_notifications(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            &self.oid,
            1000,
        )
        .await;
    }

    /// Commission one Alarm_Values element through the AddListElement handler.
    async fn add_alarm_value(&self, value: u8) {
        let request = ListElementRequest {
            object_identifier: self.oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            list_of_elements: vec![0x21, value],
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded);
        let mut db = self.db.write().await;
        handle_add_list_element(&mut db, &encoded)
            .expect("AddListElement is the working network Alarm_Values route");
    }

    async fn remove_alarm_value(&self, value: u8) {
        let request = ListElementRequest {
            object_identifier: self.oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            list_of_elements: vec![0x21, value],
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded);
        let mut db = self.db.write().await;
        handle_remove_list_element(&mut db, &encoded)
            .expect("RemoveListElement must remove the commissioned alarm value");
    }

    /// Take the broadcasts recorded since the last call.
    fn drain(&self) -> Vec<Bytes> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }

    async fn event_state(&self) -> PropertyValue {
        self.db
            .read()
            .await
            .get(&self.oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap()
    }
}

/// Decode the one broadcast expected in `sent` and assert it is a CHANGE_OF_STATE
/// notification for the fixture's MSI describing `from` -> `to`, carrying the
/// `Notify_Type` the fixture commissioned.
fn assert_sole_notification(
    sent: Vec<Bytes>,
    oid: ObjectIdentifier,
    from: EventState,
    to: EventState,
    context: &str,
) {
    assert_eq!(
        sent.len(),
        1,
        "{context}: expected exactly one broadcast EventNotification"
    );
    let npdu = decode_npdu(sent[0].clone()).expect("decode NPDU");
    let notif = match decode_apdu(npdu.payload).expect("decode APDU") {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
            EventNotificationRequest::decode(&req.service_request)
                .expect("decode EventNotification")
        }
        other => panic!("{context}: expected UnconfirmedRequest, got {other:?}"),
    };

    assert_eq!(
        notif.event_object_identifier, oid,
        "{context}: notification must name the commissioned object"
    );
    assert_eq!(
        notif.event_type,
        EventType::CHANGE_OF_STATE.to_raw(),
        "{context}: the detector's CHANGE_OF_STATE algorithm must reach the wire"
    );
    assert_eq!(
        notif.from_state,
        from.to_raw(),
        "{context}: from_state on the wire"
    );
    assert_eq!(
        notif.to_state,
        to.to_raw(),
        "{context}: to_state on the wire"
    );
    assert_eq!(
        notif.notify_type,
        NotifyType::EVENT.to_raw(),
        "{context}: the commissioned Notify_Type must reach the wire, not the ALARM fallback"
    );
}

/// TO_OFFNORMAL alone distributes the entry into OFFNORMAL; every other single
/// bit leaves it undistributed while the transition itself still happens.
#[tokio::test]
async fn msi_to_offnormal_bit_alone_distributes_the_offnormal_transition() {
    let enabled = Fixture::new(EventTransitionBits::TO_OFFNORMAL).await;
    enabled.write_present_value(ALARM_STATE).await;
    assert_sole_notification(
        enabled.drain(),
        enabled.oid,
        EventState::NORMAL,
        EventState::OFFNORMAL,
        "TO_OFFNORMAL set",
    );

    for bits in [
        EventTransitionBits::TO_FAULT,
        EventTransitionBits::TO_NORMAL,
        EventTransitionBits::empty(),
    ] {
        let suppressed = Fixture::new(bits).await;
        suppressed.write_present_value(ALARM_STATE).await;
        assert!(
            suppressed.drain().is_empty(),
            "Event_Enable {bits} must not distribute a TO_OFFNORMAL notification"
        );
        assert_eq!(
            suppressed.event_state().await,
            PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
            "Event_Enable {bits}: the transition must still occur, only distribution is withheld"
        );
    }
}

/// TO_NORMAL alone distributes the return to NORMAL and nothing else, and a
/// TO_OFFNORMAL-only object stays silent on that same return.
///
/// This is the direction that catches a gate keyed to a fixed bit rather than to
/// the transition being reported.
#[tokio::test]
async fn msi_to_normal_bit_alone_distributes_the_return_to_normal() {
    let enabled = Fixture::new(EventTransitionBits::TO_NORMAL).await;
    enabled.write_present_value(ALARM_STATE).await;
    assert!(
        enabled.drain().is_empty(),
        "TO_NORMAL set alone must not distribute the entry into OFFNORMAL"
    );
    assert_eq!(
        enabled.event_state().await,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
        "the object must be in OFFNORMAL for the next write to be a return to normal"
    );

    enabled.write_present_value(NORMAL_STATE).await;
    assert_sole_notification(
        enabled.drain(),
        enabled.oid,
        EventState::OFFNORMAL,
        EventState::NORMAL,
        "TO_NORMAL set",
    );

    let offnormal_only = Fixture::new(EventTransitionBits::TO_OFFNORMAL).await;
    offnormal_only.write_present_value(ALARM_STATE).await;
    assert_eq!(
        offnormal_only.drain().len(),
        1,
        "TO_OFFNORMAL set must distribute the entry into OFFNORMAL"
    );
    offnormal_only.write_present_value(NORMAL_STATE).await;
    assert!(
        offnormal_only.drain().is_empty(),
        "TO_OFFNORMAL set alone must not distribute the return to NORMAL"
    );
    assert_eq!(
        offnormal_only.event_state().await,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "the return to NORMAL must still have happened internally"
    );
}

#[tokio::test]
async fn bi_bv_and_msv_alarm_values_commission_and_reach_the_wire() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    for (property, value) in [
        (
            PropertyIdentifier::ALARM_VALUE,
            PropertyValue::Enumerated(0),
        ),
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
            },
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
        ),
        (
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        ),
    ] {
        bi.write_property(property, None, value, None).unwrap();
    }
    let bi_fixture = Fixture::from_object(Box::new(bi)).await;
    bi_fixture
        .write_present_value_as(PropertyValue::Enumerated(0))
        .await;
    assert_sole_notification(
        bi_fixture.drain(),
        bi_fixture.oid,
        EventState::NORMAL,
        EventState::OFFNORMAL,
        "Binary Input non-default Alarm_Value",
    );

    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    for (property, value) in [
        (
            PropertyIdentifier::ALARM_VALUE,
            PropertyValue::Enumerated(0),
        ),
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
            },
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
        ),
        (
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(1),
        ),
    ] {
        bv.write_property(property, None, value, None).unwrap();
    }
    let bv_fixture = Fixture::from_object(Box::new(bv)).await;
    bv_fixture
        .write_present_value_as(PropertyValue::Enumerated(0))
        .await;
    assert_sole_notification(
        bv_fixture.drain(),
        bv_fixture.oid,
        EventState::NORMAL,
        EventState::OFFNORMAL,
        "Binary Value non-default Alarm_Value",
    );

    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    for (property, value) in [
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
            },
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
        ),
    ] {
        msv.write_property(property, None, value, None).unwrap();
    }
    let msv_fixture = Fixture::from_object(Box::new(msv)).await;
    msv_fixture.add_alarm_value(2).await;
    msv_fixture.write_present_value(2).await;
    assert_sole_notification(
        msv_fixture.drain(),
        msv_fixture.oid,
        EventState::NORMAL,
        EventState::OFFNORMAL,
        "Multi-state Value Alarm_Values",
    );
}

#[tokio::test]
async fn fresh_binary_input_active_is_offnormal_without_distribution() {
    let mut bi = BinaryInputObject::new(2, "BI-default").unwrap();
    bi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let fixture = Fixture::from_object(Box::new(bi)).await;
    fixture
        .write_present_value_as(PropertyValue::Enumerated(1))
        .await;
    assert_eq!(
        fixture.event_state().await,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw())
    );
    assert!(
        fixture.drain().is_empty(),
        "default Event_Enable must suppress distribution"
    );
}

#[tokio::test]
async fn remove_last_alarm_value_disarms_msi() {
    let fixture = Fixture::new(EventTransitionBits::TO_OFFNORMAL).await;
    fixture.remove_alarm_value(ALARM_STATE as u8).await;
    assert_eq!(
        fixture
            .db
            .read()
            .await
            .get(&fixture.oid)
            .unwrap()
            .read_property(PropertyIdentifier::ALARM_VALUES, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
    fixture.write_present_value(ALARM_STATE).await;
    assert_eq!(
        fixture.event_state().await,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert!(
        fixture.drain().is_empty(),
        "an empty Alarm_Values list must not distribute"
    );
}
