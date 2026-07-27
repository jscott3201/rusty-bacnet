//! EventEnrollmentObject property and lifecycle tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;

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
fn write_event_parameters_structured_round_trip() {
    use bacnet_types::constructed::{event_parameter_tag, BACnetEventParameter};
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let params = BACnetEventParameter::OutOfRange {
        time_delay: 5,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    };
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        params.encode(),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert_eq!(BACnetEventParameter::decode(&val).unwrap(), params);
    assert_eq!(params.tag(), event_parameter_tag::OUT_OF_RANGE);
}

#[test]
fn write_event_parameters_opaque_octets_preserved() {
    use bacnet_types::constructed::BACnetEventParameter;
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    // Legacy raw-octet writes are preserved verbatim as an Opaque value so a
    // remote client that wrote an algorithm this library does not model is
    // never silently dropped.
    let bytes = vec![0x01, 0x02, 0x03];
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::OctetString(bytes.clone()),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    match BACnetEventParameter::decode(&val).unwrap() {
        BACnetEventParameter::Opaque { data, .. } => assert_eq!(data, bytes),
        other => panic!("expected Opaque, got {other:?}"),
    }
}

#[test]
fn write_event_parameters_rejects_non_list() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let result = ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::Boolean(true),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn read_event_state_default() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

/// `Event_State` is algorithmically derived (ASHRAE 135-2020 Clause 12.12) and
/// read-only over the network: a WriteProperty of `EVENT_STATE` is rejected
/// with `WRITE_ACCESS_DENIED` and leaves the field unchanged (issue #130).
#[test]
fn write_event_state_rejected_over_network() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let result = ee.write_property(
        PropertyIdentifier::EVENT_STATE,
        None,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
        None,
    );
    assert!(
        result.is_err(),
        "network EVENT_STATE write must be rejected"
    );
    // The field is unchanged.
    let val = ee
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(EventState::NORMAL.to_raw()));
}

/// The internal lifecycle path (`set_event_state_internal`) is the distinct
/// route the evaluator uses to persist a detected transition. It is not the
/// network `write_property` route (issue #130).
#[test]
fn set_event_state_internal_updates_field() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let result = ee.set_event_state_internal(EventState::HIGH_LIMIT);
    assert!(
        result.is_ok(),
        "internal setter must accept a modeled state"
    );
    let val = ee
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

/// The inherent `set_event_state` builder seeds state for tests/setup without
/// going through the network write route (issue #130).
#[test]
fn set_event_state_seeds_field() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_event_state(EventState::LOW_LIMIT.to_raw());
    let val = ee
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw())
    );
}

/// Objects that report `EVENT_ENROLLMENT` but do NOT model an algorithmic
/// `Event_State` get the trait default for `set_event_state_internal`, which
/// rejects with `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`. The server evaluator
/// drops such transitions (it only calls this on objects it found by type),
/// so a custom downstream impl that forgets to override the method fails
/// closed rather than silently storing garbage (issue #130).
#[test]
fn set_event_state_internal_default_rejects() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::primitives::PropertyValue;
    use std::borrow::Cow;

    /// Reports `EVENT_ENROLLMENT` but does not override
    /// `set_event_state_internal` — exercises the trait default.
    struct DefaultOnly {
        oid: ObjectIdentifier,
    }

    impl BACnetObject for DefaultOnly {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.oid
        }
        fn object_name(&self) -> &str {
            "default-only"
        }
        fn read_property(
            &self,
            property: PropertyIdentifier,
            _array_index: Option<u32>,
        ) -> Result<PropertyValue, bacnet_types::error::Error> {
            if property == PropertyIdentifier::OBJECT_NAME {
                Ok(PropertyValue::CharacterString("default-only".to_string()))
            } else {
                Err(bacnet_types::error::Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                })
            }
        }
        fn write_property(
            &mut self,
            _property: PropertyIdentifier,
            _array_index: Option<u32>,
            _value: PropertyValue,
            _priority: Option<u8>,
        ) -> Result<(), bacnet_types::error::Error> {
            Err(bacnet_types::error::Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            })
        }
        fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
            Cow::Borrowed(&[PropertyIdentifier::OBJECT_NAME])
        }
    }

    let mut obj = DefaultOnly {
        oid: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
    };
    let result = obj.set_event_state_internal(EventState::FAULT);
    assert!(
        result.is_err(),
        "default set_event_state_internal must reject"
    );
    let err = result.unwrap_err();
    match err {
        bacnet_types::error::Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::OBJECT.to_raw() as u32);
            assert_eq!(
                code,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
            );
        }
        _ => panic!("expected Error::Protocol from default set_event_state_internal"),
    }
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
