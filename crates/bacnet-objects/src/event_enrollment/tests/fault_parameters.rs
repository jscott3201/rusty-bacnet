//! `Fault_Parameters` encode/decode tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_types::enums::FaultType;

/// Decode the read arm's framed wire form back to a structured value.
fn decode_framed(val: PropertyValue) -> FaultParameters {
    let PropertyValue::ApplicationData(bytes) = val else {
        panic!("expected framed ApplicationData, got {val:?}");
    };
    bacnet_encoding::constructed::decode_fault_parameters(&bytes, 0)
        .unwrap()
        .0
}

fn read_fault_type(ee: &EventEnrollmentObject) -> u32 {
    let PropertyValue::Enumerated(value) = ee
        .read_property(PropertyIdentifier::FAULT_TYPE, None)
        .unwrap()
    else {
        panic!("Fault_Type must be Enumerated");
    };
    value
}

// FaultParameters tests
// -----------------------------------------------------------------------

#[test]
fn fault_parameters_default_to_none_choice() {
    let ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), FaultParameters::FaultNone);
    assert_eq!(read_fault_type(&ee), FaultType::NONE.to_raw());
}

#[test]
fn fault_type_tracks_each_fault_parameters_alternative() {
    use bacnet_types::constructed::BACnetPropertyStates;

    let reference = BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    };
    let cases = vec![
        (FaultParameters::FaultNone, FaultType::NONE),
        (
            FaultParameters::FaultCharacterString {
                fault_values: vec!["fault".to_string()],
            },
            FaultType::FAULT_CHARACTERSTRING,
        ),
        (
            FaultParameters::FaultExtended {
                vendor_id: 1,
                extended_fault_type: 2,
                parameters: vec![],
            },
            FaultType::FAULT_EXTENDED,
        ),
        (
            FaultParameters::FaultLifeSafety {
                fault_values: vec![1],
                mode_for_reference: reference.clone(),
            },
            FaultType::FAULT_LIFE_SAFETY,
        ),
        (
            FaultParameters::FaultState {
                fault_values: vec![BACnetPropertyStates::BooleanValue(true)],
            },
            FaultType::FAULT_STATE,
        ),
        (
            FaultParameters::FaultStatusFlags {
                reference: reference.clone(),
            },
            FaultType::FAULT_STATUS_FLAGS,
        ),
        (
            FaultParameters::FaultOutOfRange {
                min_normal: 0.0,
                max_normal: 1.0,
            },
            FaultType::FAULT_OUT_OF_RANGE,
        ),
        (
            FaultParameters::FaultListed { reference },
            FaultType::FAULT_LISTED,
        ),
    ];
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();

    for (parameters, expected) in cases {
        ee.set_fault_parameters(Some(parameters));
        assert_eq!(read_fault_type(&ee), expected.to_raw());
    }
}

#[test]
fn fault_parameters_none_variant() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultNone;
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_character_string() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultCharacterString {
        fault_values: vec!["alarm".to_string(), "critical".to_string()],
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_extended() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        parameters: vec![0x01, 0x02],
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_life_safety() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let fp = FaultParameters::FaultLifeSafety {
        fault_values: vec![1, 2, 3],
        mode_for_reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_state() {
    use bacnet_types::constructed::BACnetPropertyStates;
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::BooleanValue(true)],
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_status_flags() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let fp = FaultParameters::FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::STATUS_FLAGS.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_out_of_range() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_listed() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let fp = FaultParameters::FaultListed {
        reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    };
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_properties_are_advertised_together() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let props = ee.property_list();
    assert!(props.contains(&PropertyIdentifier::FAULT_TYPE));
    assert!(props.contains(&PropertyIdentifier::FAULT_PARAMETERS));
    assert!(!ee.is_writable_property(PropertyIdentifier::FAULT_TYPE));
    assert!(ee
        .write_property(
            PropertyIdentifier::FAULT_TYPE,
            None,
            PropertyValue::Enumerated(FaultType::FAULT_LISTED.to_raw()),
            None,
        )
        .is_err());
}

#[test]
fn fault_parameters_write_round_trip() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 55.0,
    };
    ee.write_property(
        PropertyIdentifier::FAULT_PARAMETERS,
        None,
        fp.encode_property_value(),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_framed_write_round_trip() {
    // Framed wire form write: exactly what a conformant peer sends, and the
    // read arm's bytes come back byte-identical.
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 55.0,
    };
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_fault_parameters(&mut buf, &fp).unwrap();
    ee.write_property(
        PropertyIdentifier::FAULT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(buf.to_vec()),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    let PropertyValue::ApplicationData(bytes) = &val else {
        panic!("expected ApplicationData");
    };
    assert_eq!(bytes.as_slice(), buf.as_ref());
    assert_eq!(decode_framed(val), fp);
}

#[test]
fn fault_parameters_framed_trailing_garbage_rejected() {
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 55.0,
    };
    let mut good = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_fault_parameters(&mut good, &fp).unwrap();
    for extra in 1..=4usize {
        let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
        let mut bytes = good.to_vec();
        bytes.extend_from_slice(&vec![0x55; extra]);
        let result = ee.write_property(
            PropertyIdentifier::FAULT_PARAMETERS,
            None,
            PropertyValue::ApplicationData(bytes),
            None,
        );
        assert!(result.is_err(), "trailing {extra} byte(s) must be rejected");
        // Untouched: the effective choice remains FaultNone.
        let val = ee
            .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
            .unwrap();
        assert_eq!(decode_framed(val), FaultParameters::FaultNone);
    }
}

#[test]
fn fault_parameters_framed_malformed_rejected() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    // opening [6] with no closing.
    let result = ee.write_property(
        PropertyIdentifier::FAULT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(vec![0x6E, 0x0E]),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn fault_parameters_write_clear_to_null() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultNone));
    ee.write_property(
        PropertyIdentifier::FAULT_PARAMETERS,
        None,
        PropertyValue::Null,
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), FaultParameters::FaultNone);
    assert_eq!(read_fault_type(&ee), FaultType::NONE.to_raw());
}

#[test]
fn fault_parameters_clear() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultNone));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), FaultParameters::FaultNone);

    // Clear back to the effective NONE alternative.
    ee.set_fault_parameters(None);
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed(val), FaultParameters::FaultNone);
    assert_eq!(read_fault_type(&ee), FaultType::NONE.to_raw());
}

// -----------------------------------------------------------------------
