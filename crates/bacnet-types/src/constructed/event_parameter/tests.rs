//! Round-trip tests for [`BACnetEventParameter`] and [`FaultParameters`]
//! encode/decode.

use super::*;
use crate::constructed::FaultParameters;
use crate::primitives::ObjectIdentifier;

/// Build a local BACnetDeviceObjectPropertyReference for tests.
fn dopr(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(crate::enums::ObjectType::ANALOG_INPUT, instance).unwrap(),
        crate::enums::PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

#[test]
fn out_of_range_round_trip() {
    let p = BACnetEventParameter::OutOfRange {
        time_delay: 7,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 2.0,
    };
    assert_eq!(p.tag(), event_parameter_tag::OUT_OF_RANGE);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn floating_limit_round_trip() {
    let p = BACnetEventParameter::FloatingLimit {
        time_delay: 3,
        setpoint_reference: dopr(5),
        low_diff_limit: 1.0,
        high_diff_limit: 2.0,
        deadband: 0.5,
    };
    assert_eq!(p.tag(), event_parameter_tag::FLOATING_LIMIT);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn change_of_state_round_trip() {
    let p = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
    };
    assert_eq!(p.tag(), event_parameter_tag::CHANGE_OF_STATE);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn change_of_state_legacy_form_preserves_corrected_property_states() {
    let states = vec![
        BACnetPropertyStates::RestartReason(1),
        BACnetPropertyStates::DoorAlarmState(2),
        BACnetPropertyStates::Action(3),
        BACnetPropertyStates::DoorSecuredStatus(4),
        BACnetPropertyStates::DoorStatus(5),
        BACnetPropertyStates::DoorValue(6),
        BACnetPropertyStates::LightingInProgress(7),
        BACnetPropertyStates::LightingOperation(8),
        BACnetPropertyStates::LightingTransition(9),
        BACnetPropertyStates::IntegerValue(-10),
        BACnetPropertyStates::BinaryLightingValue(11),
        BACnetPropertyStates::TimerState(12),
        BACnetPropertyStates::TimerTransition(13),
        BACnetPropertyStates::LiftCarDirection(14),
        BACnetPropertyStates::LiftCarDoorCommand(15),
        BACnetPropertyStates::AuditOperation(16),
        BACnetPropertyStates::ExtendedValue(
            BACnetExtendedPropertyState::from_encoded(25_500_008).unwrap(),
        ),
        BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::primitive(64, vec![0xde, 0xad]).unwrap(),
        ),
        BACnetPropertyStates::Other(
            BACnetProprietaryPropertyState::constructed(65, vec![0x21, 0x07]).unwrap(),
        ),
    ];
    let parameters = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: states,
    };
    assert_eq!(
        BACnetEventParameter::decode(&parameters.encode()).unwrap(),
        parameters
    );
}

#[test]
fn change_of_state_accepts_every_base_flat_property_state() {
    let base_states = [
        (0, BACnetPropertyStates::BooleanValue(true)),
        (1, BACnetPropertyStates::BinaryValue(7)),
        (2, BACnetPropertyStates::EventType(7)),
        (3, BACnetPropertyStates::Polarity(7)),
        (4, BACnetPropertyStates::ProgramChange(7)),
        (5, BACnetPropertyStates::ProgramState(7)),
        (6, BACnetPropertyStates::ReasonForHalt(7)),
        (7, BACnetPropertyStates::Reliability(7)),
        (8, BACnetPropertyStates::State(7)),
        (9, BACnetPropertyStates::SystemStatus(7)),
        (10, BACnetPropertyStates::Units(7)),
        (11, BACnetPropertyStates::UnsignedValue(7)),
        (12, BACnetPropertyStates::LifeSafetyMode(7)),
        (13, BACnetPropertyStates::LifeSafetyState(7)),
        (14, BACnetPropertyStates::DoorAlarmState(7)),
        (15, BACnetPropertyStates::Action(7)),
        (16, BACnetPropertyStates::DoorSecuredStatus(7)),
        (17, BACnetPropertyStates::DoorStatus(7)),
        (18, BACnetPropertyStates::DoorValue(7)),
        (38, BACnetPropertyStates::TimerState(7)),
        (39, BACnetPropertyStates::TimerTransition(7)),
        (40, BACnetPropertyStates::LiftCarDirection(7)),
        (42, BACnetPropertyStates::LiftCarDoorCommand(7)),
    ];

    for (tag, expected) in base_states {
        let data = if tag == 0 {
            vec![1]
        } else {
            7u32.to_le_bytes().to_vec()
        };
        let flat = PropertyValue::List(vec![
            PropertyValue::Unsigned(tag),
            PropertyValue::OctetString(data),
        ]);
        assert_eq!(property_state_from_pv(&flat).unwrap(), expected);
    }
}

#[test]
fn change_of_state_accepts_base_flat_integer_state() {
    let legacy = PropertyValue::List(vec![
        PropertyValue::Unsigned(event_parameter_tag::CHANGE_OF_STATE as u64),
        PropertyValue::Unsigned(0),
        PropertyValue::List(vec![PropertyValue::List(vec![
            PropertyValue::Unsigned(41),
            PropertyValue::OctetString(vec![0xFD]),
        ])]),
    ]);
    let expected = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::IntegerValue(-3)],
    };

    assert_eq!(BACnetEventParameter::decode(&legacy).unwrap(), expected);
    let PropertyValue::List(encoded) = expected.encode() else {
        unreachable!();
    };
    let PropertyValue::List(states) = &encoded[2] else {
        unreachable!();
    };
    let PropertyValue::List(integer) = &states[0] else {
        unreachable!();
    };
    assert_eq!(integer.last(), Some(&PropertyValue::Boolean(false)));
}

#[test]
fn change_of_bitstring_round_trip() {
    let p = BACnetEventParameter::ChangeOfBitstring {
        time_delay: 4,
        bitmask: (0, vec![0xFF]),
        list_of_values: vec![(0, vec![0xE0])],
    };
    assert_eq!(p.tag(), event_parameter_tag::CHANGE_OF_BITSTRING);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn change_of_value_increment_round_trip() {
    let p = BACnetEventParameter::ChangeOfValue {
        time_delay: 2,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    };
    assert_eq!(p.tag(), event_parameter_tag::CHANGE_OF_VALUE);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn change_of_value_bitmask_round_trip() {
    let p = BACnetEventParameter::ChangeOfValue {
        time_delay: 2,
        criteria: ChangeOfValueCriteria::Bitmask {
            unused_bits: 5,
            data: vec![0x80],
        },
    };
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn extended_round_trip() {
    let p = BACnetEventParameter::Extended {
        vendor_id: 42,
        extended_event_type: 99,
        parameters: vec![0x21, 0x07],
    };
    assert_eq!(p.tag(), event_parameter_tag::EXTENDED);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn opaque_unknown_tag_preserved() {
    // An unknown algorithm tag round-trips through the Opaque catch-all.
    let p = BACnetEventParameter::Opaque {
        tag: 0x6F,
        data: vec![0x21, 0x03],
    };
    assert_eq!(p.tag(), 0x6F);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn decode_rejects_non_list() {
    assert!(BACnetEventParameter::decode(&PropertyValue::Null).is_err());
}

#[test]
fn decode_rejects_empty_list() {
    assert!(BACnetEventParameter::decode(&PropertyValue::List(Vec::new())).is_err());
}

#[test]
fn decode_rejects_non_unsigned_tag() {
    assert!(
        BACnetEventParameter::decode(&PropertyValue::List(vec![PropertyValue::Boolean(true)]))
            .is_err()
    );
}

#[test]
fn decode_rejects_truncated_out_of_range() {
    // tag + time_delay only — missing the three REAL limits.
    assert!(BACnetEventParameter::decode(&PropertyValue::List(vec![
        PropertyValue::Unsigned(event_parameter_tag::OUT_OF_RANGE as u64),
        PropertyValue::Unsigned(0),
    ]))
    .is_err());
}

#[test]
fn decode_rejects_out_of_range_tags_and_trailing_values() {
    let valid = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::BinaryValue(1)],
    }
    .encode();
    let PropertyValue::List(mut items) = valid else {
        unreachable!();
    };
    items[0] = PropertyValue::Unsigned(257);
    assert!(BACnetEventParameter::decode(&PropertyValue::List(items)).is_err());

    let PropertyValue::List(mut items) = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::BinaryValue(1)],
    }
    .encode() else {
        unreachable!();
    };
    items.push(PropertyValue::Boolean(true));
    assert!(BACnetEventParameter::decode(&PropertyValue::List(items)).is_err());
}

#[test]
fn flat_references_reject_overflow_and_extra_members() {
    let PropertyValue::List(items) = (BACnetEventParameter::FloatingLimit {
        time_delay: 1,
        setpoint_reference: dopr(1),
        low_diff_limit: 1.0,
        high_diff_limit: 2.0,
        deadband: 0.5,
    })
    .encode() else {
        unreachable!();
    };

    for field in [1, 2] {
        let mut malformed = items.clone();
        let PropertyValue::List(reference) = &mut malformed[2] else {
            unreachable!();
        };
        reference[field] = PropertyValue::Unsigned(u64::MAX);
        assert!(BACnetEventParameter::decode(&PropertyValue::List(malformed)).is_err());
    }

    let mut malformed = items;
    let PropertyValue::List(reference) = &mut malformed[2] else {
        unreachable!();
    };
    reference.push(PropertyValue::Boolean(true));
    assert!(BACnetEventParameter::decode(&PropertyValue::List(malformed)).is_err());
}

#[test]
fn flat_fault_life_safety_values_reject_u32_overflow() {
    let PropertyValue::List(mut items) = (FaultParameters::FaultLifeSafety {
        fault_values: vec![1],
        mode_for_reference: dopr(1),
    })
    .encode_property_value() else {
        unreachable!();
    };
    let PropertyValue::List(values) = &mut items[1] else {
        unreachable!();
    };
    values[0] = PropertyValue::Unsigned(u64::from(u32::MAX) + 1);
    assert!(FaultParameters::decode_property_value(&PropertyValue::List(items)).is_err());
}

#[test]
fn fault_parameters_round_trip() {
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: 10.0,
        max_normal: 20.0,
    };
    assert_eq!(
        FaultParameters::decode_property_value(&fp.encode_property_value()).unwrap(),
        fp
    );
}

#[test]
fn fault_parameters_none_round_trip() {
    let fp = FaultParameters::FaultNone;
    assert_eq!(
        FaultParameters::decode_property_value(&fp.encode_property_value()).unwrap(),
        fp
    );
}

#[test]
fn fault_parameters_reject_out_of_range_legacy_tags() {
    for tag in [8, 255, 256, u64::MAX] {
        assert!(
            FaultParameters::decode_property_value(&PropertyValue::List(vec![
                PropertyValue::Unsigned(tag),
            ]))
            .is_err()
        );
    }
}

#[test]
fn fault_parameters_legacy_alternatives_require_exact_members() {
    let reference = dopr(1);
    let alternatives = vec![
        FaultParameters::FaultNone,
        FaultParameters::FaultCharacterString {
            fault_values: vec!["fault".to_string()],
        },
        FaultParameters::FaultExtended {
            vendor_id: 1,
            extended_fault_type: 2,
            parameters: vec![0x21, 0x03],
        },
        FaultParameters::FaultLifeSafety {
            fault_values: vec![1],
            mode_for_reference: reference.clone(),
        },
        FaultParameters::FaultState {
            fault_values: vec![BACnetPropertyStates::BooleanValue(true)],
        },
        FaultParameters::FaultStatusFlags {
            reference: reference.clone(),
        },
        FaultParameters::FaultOutOfRange {
            min_normal: 0.0,
            max_normal: 1.0,
        },
        FaultParameters::FaultListed { reference },
    ];

    for parameters in alternatives {
        let encoded = parameters.encode_property_value();
        assert_eq!(
            FaultParameters::decode_property_value(&encoded).unwrap(),
            parameters
        );

        let PropertyValue::List(mut items) = encoded else {
            unreachable!();
        };
        items.push(PropertyValue::Boolean(true));
        assert!(FaultParameters::decode_property_value(&PropertyValue::List(items)).is_err());
    }
}
