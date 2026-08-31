//! Multi-state Number_Of_States configuration and object-owned Reliability.

use super::super::*;
use crate::traits::ReliabilityEvaluation;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, Reliability};

fn read_reliability(object: &dyn BACnetObject) -> u32 {
    match object
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
    {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected Enumerated Reliability, got {other:?}"),
    }
}

fn read_unsigned(object: &dyn BACnetObject, property: PropertyIdentifier) -> u64 {
    match object.read_property(property, None).unwrap() {
        PropertyValue::Unsigned(value) => value,
        other => panic!("expected Unsigned property, got {other:?}"),
    }
}

fn read_state_text(object: &dyn BACnetObject) -> Vec<PropertyValue> {
    match object
        .read_property(PropertyIdentifier::STATE_TEXT, None)
        .unwrap()
    {
        PropertyValue::List(values) => values,
        other => panic!("expected State_Text list, got {other:?}"),
    }
}

fn write_bool(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: bool) {
    object
        .write_property(property, None, PropertyValue::Boolean(value), None)
        .unwrap();
}

fn write_unsigned(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: u64) {
    object
        .write_property(property, None, PropertyValue::Unsigned(value), None)
        .unwrap();
}

fn assert_unknown_property(result: Result<PropertyValue, Error>) {
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
    ));
}

macro_rules! assert_number_of_states_policy {
    ($object:expr) => {{
        let mut object = $object;
        object
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(1),
                PropertyValue::CharacterString("Retained one".into()),
                None,
            )
            .unwrap();
        object
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(2),
                PropertyValue::CharacterString("Retained two".into()),
                None,
            )
            .unwrap();
        object
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(3),
                PropertyValue::CharacterString("Discarded three".into()),
                None,
            )
            .unwrap();

        let before_count = read_unsigned(&object, PropertyIdentifier::NUMBER_OF_STATES);
        let before_text = read_state_text(&object);
        let before_reliability = read_reliability(&object);
        assert!(object.set_number_of_states(0).is_err());
        assert_eq!(
            read_unsigned(&object, PropertyIdentifier::NUMBER_OF_STATES),
            before_count
        );
        assert_eq!(read_state_text(&object), before_text);
        assert_eq!(read_reliability(&object), before_reliability);

        object.set_number_of_states(2).unwrap();
        assert_eq!(
            read_unsigned(&object, PropertyIdentifier::NUMBER_OF_STATES),
            2
        );
        assert_eq!(
            read_state_text(&object),
            vec![
                PropertyValue::CharacterString("Retained one".into()),
                PropertyValue::CharacterString("Retained two".into()),
            ]
        );
        assert!(object
            .read_property(PropertyIdentifier::STATE_TEXT, Some(3))
            .is_err());

        object.set_number_of_states(4).unwrap();
        assert_eq!(
            read_state_text(&object),
            vec![
                PropertyValue::CharacterString("Retained one".into()),
                PropertyValue::CharacterString("Retained two".into()),
                PropertyValue::CharacterString("State 3".into()),
                PropertyValue::CharacterString("State 4".into()),
            ],
            "growth must append constructor-style labels, not resurrect truncated text"
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
                .unwrap(),
            PropertyValue::CharacterString("State 4".into())
        );
        assert!(object
            .read_property(PropertyIdentifier::STATE_TEXT, Some(5))
            .is_err());

        assert!(object
            .write_property(
                PropertyIdentifier::NUMBER_OF_STATES,
                None,
                PropertyValue::Unsigned(3),
                None,
            )
            .is_err());
        assert!(!object.is_writable_property(PropertyIdentifier::NUMBER_OF_STATES));
        assert_eq!(
            read_unsigned(&object, PropertyIdentifier::NUMBER_OF_STATES),
            4,
            "network denial must leave local configuration unchanged"
        );
    }};
}

#[test]
fn local_count_setters_are_atomic_and_resize_state_text_with_one_shared_policy() {
    assert_number_of_states_policy!(MultiStateInputObject::new(1, "MSI-1", 3).unwrap());
    assert_number_of_states_policy!(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap());
    assert_number_of_states_policy!(MultiStateValueObject::new(1, "MSV-1", 3).unwrap());
}

#[test]
fn msi_recomputes_range_reliability_synchronously_and_recovers() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();

    msi.set_present_value(4);
    assert_eq!(
        read_reliability(&msi),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw()
    );
    assert_eq!(
        msi.evaluate_reliability_internal().unwrap(),
        ReliabilityEvaluation::Unchanged,
        "the periodic safety-net hook must normally have nothing left to do"
    );

    msi.set_number_of_states(4).unwrap();
    assert_eq!(
        read_reliability(&msi),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    msi.set_number_of_states(1).unwrap();
    assert_eq!(
        read_reliability(&msi),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw()
    );
    msi.set_present_value(1);
    assert_eq!(
        read_reliability(&msi),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
}

#[test]
fn mso_configuration_sources_are_scanned_and_dominate_invalid_present_value() {
    let mut priority = MultiStateOutputObject::new(1, "MSO-priority", 3).unwrap();
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    priority.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&priority),
        Reliability::CONFIGURATION_ERROR.to_raw(),
        "an invalid inactive priority slot is still a configuration error"
    );
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    assert_eq!(
        read_reliability(&priority),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut default = MultiStateOutputObject::new(2, "MSO-default", 3).unwrap();
    default.set_relinquish_default(3).unwrap();
    default
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            Some(8),
        )
        .unwrap();
    default.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&default),
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    default.set_relinquish_default(2).unwrap();
    assert_eq!(
        read_reliability(&default),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut feedback = MultiStateOutputObject::new(3, "MSO-feedback", 3).unwrap();
    write_unsigned(&mut feedback, PropertyIdentifier::FEEDBACK_VALUE, 3);
    feedback.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&feedback),
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    write_unsigned(&mut feedback, PropertyIdentifier::FEEDBACK_VALUE, 2);
    assert_eq!(
        read_reliability(&feedback),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut combined = MultiStateOutputObject::new(4, "MSO-combined", 3).unwrap();
    combined.set_relinquish_default(3).unwrap();
    combined.set_number_of_states(2).unwrap();
    assert_eq!(
        read_unsigned(&combined, PropertyIdentifier::PRESENT_VALUE),
        3
    );
    assert_eq!(
        read_reliability(&combined),
        Reliability::CONFIGURATION_ERROR.to_raw(),
        "configuration error must dominate an out-of-range Present_Value"
    );
    combined.set_number_of_states(3).unwrap();
    assert_eq!(
        read_reliability(&combined),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
}

#[test]
fn msv_configuration_sources_recompute_immediately_and_fault_values_stays_absent() {
    let mut priority = MultiStateValueObject::new(1, "MSV-priority", 3).unwrap();
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    priority.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&priority),
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    priority
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    assert_eq!(
        read_reliability(&priority),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut default = MultiStateValueObject::new(2, "MSV-default", 3).unwrap();
    default.set_relinquish_default(3).unwrap();
    default
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            Some(8),
        )
        .unwrap();
    default.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&default),
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    default.set_relinquish_default(2).unwrap();
    assert_eq!(
        read_reliability(&default),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut alarms = MultiStateValueObject::new(3, "MSV-alarms", 3).unwrap();
    alarms.set_alarm_values(vec![3]);
    assert_eq!(
        read_reliability(&alarms),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    alarms.set_number_of_states(2).unwrap();
    assert_eq!(
        read_reliability(&alarms),
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    alarms.set_alarm_values(vec![2]);
    assert_eq!(
        read_reliability(&alarms),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    alarms
        .write_property(
            PropertyIdentifier::ALARM_VALUES,
            None,
            PropertyValue::List(vec![PropertyValue::Unsigned(4)]),
            None,
        )
        .unwrap();
    assert_eq!(
        read_reliability(&alarms),
        Reliability::CONFIGURATION_ERROR.to_raw(),
        "the list write route must funnel through the same synchronous setter"
    );

    assert_unknown_property(alarms.read_property(PropertyIdentifier::FAULT_VALUES, None));
    assert!(!alarms
        .property_list()
        .contains(&PropertyIdentifier::FAULT_VALUES));
    assert!(!alarms.is_writable_property(PropertyIdentifier::FAULT_VALUES));
}

#[test]
fn multistate_evaluator_recovers_only_faults_it_owns() {
    let mut msi = MultiStateInputObject::new(1, "MSI-owner", 2).unwrap();
    msi.set_reliability_internal(Reliability::NO_SENSOR.to_raw())
        .unwrap();
    msi.set_present_value(3);
    assert_eq!(read_reliability(&msi), Reliability::NO_SENSOR.to_raw());

    msi.set_reliability_internal(Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw())
        .unwrap();
    msi.set_present_value(3);
    msi.set_present_value(1);
    assert_eq!(
        read_reliability(&msi),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw(),
        "an equal numeric Reliability without ownership must not be claimed or cleared"
    );

    msi.set_reliability_internal(Reliability::NO_FAULT_DETECTED.to_raw())
        .unwrap();
    msi.set_present_value(3);
    assert_eq!(
        read_reliability(&msi),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw()
    );
    msi.set_present_value(1);
    assert_eq!(
        read_reliability(&msi),
        Reliability::NO_FAULT_DETECTED.to_raw(),
        "an evaluator-owned fault must recover"
    );
}

#[test]
fn oos_and_inhibit_suppress_mutations_then_release_current_state_synchronously() {
    let mut inhibited = MultiStateInputObject::new(1, "MSI-inhibited", 2).unwrap();
    write_bool(
        &mut inhibited,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        true,
    );
    inhibited.set_present_value(3);
    assert_eq!(
        read_reliability(&inhibited),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );
    write_bool(
        &mut inhibited,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        false,
    );
    assert_eq!(
        read_reliability(&inhibited),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw(),
        "disabling inhibit must evaluate the current value before returning"
    );
    write_bool(
        &mut inhibited,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        true,
    );
    inhibited.set_present_value(1);
    write_bool(
        &mut inhibited,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        false,
    );
    assert_eq!(
        read_reliability(&inhibited),
        Reliability::NO_FAULT_DETECTED.to_raw()
    );

    let mut oos = MultiStateInputObject::new(2, "MSI-oos", 3).unwrap();
    write_bool(&mut oos, PropertyIdentifier::OUT_OF_SERVICE, true);
    write_bool(
        &mut oos,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        true,
    );
    oos.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    oos.set_number_of_states(1).unwrap();
    oos.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        None,
    )
    .unwrap();
    write_bool(
        &mut oos,
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        false,
    );
    assert_eq!(
        read_reliability(&oos),
        Reliability::NO_SENSOR.to_raw(),
        "accepted OOS alternate must remain authoritative while OOS"
    );
    write_bool(&mut oos, PropertyIdentifier::OUT_OF_SERVICE, false);
    assert_eq!(
        read_reliability(&oos),
        Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw(),
        "leaving OOS must restore then evaluate the retained current configuration"
    );
}

#[test]
fn synchronous_reliability_is_observed_by_the_existing_intrinsic_event_kernel() {
    let mut msi = MultiStateInputObject::new(1, "MSI-events", 2).unwrap();
    msi.set_present_value(3);
    let fault = msi
        .evaluate_intrinsic_reporting()
        .expect("synchronous Reliability fault must be observable");
    assert_eq!(fault.change.to, EventState::FAULT);
    crate::event::commit_test_proposal(&mut msi, fault);

    msi.set_present_value(1);
    let recovery = msi
        .evaluate_intrinsic_reporting()
        .expect("synchronous Reliability recovery must be observable");
    assert_eq!(recovery.change.from, EventState::FAULT);
    assert_eq!(recovery.change.to, EventState::NORMAL);
}
