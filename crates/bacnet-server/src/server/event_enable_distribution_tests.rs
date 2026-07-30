//! Wire-level proof that a commissioned `Event_Enable` reaches distribution on
//! Multi-state Input (#229).
//!
//! `event_notifications_tests.rs` pins the same gate for AnalogInput and has no
//! headroom left under the 700-LOC cap, so the Multi-state Input case lives
//! here. Multi-state Input is the vehicle because it is the only one of the four
//! types wired by #229 whose `ChangeOfStateDetector` can be given an alarm value
//! through a public API; the other three have no such path until #228 lands.

use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{EventType, ObjectType};
use bytes::Bytes;
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

/// Commission a Multi-state Input with `bits` in `Event_Enable`, put it in an
/// alarm state, drive the per-write notification path once, and return whatever
/// went out on the wire.
///
/// Both `Event_Enable` and the alarm state are established through the same
/// entry points a network client uses, so the fixture cannot pass by way of a
/// path clients cannot reach.
async fn broadcasts_for_msi(bits: EventTransitionBits) -> Vec<Bytes> {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    msi.set_alarm_values(vec![2]);
    msi.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![bits.to_bacnet()],
        },
        None,
    )
    .unwrap();
    msi.set_present_value(2); // an alarm value: NORMAL -> OFFNORMAL
    let oid = msi.object_identifier();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(msi)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let db = Arc::new(RwLock::new(db));

    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let network = Arc::new(NetworkLayer::new(RecordingTransport {
        sent_broadcast: StdArc::clone(&sent),
    }));

    BACnetServer::<RecordingTransport>::fire_event_notifications(
        &db,
        &network,
        &Arc::new(AtomicU8::new(0)), // DCC not blocking
        &Arc::new(Mutex::new(ServerTsm::new())),
        &oid,
        1000,
    )
    .await;

    let out = sent.lock().unwrap().clone();
    out
}

/// `Event_Enable` names one bit per transition, so a fixture that sets all three
/// cannot tell a correct bit from a wrong one — an inverted or off-by-one mask
/// passes it. These assertions set exactly one bit at a time: only the bit that
/// names this transition may put a notification on the wire.
#[tokio::test]
async fn msi_event_enable_to_offnormal_bit_alone_gates_the_wire() {
    let sent = broadcasts_for_msi(EventTransitionBits::TO_OFFNORMAL).await;
    assert_eq!(
        sent.len(),
        1,
        "TO_OFFNORMAL set must distribute the CHANGE_OF_STATE notification"
    );

    let npdu = decode_npdu(sent[0].clone()).expect("decode NPDU");
    match decode_apdu(npdu.payload).expect("decode APDU") {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(
                req.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
            let notif = EventNotificationRequest::decode(&req.service_request)
                .expect("decode EventNotification");
            assert_eq!(
                notif.event_type,
                EventType::CHANGE_OF_STATE.to_raw(),
                "the detector's CHANGE_OF_STATE algorithm must reach the wire"
            );
            assert_eq!(
                notif.event_object_identifier,
                ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap()
            );
        }
        other => panic!("expected UnconfirmedRequest, got {other:?}"),
    }

    for suppressed in [
        EventTransitionBits::TO_FAULT,
        EventTransitionBits::TO_NORMAL,
        EventTransitionBits::empty(),
    ] {
        assert!(
            broadcasts_for_msi(suppressed).await.is_empty(),
            "Event_Enable {suppressed} must not distribute a TO_OFFNORMAL notification"
        );
    }
}
