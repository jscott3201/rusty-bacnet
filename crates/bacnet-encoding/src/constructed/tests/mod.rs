//! Tests for the full ASN.1 framing codecs (Clause 20.2.1.5 + Clause 21).

use super::*;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetExtendedPropertyState,
    BACnetPropertyStates, BACnetProprietaryPropertyState,
};
use bacnet_types::enums::ObjectType;

mod cov_subscription;
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

fn modeled_property_states() -> Vec<(u8, BACnetPropertyStates)> {
    use BACnetPropertyStates as S;

    vec![
        (0, S::BooleanValue(true)),
        (1, S::BinaryValue(1)),
        (2, S::EventType(1)),
        (3, S::Polarity(1)),
        (4, S::ProgramChange(1)),
        (5, S::ProgramState(1)),
        (6, S::ReasonForHalt(1)),
        (7, S::Reliability(1)),
        (8, S::State(1)),
        (9, S::SystemStatus(1)),
        (10, S::Units(1)),
        (11, S::UnsignedValue(1)),
        (12, S::LifeSafetyMode(1)),
        (13, S::LifeSafetyState(1)),
        (14, S::RestartReason(1)),
        (15, S::DoorAlarmState(1)),
        (16, S::Action(1)),
        (17, S::DoorSecuredStatus(1)),
        (18, S::DoorStatus(1)),
        (19, S::DoorValue(1)),
        (20, S::FileAccessMethod(1)),
        (21, S::LockStatus(1)),
        (22, S::LifeSafetyOperation(1)),
        (23, S::Maintenance(1)),
        (24, S::NodeType(1)),
        (25, S::NotifyType(1)),
        (27, S::ShedState(1)),
        (28, S::SilencedState(1)),
        (30, S::AccessEvent(1)),
        (31, S::ZoneOccupancyState(1)),
        (32, S::AccessCredentialDisableReason(1)),
        (33, S::AccessCredentialDisable(1)),
        (34, S::AuthenticationStatus(1)),
        (36, S::BackupState(1)),
        (37, S::WriteStatus(1)),
        (38, S::LightingInProgress(1)),
        (39, S::LightingOperation(1)),
        (40, S::LightingTransition(1)),
        (41, S::IntegerValue(-1)),
        (42, S::BinaryLightingValue(1)),
        (43, S::TimerState(1)),
        (44, S::TimerTransition(1)),
        (45, S::BacnetIpMode(1)),
        (46, S::NetworkPortCommand(1)),
        (47, S::NetworkType(1)),
        (48, S::NetworkNumberQuality(1)),
        (49, S::EscalatorOperationDirection(1)),
        (50, S::EscalatorFault(1)),
        (51, S::EscalatorMode(1)),
        (52, S::LiftCarDirection(1)),
        (53, S::LiftCarDoorCommand(1)),
        (54, S::LiftCarDriveStatus(1)),
        (55, S::LiftCarMode(1)),
        (56, S::LiftGroupMode(1)),
        (57, S::LiftFault(1)),
        (58, S::ProtocolLevel(1)),
        (59, S::AuditLevel(1)),
        (60, S::AuditOperation(1)),
        (
            63,
            S::ExtendedValue(BACnetExtendedPropertyState::new(255, 1).unwrap()),
        ),
    ]
}

fn raw_property_state(tag: u8, content: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_tag(&mut buf, tag, TagClass::Context, content.len() as u32);
    buf.extend_from_slice(content);
    buf
}

#[test]
fn property_state_clause_21_tag_table_and_round_trips() {
    for (tag, state) in modeled_property_states() {
        let mut encoded = BytesMut::new();
        encode_property_state(&mut encoded, &state).unwrap();
        let expected = if let BACnetPropertyStates::ExtendedValue(value) = state {
            let mut expected = vec![0xfc, 63];
            expected.extend_from_slice(&value.encoded().to_be_bytes());
            expected
        } else {
            let content = if matches!(state, BACnetPropertyStates::IntegerValue(_)) {
                0xff
            } else {
                1
            };
            if tag <= 14 {
                vec![(tag << 4) | 0x09, content]
            } else {
                vec![0xf9, tag, content]
            }
        };
        assert_eq!(encoded.as_ref(), expected, "context tag {tag}");

        let (decoded, end) = decode_property_state(&encoded, 0).unwrap();
        assert_eq!(decoded, state, "context tag {tag}");
        assert_eq!(end, encoded.len());
    }
}

#[test]
fn property_state_unsigned_alternatives_enforce_u32() {
    for (tag, state) in modeled_property_states() {
        if matches!(
            state,
            BACnetPropertyStates::BooleanValue(_) | BACnetPropertyStates::IntegerValue(_)
        ) {
            continue;
        }
        assert!(decode_property_state(&raw_property_state(tag, &[1, 0, 0, 0, 0]), 0).is_err());
        assert!(decode_property_state(&raw_property_state(tag, &[0xff; 8]), 0).is_err());

        let (decoded, _) =
            decode_property_state(&raw_property_state(tag, &[0, 0xff, 0xff, 0xff, 0xff]), 0)
                .unwrap();
        let expected = if tag == 63 {
            Some(u32::MAX % 100_000)
        } else {
            Some(u32::MAX)
        };
        assert_eq!(decoded.as_u32(), expected, "context tag {tag}");
    }
}

#[test]
fn property_state_validates_boolean_integer_and_tag_forms() {
    assert_eq!(
        decode_property_state(&raw_property_state(0, &[0]), 0)
            .unwrap()
            .0,
        BACnetPropertyStates::BooleanValue(false)
    );
    for content in [&[][..], &[2], &[0, 0]] {
        assert!(decode_property_state(&raw_property_state(0, content), 0).is_err());
    }

    for value in [i32::MIN, -1, 0, i32::MAX] {
        let mut encoded = BytesMut::new();
        encode_property_state(&mut encoded, &BACnetPropertyStates::IntegerValue(value)).unwrap();
        assert_eq!(
            decode_property_state(&encoded, 0).unwrap().0,
            BACnetPropertyStates::IntegerValue(value)
        );
    }
    assert!(decode_property_state(&raw_property_state(41, &[0; 5]), 0).is_err());
    for content in [&[0x00, 0x01][..], &[0xFF, 0x80]] {
        assert!(decode_property_state(&raw_property_state(41, content), 0).is_err());
    }

    let mut application = BytesMut::new();
    primitives::encode_app_unsigned(&mut application, 1);
    assert!(decode_property_state(&application, 0).is_err());
    let mut opening = BytesMut::new();
    tags::encode_opening_tag(&mut opening, 0);
    assert!(decode_property_state(&opening, 0).is_err());
    let mut closing = BytesMut::new();
    tags::encode_closing_tag(&mut closing, 0);
    assert!(decode_property_state(&closing, 0).is_err());

    assert!(decode_property_state(&[0x1a], 0).is_err());
    assert!(decode_property_state(&[0xf9], 0).is_err());
    assert!(decode_property_state(&[0xf9, 0xff, 0], 0).is_err());

    assert_eq!(BACnetPropertyStates::BooleanValue(true).as_u32(), Some(1));
    assert_eq!(BACnetPropertyStates::IntegerValue(7).as_u32(), Some(7));
    assert_eq!(BACnetPropertyStates::IntegerValue(-1).as_u32(), None);
}

#[test]
fn property_state_rejects_reserved_tags_and_preserves_proprietary_tags() {
    for tag in [26, 29, 35, 61, 62] {
        assert!(decode_property_state(&raw_property_state(tag, &[0]), 0).is_err());
    }

    for tag in [64, 254] {
        let expected = BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::primitive(tag, vec![0xde, 0xad]).unwrap(),
        );
        let (decoded, _) =
            decode_property_state(&raw_property_state(tag, &[0xde, 0xad]), 0).unwrap();
        assert_eq!(decoded, expected);
        let mut encoded = BytesMut::new();
        encode_property_state(&mut encoded, &expected).unwrap();
        assert_eq!(decode_property_state(&encoded, 0).unwrap().0, expected);
    }

    let expected = BACnetPropertyStates::Other(
        BACnetProprietaryPropertyState::constructed(64, vec![0x21, 0x07]).unwrap(),
    );
    let mut encoded = BytesMut::new();
    encode_property_state(&mut encoded, &expected).unwrap();
    assert_eq!(encoded.as_ref(), &[0xfe, 64, 0x21, 0x07, 0xff, 64]);
    assert_eq!(decode_property_state(&encoded, 0).unwrap().0, expected);

    for tag in [0, 26, 63, 255] {
        assert!(BACnetProprietaryPropertyState::primitive(tag, vec![]).is_err());
        assert!(BACnetProprietaryPropertyState::constructed(tag, vec![]).is_err());
    }

    assert!(BACnetExtendedPropertyState::new(254, 0).is_err());
    assert!(BACnetExtendedPropertyState::new(255, 100_000).is_err());
    assert!(BACnetExtendedPropertyState::new(u32::MAX, 0).is_err());
    assert!(decode_property_state(&raw_property_state(63, &[1]), 0).is_err());

    let extended = BACnetExtendedPropertyState::new(256, 7).unwrap();
    assert_eq!(extended.encoded(), 25_600_007);
    assert_eq!(
        BACnetExtendedPropertyState::from_encoded(extended.encoded()).unwrap(),
        extended
    );

    let malformed = BACnetPropertyStates::Other(
        BACnetProprietaryPropertyState::constructed(64, vec![0xde]).unwrap(),
    );
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(encode_property_state(&mut untouched, &malformed).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);

    let mismatched = BACnetPropertyStates::Other(
        BACnetProprietaryPropertyState::constructed(64, vec![0x1e, 0x2f]).unwrap(),
    );
    assert!(encode_property_state(&mut BytesMut::new(), &mismatched).is_err());
    assert!(decode_property_state(&[0xfe, 64, 0x1e, 0x2f, 0xff, 64], 0).is_err());

    for body in [
        vec![0x01, 0x00],
        vec![0x12],
        vec![0x0e, 0x01, 0x00, 0x0f],
        vec![0x1e, 0x12, 0x1f],
    ] {
        let malformed = BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(64, body.clone()).unwrap(),
        );
        let mut untouched = BytesMut::from(&[0xaa][..]);
        assert!(encode_property_state(&mut untouched, &malformed).is_err());
        assert_eq!(untouched.as_ref(), &[0xaa]);

        let mut framed = vec![0xfe, 64];
        framed.extend_from_slice(&body);
        framed.extend_from_slice(&[0xff, 64]);
        assert!(decode_property_state(&framed, 0).is_err());
    }
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
    let mut encoded = BytesMut::new();
    encode_event_parameter(&mut encoded, &decoded).unwrap();
    assert_eq!(encoded.as_ref(), data);
}
