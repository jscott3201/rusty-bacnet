use super::*;
use bacnet_types::enums::{EventState, EventType};

fn write_unsigned(object: &mut MultiStateOutputObject, property: PropertyIdentifier, value: u64) {
    object
        .write_property_from(
            property,
            None,
            PropertyValue::Unsigned(value),
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
}

fn set_detection_enabled(object: &mut MultiStateOutputObject, enabled: bool) {
    object
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(enabled),
            None,
        )
        .unwrap();
}

#[test]
fn feedback_value_round_trips_and_is_advertised_writable() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();

    write_unsigned(&mut mso, PropertyIdentifier::FEEDBACK_VALUE, 2);

    assert_eq!(
        mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert!(mso
        .property_list()
        .contains(&PropertyIdentifier::FEEDBACK_VALUE));
    assert!(mso.is_writable_property(PropertyIdentifier::FEEDBACK_VALUE));
    assert!(mso
        .write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            PropertyValue::Enumerated(2),
            None,
        )
        .is_err());
}

/// Clause 12.19 defines an out-of-range Feedback_Value as a reportable condition
/// (Reliability CONFIGURATION_ERROR), not a value to refuse. Refusing it
/// would make that reliability unreachable, so the write is accepted even though
/// Present_Value at the same value would be rejected.
#[test]
fn feedback_value_outside_the_state_set_is_accepted_unlike_present_value() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();

    write_unsigned(&mut mso, PropertyIdentifier::FEEDBACK_VALUE, 7);
    assert_eq!(
        mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(7)
    );

    assert!(mso
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(7),
            None,
            &crate::command_source::test_origin(),
        )
        .is_err());
}

/// `feedback_value` initializes to match the initial `Present_Value` so that enabling
/// detection on an untouched object does not immediately report a command failure. This
/// states that property directly rather than relying on it incidentally: several other
/// tests in this module also fail if the initializer changes, but each does so as a side
/// effect of asserting something else, which is a fragile thing to depend on.
#[test]
fn fresh_object_reports_nothing_when_detection_is_enabled() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    set_detection_enabled(&mut mso, true);

    assert_eq!(mso.evaluate_intrinsic_reporting(), None);
    assert_eq!(
        mso.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// A BACnet Unsigned decodes from up to 8 octets, so an unchecked `as u32` would wrap
/// a large Feedback_Value back into the valid state range and suppress the very
/// transition this object type exists to report. 0x1_0000_0002 truncates to 2, which
/// would read as agreeing with a Present_Value of 2.
#[test]
fn oversized_feedback_value_is_rejected_rather_than_wrapped_into_agreement() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    set_detection_enabled(&mut mso, true);
    write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

    assert!(mso
        .write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            PropertyValue::Unsigned(0x1_0000_0002),
            None,
        )
        .is_err());

    // The rejected write left the feedback value alone, so Present_Value 2 against the
    // initial feedback of 1 still disagrees and still reports COMMAND_FAILURE.
    assert_eq!(
        mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    let outcome = mso.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn command_failure_uses_present_and_feedback() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    set_detection_enabled(&mut mso, true);
    write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

    let proposal = mso.evaluate_intrinsic_reporting().unwrap();
    let outcome = crate::event::commit_test_proposal(&mut mso, proposal);
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

    write_unsigned(&mut mso, PropertyIdentifier::FEEDBACK_VALUE, 2);
    let returned = mso.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(returned.change.from, EventState::OFFNORMAL);
    assert_eq!(returned.change.to, EventState::NORMAL);
    assert_eq!(returned.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn time_delay_gates_command_failure() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    set_detection_enabled(&mut mso, true);
    write_unsigned(&mut mso, PropertyIdentifier::TIME_DELAY, 2);
    write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

    assert_eq!(mso.evaluate_intrinsic_reporting(), None);
    assert_eq!(mso.tick_intrinsic_reporting(), None);
    let outcome = mso.tick_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn event_enable_to_offnormal_bit_controls_distribution() {
    for (encoded, expected) in [(0x80, true), (0x00, false)] {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        set_detection_enabled(&mut mso, true);
        mso.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![encoded],
            },
            None,
        )
        .unwrap();
        write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);
        assert_eq!(
            mso.evaluate_intrinsic_reporting().unwrap().distribute,
            expected
        );
    }
}

#[test]
fn generic_event_properties_round_trip_and_match_pics() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    let writes = [
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(42),
        ),
    ];
    for (property, value) in writes {
        mso.write_property_from(
            property,
            None,
            value.clone(),
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
        assert_eq!(mso.read_property(property, None).unwrap(), value);
    }

    // Acked_Transitions is readable but NOT writable: only the AcknowledgeAlarm service
    // may change it. A property write would assign where the service ORs, so it could
    // both fabricate and erase acknowledgments, and it would break the Clause 12.19
    // requirement that the field sit at its initial condition while
    // Event_Detection_Enable is FALSE.
    assert!(mso
        .write_property(
            PropertyIdentifier::ACKED_TRANSITIONS,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x80],
            },
            None,
        )
        .is_err());
    assert!(!mso.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));

    for property in [
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::TIME_DELAY,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyIdentifier::NOTIFICATION_CLASS,
    ] {
        assert!(mso.property_list().contains(&property));
        assert!(mso.is_writable_property(property));
    }
    assert!(mso
        .write_property(
            PropertyIdentifier::EVENT_STATE,
            None,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
            None,
        )
        .is_err());
    assert!(!mso.is_writable_property(PropertyIdentifier::EVENT_STATE));
}

#[test]
fn detection_enable_is_a_disabled_by_default_invariant() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

    assert_eq!(mso.evaluate_intrinsic_reporting(), None);
    assert_eq!(mso.tick_intrinsic_reporting(), None);
    assert_eq!(
        mso.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        mso.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );

    set_detection_enabled(&mut mso, true);
    assert_eq!(
        mso.evaluate_intrinsic_reporting().unwrap().change.to,
        EventState::OFFNORMAL
    );
    mso.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
    set_detection_enabled(&mut mso, false);

    assert_eq!(
        mso.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        mso.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe0],
        }
    );
    assert_eq!(mso.evaluate_intrinsic_reporting(), None);
    assert_eq!(mso.tick_intrinsic_reporting(), None);

    mso.reliability = Reliability::NO_SENSOR;
    assert_eq!(
        mso.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x40],
        }
    );
    assert_eq!(
        mso.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert!(mso
        .property_list()
        .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
    assert!(mso.is_writable_property(PropertyIdentifier::EVENT_DETECTION_ENABLE));
}
