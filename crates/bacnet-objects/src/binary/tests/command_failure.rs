use super::super::*;
use bacnet_types::enums::{EventState, EventType};

fn write_enumerated(object: &mut BinaryOutputObject, property: PropertyIdentifier, value: u32) {
    object
        .write_property(property, None, PropertyValue::Enumerated(value), None)
        .unwrap();
}

#[test]
fn bo_feedback_value_round_trips_and_is_advertised_writable() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();

    write_enumerated(&mut bo, PropertyIdentifier::FEEDBACK_VALUE, 1);

    assert_eq!(
        bo.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert!(bo
        .property_list()
        .contains(&PropertyIdentifier::FEEDBACK_VALUE));
    assert!(bo.is_writable_property(PropertyIdentifier::FEEDBACK_VALUE));
    assert!(bo
        .write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .is_err());
}

#[test]
fn bo_command_failure_uses_present_and_feedback_values() {
    let mut disagreeing = BinaryOutputObject::new(1, "BO-disagree").unwrap();
    write_enumerated(&mut disagreeing, PropertyIdentifier::PRESENT_VALUE, 1);

    let outcome = disagreeing.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

    let mut agreeing = BinaryOutputObject::new(2, "BO-agree").unwrap();
    write_enumerated(&mut agreeing, PropertyIdentifier::FEEDBACK_VALUE, 1);
    write_enumerated(&mut agreeing, PropertyIdentifier::PRESENT_VALUE, 1);

    assert_eq!(agreeing.evaluate_intrinsic_reporting(), None);
    assert_eq!(
        agreeing
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn bo_time_delay_gates_command_failure() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.event_detector.time_delay = 2;
    write_enumerated(&mut bo, PropertyIdentifier::PRESENT_VALUE, 1);

    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(bo.tick_intrinsic_reporting(), None);
    let outcome = bo.tick_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}
