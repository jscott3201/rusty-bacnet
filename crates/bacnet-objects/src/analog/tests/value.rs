use super::super::*;
use crate::event::LimitEnable;
use bacnet_types::enums::EventState;

// --- AnalogValue ---

#[test]
fn av_read_present_value_default() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(0.0));
}

#[test]
fn av_set_present_value() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.set_present_value(42.5);
    let val = av
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(42.5));
}

#[test]
fn av_read_object_type_returns_analog_value() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw())
    );
}

#[test]
fn av_read_units() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av.read_property(PropertyIdentifier::UNITS, None).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(62));
}

#[test]
fn av_write_with_priority() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

    // Write at priority 8
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(55.0),
        Some(8),
    )
    .unwrap();

    let val = av
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(55.0));

    // Priority array at index 8 should have the value
    let slot = av
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Real(55.0));

    // Priority array at index 1 should be Null
    let slot = av
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .unwrap();
    assert_eq!(slot, PropertyValue::Null);
}

#[test]
fn av_relinquish_falls_to_default() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

    // Write at priority 16 (lowest)
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(75.0),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(75.0)
    );

    // Relinquish (write Null)
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    )
    .unwrap();

    // Should fall back to relinquish-default (0.0)
    assert_eq!(
        av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn av_higher_priority_wins() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();

    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(10.0),
        Some(16),
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(90.0),
        Some(8),
    )
    .unwrap();

    // Priority 8 wins over 16
    assert_eq!(
        av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(90.0)
    );
}

#[test]
fn av_priority_array_read_all_slots_none_by_default() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
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
fn av_priority_array_index_zero_returns_size() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(16));
}

#[test]
fn av_priority_array_index_out_of_bounds() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(17));
    assert!(result.is_err());
}

#[test]
fn av_priority_array_index_u32_max_out_of_bounds() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(u32::MAX));
    assert!(result.is_err());
}

#[test]
fn av_write_with_priority_zero_rejected() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(0),
    );
    assert!(result.is_err());
}

#[test]
fn av_write_with_priority_17_rejected() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(17),
    );
    assert!(result.is_err());
}

#[test]
fn av_write_with_all_valid_priorities() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    for prio in 1..=16u8 {
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(prio as f32),
            Some(prio),
        )
        .unwrap();
    }
    // Present value should be the highest priority (priority 1)
    let val = av
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(1.0));
}

#[test]
fn av_direct_priority_array_write_value() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Real(42.0),
        None,
    )
    .unwrap();
    assert_eq!(
        av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(42.0)
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Real(42.0)
    );
}

#[test]
fn av_direct_priority_array_relinquish() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Real(42.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        av.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Null
    );
}

#[test]
fn av_direct_priority_array_no_index_error() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn av_direct_priority_array_index_zero_error() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn av_direct_priority_array_index_17_error() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let result = av.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(17),
        PropertyValue::Real(42.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn av_intrinsic_reporting_normal_to_high_limit_to_normal() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    // Configure: high=80, low=20, deadband=2, both limits enabled
    av.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x07 << 5],
        },
        None,
    )
    .unwrap();

    // Normal value — no transition
    av.set_present_value(50.0);
    assert!(av.evaluate_intrinsic_reporting().is_none());

    // Go above high limit
    av.set_present_value(81.0);
    let change = av.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.from, EventState::NORMAL);
    assert_eq!(change.to, EventState::HIGH_LIMIT);

    // Verify event_state property reads correctly
    assert_eq!(
        av.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );

    // Drop below deadband threshold → back to NORMAL
    av.set_present_value(77.0);
    let change = av.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.to, EventState::NORMAL);
}

#[test]
fn av_intrinsic_reporting_after_priority_write() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.write_property(
        PropertyIdentifier::HIGH_LIMIT,
        None,
        PropertyValue::Real(80.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::LOW_LIMIT,
        None,
        PropertyValue::Real(20.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::DEADBAND,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    av.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x07 << 5],
        },
        None,
    )
    .unwrap();

    // Write a high value via priority array
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(85.0),
        Some(8),
    )
    .unwrap();
    let change = av.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(change.to, EventState::HIGH_LIMIT);
}

#[test]
fn av_read_reliability_default() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

#[test]
fn av_description_read_write() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    // Default description is empty
    let val = av
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString(String::new()));
    // Write a description
    av.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Setpoint".into()),
        None,
    )
    .unwrap();
    let val = av
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("Setpoint".into()));
}

#[test]
fn av_set_description_convenience() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.set_description("Zone temperature setpoint");
    assert_eq!(
        av.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Zone temperature setpoint".into())
    );
}

#[test]
fn av_description_in_property_list() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    assert!(av
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn av_property_list_includes_priority_array_and_relinquish_default() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let list = av.property_list();
    assert!(list.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(list.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    assert!(list.contains(&PropertyIdentifier::COV_INCREMENT));
    assert!(list.contains(&PropertyIdentifier::UNITS));
}

#[test]
fn av_read_event_state_default_normal() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(EventState::NORMAL.to_raw()));
}

#[test]
fn av_cov_increment_read_write() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    av.write_property(
        PropertyIdentifier::COV_INCREMENT,
        None,
        PropertyValue::Real(1.5),
        None,
    )
    .unwrap();
    assert_eq!(
        av.read_property(PropertyIdentifier::COV_INCREMENT, None)
            .unwrap(),
        PropertyValue::Real(1.5)
    );
    assert_eq!(av.cov_increment(), Some(1.5));
}

#[test]
fn av_read_relinquish_default() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let val = av
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(0.0));
}

#[test]
fn av_unknown_property_returns_error() {
    let av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    // File-object property does not exist on AV
    let result = av.read_property(PropertyIdentifier::FILE_SIZE, None);
    assert!(result.is_err());
}

// --- PROPERTY_LIST ---

#[test]
fn ai_property_list_returns_full_list() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let result = ai
        .read_property(PropertyIdentifier::PROPERTY_LIST, None)
        .unwrap();
    if let PropertyValue::List(elements) = result {
        assert!(!elements.is_empty());
        assert!(matches!(elements[0], PropertyValue::Enumerated(_)));
    } else {
        panic!("Expected PropertyValue::List");
    }
}

#[test]
fn ai_property_list_index_zero_returns_count() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // Property_List excludes OBJECT_IDENTIFIER, OBJECT_NAME,
    // OBJECT_TYPE, and PROPERTY_LIST itself.
    let filtered_count = ai
        .property_list()
        .iter()
        .filter(|p| {
            **p != PropertyIdentifier::OBJECT_IDENTIFIER
                && **p != PropertyIdentifier::OBJECT_NAME
                && **p != PropertyIdentifier::OBJECT_TYPE
                && **p != PropertyIdentifier::PROPERTY_LIST
        })
        .count() as u64;
    let result = ai
        .read_property(PropertyIdentifier::PROPERTY_LIST, Some(0))
        .unwrap();
    assert_eq!(result, PropertyValue::Unsigned(filtered_count));
}

#[test]
fn ai_property_list_index_one_returns_first_prop() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    // First property after filtering the 4 excluded ones
    let first_filtered = ai
        .property_list()
        .iter()
        .copied()
        .find(|p| {
            *p != PropertyIdentifier::OBJECT_IDENTIFIER
                && *p != PropertyIdentifier::OBJECT_NAME
                && *p != PropertyIdentifier::OBJECT_TYPE
                && *p != PropertyIdentifier::PROPERTY_LIST
        })
        .unwrap();
    let result = ai
        .read_property(PropertyIdentifier::PROPERTY_LIST, Some(1))
        .unwrap();
    assert_eq!(result, PropertyValue::Enumerated(first_filtered.to_raw()));
}

#[test]
fn ai_property_list_invalid_index_returns_error() {
    let ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let count = ai.property_list().len() as u32;
    let result = ai.read_property(PropertyIdentifier::PROPERTY_LIST, Some(count + 1));
    assert!(result.is_err());
}
