use super::*;

#[test]
fn create_event_enrollment() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert_eq!(
        ee.object_identifier().object_type(),
        ObjectType::EVENT_ENROLLMENT
    );
    assert_eq!(ee.object_identifier().instance_number(), 1);
    assert_eq!(ee.object_name(), "EE-1");
}

#[test]
fn read_object_type() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::EVENT_ENROLLMENT.to_raw())
    );
}

#[test]
fn read_event_type() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 3).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(3));
}

#[test]
fn read_event_enable() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    // Default event_enable = 0b111, shifted left 5 = 0b1110_0000
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn read_notification_class() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn write_notify_type() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
}

#[test]
fn write_notify_type_wrong_type() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let result = ee.write_property(
        PropertyIdentifier::NOTIFY_TYPE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn read_acked_transitions() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap();
    // Default acked_transitions = 0b111, shifted left 5 = 0b1110_0000
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn read_object_property_reference_none() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
}

#[test]
fn read_object_property_reference_some() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: ai_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }));
    let val = ee
        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
        .unwrap();
    if let PropertyValue::List(fields) = val {
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0], PropertyValue::ObjectIdentifier(ai_oid));
        assert_eq!(
            fields[1],
            PropertyValue::Unsigned(PropertyIdentifier::PRESENT_VALUE.to_raw() as u64)
        );
        assert_eq!(fields[2], PropertyValue::Null); // no array index
        assert_eq!(fields[3], PropertyValue::Null); // no device
    } else {
        panic!("Expected List");
    }
}

#[test]
fn write_notification_class() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(42),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(42));
}

#[test]
fn write_event_enable() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    // Write only TO_OFFNORMAL enabled (bit 0 = 0b100 = 0x80 when shifted)
    ee.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        },
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        }
    );
}

#[test]
fn property_list_complete() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let props = ee.property_list();
    assert!(props.contains(&PropertyIdentifier::EVENT_TYPE));
    assert!(props.contains(&PropertyIdentifier::NOTIFY_TYPE));
    assert!(props.contains(&PropertyIdentifier::EVENT_PARAMETERS));
    assert!(props.contains(&PropertyIdentifier::OBJECT_PROPERTY_REFERENCE));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::EVENT_ENABLE));
    assert!(props.contains(&PropertyIdentifier::ACKED_TRANSITIONS));
    assert!(props.contains(&PropertyIdentifier::NOTIFICATION_CLASS));
}

#[test]
fn write_event_parameters() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let params = vec![0x01, 0x02, 0x03];
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::OctetString(params.clone()),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::OctetString(params));
}

#[test]
fn read_event_state_default() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn write_unknown_property_denied() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let result = ee.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
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
    ee.set_fault_parameters(Some(FaultParameters::FaultNone));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn fault_parameters_character_string() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultCharacterString {
        fault_values: vec!["alarm".to_string(), "critical".to_string()],
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));
}

#[test]
fn fault_parameters_extended() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultExtended {
        vendor_id: 42,
        extended_fault_type: 7,
        parameters: vec![0x01, 0x02],
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
}

#[test]
fn fault_parameters_life_safety() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultLifeSafety {
        fault_values: vec![1, 2, 3],
        mode_for_reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(3));
}

#[test]
fn fault_parameters_state() {
    use bacnet_types::constructed::BACnetPropertyStates;
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::BooleanValue(true)],
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(4));
}

#[test]
fn fault_parameters_status_flags() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::STATUS_FLAGS.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
}

#[test]
fn fault_parameters_out_of_range() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(6));
}

#[test]
fn fault_parameters_listed() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultListed {
        reference: BACnetDeviceObjectPropertyReference {
            object_identifier: ai_oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        },
    }));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(7));
}

#[test]
fn fault_parameters_in_property_list() {
    let ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    let props = ee.property_list();
    assert!(props.contains(&PropertyIdentifier::FAULT_PARAMETERS));
}

#[test]
fn fault_parameters_clear() {
    let mut ee = EventEnrollmentObject::new(1, "EE-FP", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultNone));
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));

    // Clear back to None
    ee.set_fault_parameters(None);
    let val = ee
        .read_property(PropertyIdentifier::FAULT_PARAMETERS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
}

// -----------------------------------------------------------------------
// AlertEnrollmentObject tests
// -----------------------------------------------------------------------

#[test]
fn alert_enrollment_create() {
    let ae = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    assert_eq!(
        ae.object_identifier().object_type(),
        ObjectType::ALERT_ENROLLMENT
    );
    assert_eq!(ae.object_identifier().instance_number(), 1);
    assert_eq!(ae.object_name(), "AE-1");
}

#[test]
fn alert_enrollment_object_type() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::ALERT_ENROLLMENT.to_raw())
    );
}

#[test]
fn alert_enrollment_present_value_default() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn alert_enrollment_event_detection_enable() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn alert_enrollment_write_event_detection_enable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn alert_enrollment_event_enable() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    // Default event_enable = 0b111, shifted left 5 = 0b1110_0000
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1110_0000],
        }
    );
}

#[test]
fn alert_enrollment_write_event_enable() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        },
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b1000_0000],
        }
    );
}

#[test]
fn alert_enrollment_notification_class() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let val = ae
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn alert_enrollment_write_notification_class() {
    let mut ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    ae.write_property(
        PropertyIdentifier::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(42),
        None,
    )
    .unwrap();
    let val = ae
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(42));
}

#[test]
fn alert_enrollment_property_list() {
    let ae = AlertEnrollmentObject::new(1, "AE").unwrap();
    let props = ae.property_list();
    assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(props.contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
    assert!(props.contains(&PropertyIdentifier::EVENT_ENABLE));
    assert!(props.contains(&PropertyIdentifier::NOTIFICATION_CLASS));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
}
