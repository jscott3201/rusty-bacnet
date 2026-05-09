use super::super::*;
use crate::event::LimitEnable;
use bacnet_types::enums::EventState;

// --- AnalogInput ---

#[test]
fn ai_read_present_value() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap(); // 62 = degrees-fahrenheit
    ai.set_present_value(72.5);
    let val = ai
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(72.5));
}

#[test]
fn ai_read_units() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let val = ai.read_property(PropertyIdentifier::UNITS, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(62));
}

#[test]
fn ai_write_present_value_denied_when_in_service() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let result = ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(99.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn ai_write_present_value_allowed_when_out_of_service() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(99.0),
        None,
    )
    .unwrap();
    let val = ai
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(99.0));
}

#[test]
fn ai_read_unknown_property() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let result = ai.read_property(PropertyIdentifier::PRIORITY_ARRAY, None);
    assert!(result.is_err());
}

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

// --- Intrinsic Reporting ---

#[test]
fn ai_read_event_state_default_normal() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let val = ai
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(EventState::NORMAL.to_raw()));
}

#[test]
fn ai_read_write_high_limit() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(85.0),
        None,
    )
    .unwrap();
    assert_eq!(
        ai.read_property(PropertyIdentifier::HIGH_LIMIT, None)
            .unwrap(),
        PropertyValue::Real(85.0)
    );
}

#[test]
fn ai_read_write_low_limit() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(15.0),
        None,
    )
    .unwrap();
    assert_eq!(
        ai.read_property(PropertyIdentifier::LOW_LIMIT, None)
            .unwrap(),
        PropertyValue::Real(15.0)
    );
}

#[test]
fn ai_read_write_deadband() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.5),
        None,
    )
    .unwrap();
    assert_eq!(
        ai.read_property(PropertyIdentifier::DEADBAND, None)
            .unwrap(),
        PropertyValue::Real(2.5)
    );
}

#[test]
fn ai_deadband_reject_negative() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let result = ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(-1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn ai_read_write_limit_enable() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let enable_both = LimitEnable::BOTH.to_bits();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![enable_both],
        },
        None,
    )
    .unwrap();
    let val = ai
        .read_property(PropertyIdentifier::LIMIT_ENABLE, None)
        .unwrap();
    if let PropertyValue::BitString { data, .. } = val {
        let le = LimitEnable::from_bits(data[0]);
        assert!(le.low_limit_enable);
        assert!(le.high_limit_enable);
    } else {
        panic!("Expected BitString");
    }
}

#[test]
fn ai_intrinsic_reporting_triggers_on_present_value_change() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // Configure: high=80, low=20, deadband=2, both limits enabled
    ai.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x07 << 5], // all transitions enabled
        },
        None,
    )
    .unwrap();

    // Normal value — no transition
    ai.set_present_value(50.0);
    assert!(ai.evaluate_intrinsic_reporting().is_none());

    // Go above high limit
    ai.set_present_value(81.0);
    let change = ai.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::HIGH_LIMIT);

    // Verify event_state property reads correctly
    assert_eq!(
        ai.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );

    // Drop below deadband threshold → back to NORMAL
    ai.set_present_value(77.0);
    let change = ai.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.to, EventState::NORMAL);
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
            data: vec![0x07 << 5], // all transitions enabled
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
    let change = ao.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.to, EventState::HIGH_LIMIT);
}

#[test]
fn ai_read_reliability_default() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let val = ai
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

#[test]
fn ai_description_read_write() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // Default description is empty
    let val = ai
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString(String::new()));
    // Write a description
    ai.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Zone temperature sensor".into()),
        None,
    )
    .unwrap();
    let val = ai
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::CharacterString("Zone temperature sensor".into())
    );
}

#[test]
fn ai_set_description_convenience() {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_description("Supply air temperature");
    assert_eq!(
        ai.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Supply air temperature".into())
    );
}

#[test]
fn ai_description_in_property_list() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    assert!(ai
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
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

// --- Priority array bounds tests ---

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

// --- WriteProperty with invalid priority tests ---

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

// --- Direct PRIORITY_ARRAY writes ---

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
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Writing PRIORITY_ARRAY without array_index should error
    let result = ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn ao_direct_priority_array_index_zero_error() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let result = ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn ao_direct_priority_array_index_17_error() {
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let result = ao.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(17),
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}
