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
    // Default event_enable = 0b111 -> MSB-first wire byte 0b1110_0000
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
    // Write only TO_OFFNORMAL enabled (wire bit 0 = 0x80, Clause 20.2.10)
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

/// Decode the read arm's framed wire form back to a structured value.
fn decode_framed_event_parameters(
    val: PropertyValue,
) -> bacnet_types::constructed::BACnetEventParameter {
    let PropertyValue::ApplicationData(bytes) = val else {
        panic!("expected framed ApplicationData, got {val:?}");
    };
    bacnet_encoding::constructed::decode_event_parameter(&bytes, 0)
        .unwrap()
        .0
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
    // Legacy flat form write (still accepted as a decode fallback).
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
    assert_eq!(decode_framed_event_parameters(val), params);
    assert_eq!(params.tag(), event_parameter_tag::OUT_OF_RANGE);
}

#[test]
fn write_event_parameters_framed_round_trip() {
    use bacnet_types::constructed::BACnetEventParameter;
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let params = BACnetEventParameter::OutOfRange {
        time_delay: 5,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    };
    // Framed wire form write: exactly what a conformant peer sends.
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, &params);
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(buf.to_vec()),
        None,
    )
    .unwrap();
    let val = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert_eq!(decode_framed_event_parameters(val), params);
    // And the emitted read bytes are byte-identical to the written bytes.
    let val2 = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    let PropertyValue::ApplicationData(bytes) = val2 else {
        panic!("expected ApplicationData");
    };
    assert_eq!(bytes, buf.to_vec());
}

#[test]
fn write_event_parameters_framed_trailing_garbage_rejected() {
    use bacnet_types::constructed::BACnetEventParameter;
    // Review blocker: a well-formed framed element followed by garbage must
    // be REJECTED (pre-fix the head was accepted and the tail silently
    // dropped between wire and read-back).
    let params = BACnetEventParameter::OutOfRange {
        time_delay: 5,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    };
    let mut good = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut good, &params);
    for extra in 1..=4usize {
        let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
        let mut bytes = good.to_vec();
        bytes.extend_from_slice(&vec![0xAA; extra]);
        let result = ee.write_property(
            PropertyIdentifier::EVENT_PARAMETERS,
            None,
            PropertyValue::ApplicationData(bytes),
            None,
        );
        match result.unwrap_err() {
            bacnet_types::error::Error::Protocol { class, code } => {
                assert_eq!(
                    class,
                    bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
                );
                assert_eq!(
                    code,
                    bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE.to_raw() as u32
                );
            }
            other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
        }
        // Stored value is untouched: the default Opaque placeholder remains.
        let val = ee
            .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
            .unwrap();
        let PropertyValue::ApplicationData(bytes) = &val else {
            panic!("expected ApplicationData");
        };
        let (decoded, _) = bacnet_encoding::constructed::decode_event_parameter(bytes, 0).unwrap();
        assert_eq!(
            decoded,
            BACnetEventParameter::Opaque {
                tag: 0xFF,
                data: Vec::new()
            }
        );
    }
}

#[test]
fn write_event_parameters_framed_malformed_rejected() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    // Truncated framed value: out-of-range opening with no closing.
    let result = ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(vec![0x5E, 0x09, 0x07]),
        None,
    );
    assert!(result.is_err());
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
    match decode_framed_event_parameters(val) {
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
// ──────────────────────────────────────────────────────────────────────────
// #255 — Notify_Type production validation and Event_Enable bit-string
// width validation on the enrollment objects' own write arms.
// ──────────────────────────────────────────────────────────────────────────

use bacnet_types::enums::{ErrorClass, ErrorCode};

/// BACnetNotifyType is a closed {alarm(0), event(1), ack-notification(2)}
/// production (Clause 21). An out-of-production write is PROPERTY /
/// VALUE_OUT_OF_RANGE (Clause 15.9.1.3) and leaves the stored value untouched.
#[test]
fn ee_notify_type_rejects_out_of_production_values() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );

    for out_of_production in [3u32, 99, u32::MAX] {
        match ee
            .write_property(
                PropertyIdentifier::NOTIFY_TYPE,
                None,
                PropertyValue::Enumerated(out_of_production),
                None,
            )
            .expect_err("an out-of-production Notify_Type write must be refused")
        {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32);
            }
            other => panic!("expected PROPERTY / VALUE_OUT_OF_RANGE, got {other:?}"),
        }
        assert_eq!(
            ee.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(0),
            "a refused Notify_Type write must leave the value untouched ({out_of_production})"
        );
    }
    for in_production in [0u32, 1, 2] {
        ee.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(in_production),
            None,
        )
        .expect("named Notify_Type values must be accepted");
    }
}

/// BACnetEventTransitionBits is a 3-bit production (Clause 21); its canonical
/// encoding is one content octet with 5 unused bits. A write declaring any
/// other shape is PROPERTY / INVALID_DATA_ENCODING (Clause 15.9.1.3), not a
/// value to mask and normalize.
#[test]
fn ee_event_enable_rejects_noncanonical_bit_strings() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let canonical = ee
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();

    for (unused_bits, data) in [
        (0u8, vec![0xFFu8]),     // 8-bit string where the production defines 3
        (5u8, vec![0xFF, 0xFF]), // two content octets
        (4u8, vec![0xF0u8]),     // half-octet string
        (5u8, vec![]),           // no content octet
    ] {
        match ee
            .write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString { unused_bits, data },
                None,
            )
            .expect_err("a noncanonical Event_Enable shape must be refused")
        {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_ENCODING.to_raw() as u32);
            }
            other => panic!("expected PROPERTY / INVALID_DATA_ENCODING, got {other:?}"),
        }
        assert_eq!(
            ee.read_property(PropertyIdentifier::EVENT_ENABLE, None)
                .unwrap(),
            canonical,
            "a refused Event_Enable write must leave the value untouched"
        );
    }

    // The canonical full-width shape stays accepted.
    ee.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0], // to-offnormal + to-normal, MSB-first
        },
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        }
    );
}

/// AlertEnrollment shares the family defect by way of its own EVENT_ENABLE
/// arm; it honors the same 3-bit production, so it enforces the same shape.
#[test]
fn alert_enrollment_event_enable_rejects_noncanonical_bit_strings() {
    let mut ae = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    let canonical = ae
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();

    match ae
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xFF],
            },
            None,
        )
        .expect_err("an 8-bit string must be refused where 3 bits are defined")
    {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::INVALID_DATA_ENCODING.to_raw() as u32);
        }
        other => panic!("expected PROPERTY / INVALID_DATA_ENCODING, got {other:?}"),
    }
    assert_eq!(
        ae.read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        canonical,
        "a refused Event_Enable write must leave the value untouched"
    );
}
