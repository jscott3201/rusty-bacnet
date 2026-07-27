//! `Fault_Parameters` encode/decode tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;

// FaultParameters tests
// -----------------------------------------------------------------------

#[test]
fn fault_parameters_default_none() {
    let ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
}

#[test]
fn fault_parameters_none_variant() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let fp = FaultParameters::FaultNone;
    ee.set_fault_parameters(Some(fp.clone()));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
}

#[test]
fn fault_parameters_in_property_list() {
    let ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let props = ee.property_list();
    assert!(props.contains(&PropertyIdentifier::FAULT_PARAMETERS));
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
    assert_eq!(FaultParameters::decode_property_value(&val).unwrap(), fp);
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
    assert_eq!(val, PropertyValue::Null);
}

#[test]
fn fault_parameters_clear() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultNone));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(
        FaultParameters::decode_property_value(&val).unwrap(),
        FaultParameters::FaultNone
    );

    // Clear back to None
    ee.set_fault_parameters(None);
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
}

// -----------------------------------------------------------------------
