use super::super::*;
use crate::event::LimitEnable;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::EventState;
use bytes::BytesMut;

// --- AnalogOutput ---

#[test]
fn ao_write_with_priority() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

    // Write at priority 8
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(8),
    )
    .unwrap();

    let val = ao
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(50.0));

    // Priority array at index 8 should have the value
    let slot = ao
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Real(50.0));

    // Priority array at index 1 should be Null
    let slot = ao
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .unwrap();
    assert_eq!(slot, PropertyValue::Null);
}

#[test]
fn ao_priority_array_real_encodes_as_application_value() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(8),
    )
    .unwrap();

    let priority_value = ao
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &priority_value).unwrap();

    assert_eq!(encoded.as_ref(), &[0x44, 0x42, 0x48, 0x00, 0x00]);
}

#[test]
fn ao_relinquish_falls_to_default() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

    // Write at priority 16 (lowest)
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(75.0),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(75.0)
    );

    // Relinquish (write Null)
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    )
    .unwrap();

    // Should fall back to relinquish-default (0.0)
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn ao_higher_priority_wins() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();

    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(10.0),
        Some(16),
    )
    .unwrap();
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(90.0),
        Some(8),
    )
    .unwrap();

    // Priority 8 wins over 16
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(90.0)
    );
}

#[test]
fn ao_intrinsic_reporting_after_priority_write() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    ao.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ao.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ao.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ao.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    ao.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0], // all transitions, MSB-first
        },
        None,
    )
    .unwrap();

    // Write a high value via priority array
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(85.0),
        Some(8),
    )
    .unwrap();
    let change = ao.evaluate_intrinsic_reporting().unwrap().change;
    assert_eq!(change.to, EventState::HIGH_LIMIT);
}

#[test]
fn ao_description_read_write() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    ao.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Chilled water valve".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Chilled water valve".into())
    );
}

#[test]
fn ao_description_in_property_list() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    assert!(ao
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn ao_read_reliability_default() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let val = ao
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

#[test]
fn ao_priority_array_index_zero_returns_size() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let val = ao
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(16));
}

#[test]
fn ao_priority_array_index_out_of_bounds() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Index 17 is out of bounds (valid: 0-16)
    let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
    assert!(result.is_err());
}

#[test]
fn ao_priority_array_index_far_out_of_bounds() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Large index well beyond valid range
    let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(100));
    assert!(result.is_err());
}

#[test]
fn ao_priority_array_index_u32_max_out_of_bounds() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let result = ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(u32::MAX));
    assert!(result.is_err());
}

#[test]
fn ao_write_with_priority_zero_rejected() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Priority 0 is invalid (valid range is 1-16)
    let result = ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(0),
    );
    assert!(result.is_err());
}

#[test]
fn ao_write_with_priority_17_rejected() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Priority 17 is invalid (valid range is 1-16)
    let result = ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(17),
    );
    assert!(result.is_err());
}

#[test]
fn ao_write_with_priority_255_rejected() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Priority 255 is invalid
    let result = ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(255),
    );
    assert!(result.is_err());
}

#[test]
fn ao_write_with_all_valid_priorities() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // All priorities 1 through 16 should succeed
    for prio in 1..=16u8 {
        ao.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(prio as f32),
            Some(prio),
        )
        .unwrap();
    }
    // Present value should be the highest priority (priority 1)
    let val = ao
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(1.0));
}

#[test]
fn ao_priority_array_read_all_slots_none_by_default() {
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Read entire array (no index)
    let val = ao
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    if let PropertyValue::List(elements) = val {
        assert_eq!(elements.len(), 16);
        for elem in &elements {
            assert_eq!(elem, &PropertyValue::Null);
        }
    } else {
        panic!("Expected List for priority array without index");
    }
}

#[test]
fn ao_direct_priority_array_write_value() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Write directly to PRIORITY_ARRAY[5]
    ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Real(42.0),
        None,
    )
    .unwrap();
    // present_value should reflect the written value
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(42.0)
    );
    // Slot 5 should have the value
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Real(42.0)
    );
}

#[test]
fn ao_direct_priority_array_relinquish() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Write a value at priority 5
    ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Real(42.0),
        None,
    )
    .unwrap();
    // Relinquish with Null
    ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    // Should fall back to relinquish default (0.0)
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Null
    );
}

#[test]
fn ao_direct_priority_array_no_index_error() {
    // #266: an omitted array index means whole-array access (Clause 12.1.5.1);
    // whole-array writes are unsupported here, so the object must surface a
    // mappable PROPERTY / WRITE_ACCESS_DENIED protocol error (Result(-),
    // Clause 15.9.1.3), not an opaque Error::Encoding.
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    match ao
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap_err()
    {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
            );
            assert_eq!(
                code,
                bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
            );
        }
        other => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
    }
}

#[test]
fn ao_direct_priority_array_index_zero_error() {
    // Element 0 is the read-only array size: outside the writable 1..=16
    // slots → INVALID_ARRAY_INDEX.
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    match ao
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap_err()
    {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
            );
            assert_eq!(
                code,
                bacnet_types::enums::ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32
            );
        }
        other => panic!("expected PROPERTY/INVALID_ARRAY_INDEX, got {other:?}"),
    }
}

#[test]
fn ao_direct_priority_array_index_17_error() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    match ao
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            PropertyValue::Real(42.0),
            None,
        )
        .unwrap_err()
    {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
            );
            assert_eq!(
                code,
                bacnet_types::enums::ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32
            );
        }
        other => panic!("expected PROPERTY/INVALID_ARRAY_INDEX, got {other:?}"),
    }
}

#[test]
fn ao_is_createable_matches_factory() {
    use crate::traits::BACnetObject;
    let ao = AnalogOutputObject::new(1, "ao-1", 95).unwrap();
    assert!(ao.is_createable(), "AnalogOutput is factory-constructable");
}

#[test]
fn ao_is_writable_property_mirrors_write_property() {
    use crate::traits::BACnetObject;
    let ao = AnalogOutputObject::new(1, "ao-1", 95).unwrap();
    // Commandable.
    assert!(ao.is_writable_property(PropertyIdentifier::PRIORITY_ARRAY));
    assert!(ao.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    // Event + common.
    assert!(ao.is_writable_property(PropertyIdentifier::LIMIT_ENABLE));
    assert!(ao.is_writable_property(PropertyIdentifier::NOTIFY_TYPE));
    assert!(ao.is_writable_property(PropertyIdentifier::TIME_DELAY));
    assert!(ao.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
    // #270: RELINQUISH_DEFAULT grew a validated write arm.
    assert!(ao.is_writable_property(PropertyIdentifier::RELINQUISH_DEFAULT));
}

/// #270: a Relinquish_Default write is validated like a commanded
/// Present_Value (finite Real) and — with an all-NULL priority array —
/// Present_Value immediately resolves to the written default.
#[test]
fn ao_relinquish_default_write_recaptures_present_value() {
    let mut ao = AnalogOutputObject::new(1, "ao-1", 95).unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );

    ao.write_property(
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        PropertyValue::Real(72.5),
        None,
    )
    .unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Real(72.5)
    );
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(72.5),
        "with an empty priority array, PV must resolve to the written default"
    );

    // A live command still outranks the default, and relinquishing it falls
    // back to the new default.
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(55.0),
        Some(8),
    )
    .unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(55.0)
    );
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(72.5)
    );

    // Non-finite and wrong-typed writes refuse with the property untouched.
    for (value, code) in [
        (PropertyValue::Real(f32::NAN), "VALUE_OUT_OF_RANGE"),
        (PropertyValue::Real(f32::INFINITY), "VALUE_OUT_OF_RANGE"),
        (PropertyValue::Unsigned(72), "INVALID_DATA_TYPE"),
    ] {
        match ao
            .write_property(PropertyIdentifier::RELINQUISH_DEFAULT, None, value, None)
            .expect_err("{code}: invalid Relinquish_Default write must refuse")
        {
            Error::Protocol { class, code: c } => {
                assert_eq!(
                    class,
                    bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
                );
                assert_eq!(
                    c,
                    match code {
                        "VALUE_OUT_OF_RANGE" =>
                            bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32,
                        _ => bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                    }
                );
            }
            other => panic!("expected protocol error, got {other:?}"),
        }
        assert_eq!(
            ao.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
                .unwrap(),
            PropertyValue::Real(72.5),
            "a refused write must leave Relinquish_Default untouched"
        );
    }

    // The local setter shares the validation.
    assert!(ao.set_relinquish_default(f32::NAN).is_err());
    ao.set_relinquish_default(10.0).unwrap();
    assert_eq!(
        ao.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(10.0)
    );
}
