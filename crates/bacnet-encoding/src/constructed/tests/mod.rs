//! Tests for the full ASN.1 framing codecs (Clause 20.2.1.5 + Clause 21).

use super::*;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};
use bacnet_types::enums::ObjectType;

mod event_parameter;
mod fault_parameter;
mod recipient;

/// A local BACnetDeviceObjectPropertyReference for tests.
pub(crate) fn dopr_ai(instance: u32, property: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property_identifier: property,
        property_array_index: None,
        device_identifier: None,
    }
}

// --- BACnetPropertyStates golden vectors + round-trips ----------------------

#[test]
fn property_state_golden_vectors() {
    // boolean-value [0] BOOLEAN — context-tagged boolean: 1 contents octet.
    let mut buf = BytesMut::new();
    encode_property_state(&mut buf, &BACnetPropertyStates::BooleanValue(true));
    assert_eq!(buf.as_ref(), &[0x09, 0x01]);

    // unsigned-value [11] Unsigned — (11<<4)|8|1 = 0xB9.
    buf.clear();
    encode_property_state(&mut buf, &BACnetPropertyStates::UnsignedValue(42));
    assert_eq!(buf.as_ref(), &[0xB9, 0x2A]);

    // binary-value [1] — enumerated contents under ctx tag 1 = 0x19.
    buf.clear();
    encode_property_state(&mut buf, &BACnetPropertyStates::BinaryValue(1));
    assert_eq!(buf.as_ref(), &[0x19, 0x01]);

    // door-alarm-state [15] per 135-2020 (restart-reason took [14]):
    // extended tag — 0xF9, 15.
    buf.clear();
    encode_property_state(&mut buf, &BACnetPropertyStates::DoorAlarmState(2));
    assert_eq!(buf.as_ref(), &[0xF9, 0x0F, 0x02]);

    // timer-state [43] / lift-car-direction [52] per 135-2020.
    buf.clear();
    encode_property_state(&mut buf, &BACnetPropertyStates::TimerState(1));
    assert_eq!(buf.as_ref(), &[0xF9, 0x2B, 0x01]);
    buf.clear();
    encode_property_state(&mut buf, &BACnetPropertyStates::LiftCarDirection(3));
    assert_eq!(buf.as_ref(), &[0xF9, 0x34, 0x03]);
}

#[test]
fn property_state_round_trip_all_modeled_variants() {
    let variants = [
        BACnetPropertyStates::BooleanValue(true),
        BACnetPropertyStates::BinaryValue(1),
        BACnetPropertyStates::EventType(2),
        BACnetPropertyStates::Polarity(3),
        BACnetPropertyStates::ProgramChange(4),
        BACnetPropertyStates::ProgramState(5),
        BACnetPropertyStates::ReasonForHalt(6),
        BACnetPropertyStates::Reliability(7),
        BACnetPropertyStates::State(8),
        BACnetPropertyStates::SystemStatus(9),
        BACnetPropertyStates::Units(10),
        BACnetPropertyStates::UnsignedValue(11),
        BACnetPropertyStates::LifeSafetyMode(12),
        BACnetPropertyStates::LifeSafetyState(13),
        BACnetPropertyStates::DoorAlarmState(14),
        BACnetPropertyStates::Action(15),
        BACnetPropertyStates::DoorSecuredStatus(16),
        BACnetPropertyStates::DoorStatus(17),
        BACnetPropertyStates::DoorValue(18),
        BACnetPropertyStates::TimerState(19),
        BACnetPropertyStates::TimerTransition(20),
        BACnetPropertyStates::LiftCarDirection(21),
        BACnetPropertyStates::LiftCarDoorCommand(22),
    ];
    for state in &variants {
        let mut buf = BytesMut::new();
        encode_property_state(&mut buf, state);
        let (decoded, end) = decode_property_state(&buf, 0).unwrap();
        assert_eq!(&decoded, state);
        assert_eq!(end, buf.len());
    }
}

#[test]
fn property_state_unmodeled_tag_preserved_as_other() {
    // integer-value [41] INTEGER — no Rust variant; raw contents preserved.
    let mut buf = BytesMut::new();
    primitives::encode_ctx_signed(&mut buf, 41, -3);
    let (decoded, _) = decode_property_state(&buf, 0).unwrap();
    assert_eq!(
        decoded,
        BACnetPropertyStates::Other {
            tag: 41,
            data: vec![0xFD]
        }
    );
    // Other re-encodes under its recorded tag.
    let mut buf2 = BytesMut::new();
    encode_property_state(&mut buf2, &decoded);
    let (decoded2, _) = decode_property_state(&buf2, 0).unwrap();
    assert_eq!(decoded2, decoded);
}

#[test]
fn property_state_rejects_application_and_constructed_tags() {
    // Application-tagged content is not a property-state element.
    let mut buf = BytesMut::new();
    primitives::encode_app_unsigned(&mut buf, 1);
    assert!(decode_property_state(&buf, 0).is_err());
    // Nor is an opening tag (a constructed element).
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 0);
    assert!(decode_property_state(&buf, 0).is_err());
}

// --- DOPR body codec --------------------------------------------------------

#[test]
fn dopr_body_round_trip_local_and_full() {
    let local = dopr_ai(5, 85);
    let mut buf = BytesMut::new();
    encode_dopr_body(&mut buf, &local);
    let (decoded, end) = decode_dopr_body(&buf, 0, "test").unwrap();
    assert_eq!(decoded, local);
    assert_eq!(end, buf.len());

    let full = BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
        property_identifier: 111,
        property_array_index: Some(3),
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 8).unwrap()),
    };
    let mut buf = BytesMut::new();
    encode_dopr_body(&mut buf, &full);
    let (decoded, _) = decode_dopr_body(&buf, 0, "test").unwrap();
    assert_eq!(decoded, full);
}

#[test]
fn event_parameter_opaque_primitive_form_decodes() {
    // none [20] NULL — primitive context tag, zero contents (extended tag).
    let data = [0xF8, 20];
    let (decoded, end) = decode_event_parameter(&data, 0).unwrap();
    assert_eq!(
        decoded,
        BACnetEventParameter::Opaque {
            tag: 20,
            data: Vec::new()
        }
    );
    assert_eq!(end, data.len());
}
