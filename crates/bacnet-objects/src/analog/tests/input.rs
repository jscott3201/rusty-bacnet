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
            data: vec![0xE0], // all transitions, MSB-first
        },
        None,
    )
    .unwrap();

    // Normal value — no transition
    ai.set_present_value(50.0);
    assert!(ai.evaluate_intrinsic_reporting().is_none());

    // Go above high limit
    ai.set_present_value(81.0);
    let change = ai.evaluate_intrinsic_reporting().unwrap().change;
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
    let change = ai.evaluate_intrinsic_reporting().unwrap().change;
    assert_eq!(change.to, EventState::NORMAL);
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
fn ai_is_createable_matches_factory() {
    use crate::traits::BACnetObject;
    let ai = AnalogInputObject::new(1, "ai-1", 95).unwrap();
    assert!(ai.is_createable(), "AnalogInput is factory-constructable");
}

#[test]
fn ai_is_writable_property_mirrors_write_property() {
    use crate::traits::BACnetObject;
    let ai = AnalogInputObject::new(1, "ai-1", 95).unwrap();
    // Event properties accepted via write_generic_event_properties! / write_analog_event_properties! — the old
    // heuristic omitted these (false-negatives).
    assert!(ai.is_writable_property(PropertyIdentifier::LIMIT_ENABLE));
    assert!(ai.is_writable_property(PropertyIdentifier::NOTIFY_TYPE));
    assert!(ai.is_writable_property(PropertyIdentifier::TIME_DELAY));
    assert!(ai.is_writable_property(PropertyIdentifier::EVENT_ENABLE));
    assert!(ai.is_writable_property(PropertyIdentifier::NOTIFICATION_CLASS));
    // Common + RELIABILITY + COV_INCREMENT.
    assert!(ai.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
    assert!(ai.is_writable_property(PropertyIdentifier::OBJECT_NAME));
    assert!(ai.is_writable_property(PropertyIdentifier::DESCRIPTION));
    assert!(ai.is_writable_property(PropertyIdentifier::RELIABILITY));
    assert!(ai.is_writable_property(PropertyIdentifier::COV_INCREMENT));
    // PRESENT_VALUE accepted when out-of-service.
    assert!(ai.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    // Read-only despite being an event property: Acked_Transitions is modified only by the
    // AcknowledgeAlarm service, which ORs the acknowledged bit in where a property write
    // would assign — so an assignable arm could both fabricate and erase acknowledgments,
    // and GetAlarmSummary / GetEventInformation read the field straight off the object.
    // Asserted on both halves because this test is the mirror check: the #222 macro split
    // silently turned this write into Ok(()) on all three analog types, and nothing here
    // caught it.
    let mut ai_mut = AnalogInputObject::new(2, "ai-2", 95).unwrap();
    assert!(!ai.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));
    assert!(ai_mut
        .write_property(
            PropertyIdentifier::ACKED_TRANSITIONS,
            None,
            bacnet_types::primitives::PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x80],
            },
            None,
        )
        .is_err());

    // Universal read-only.
    assert!(!ai.is_writable_property(PropertyIdentifier::OBJECT_IDENTIFIER));
    assert!(!ai.is_writable_property(PropertyIdentifier::OBJECT_TYPE));
    assert!(!ai.is_writable_property(PropertyIdentifier::PROPERTY_LIST));
    assert!(!ai.is_writable_property(PropertyIdentifier::STATUS_FLAGS));
    // Not accepted by AI write_property.
    assert!(!ai.is_writable_property(PropertyIdentifier::PRIORITY_ARRAY));
    assert!(!ai.is_writable_property(PropertyIdentifier::RELINQUISH_DEFAULT));
}
