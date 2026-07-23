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
        parameters: vec![0xDE, 0xAD],
    };
    assert_eq!(p.tag(), event_parameter_tag::EXTENDED);
    assert_eq!(BACnetEventParameter::decode(&p.encode()).unwrap(), p);
}

#[test]
fn opaque_unknown_tag_preserved() {
    // An unknown algorithm tag round-trips through the Opaque catch-all.
    let p = BACnetEventParameter::Opaque {
        tag: 0x6F,
        data: vec![1, 2, 3],
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
