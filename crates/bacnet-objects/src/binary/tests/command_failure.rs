use super::super::*;
use bacnet_types::enums::{EventState, EventType};

fn write_enumerated(object: &mut BinaryOutputObject, property: PropertyIdentifier, value: u32) {
    object
        .write_property(property, None, PropertyValue::Enumerated(value), None)
        .unwrap();
}

fn write_event_enable(object: &mut BinaryOutputObject, byte: u8) {
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![byte],
            },
            None,
        )
        .unwrap();
}

fn set_detection_enabled(object: &mut BinaryOutputObject, enabled: bool) {
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

/// `BACnetBinaryPV` is a two-valued enumeration, so 2 is not a member of the datatype rather
/// than merely out of some configured range. Nothing else covered this: the round-trip test
/// writes 1 (accepted) and `Unsigned(1)` (rejected by the separate datatype arm), so the
/// guard could be deleted with a green suite.
#[test]
fn bo_feedback_value_rejects_a_value_outside_the_binary_enumeration() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();

    assert!(bo
        .write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            PropertyValue::Enumerated(2),
            None,
        )
        .is_err());
    assert_eq!(
        bo.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0),
        "a rejected write must not have stored anything"
    );
}

/// `feedback_value` initializes to match the initial `Present_Value` so that enabling
/// detection on an untouched object does not immediately report a command failure. With
/// detection now defaulting to FALSE, nothing else pins that initializer — every other test
/// writes both properties before evaluating.
#[test]
fn bo_fresh_object_reports_nothing_when_detection_is_enabled() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    set_detection_enabled(&mut bo, true);

    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(
        bo.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn bo_command_failure_uses_present_and_feedback_values() {
    let mut disagreeing = BinaryOutputObject::new(1, "BO-disagree").unwrap();
    set_detection_enabled(&mut disagreeing, true);
    write_enumerated(&mut disagreeing, PropertyIdentifier::PRESENT_VALUE, 1);

    let outcome = disagreeing.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

    let mut agreeing = BinaryOutputObject::new(2, "BO-agree").unwrap();
    set_detection_enabled(&mut agreeing, true);
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
    set_detection_enabled(&mut bo, true);
    bo.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(2),
        None,
    )
    .unwrap();
    write_enumerated(&mut bo, PropertyIdentifier::PRESENT_VALUE, 1);

    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(bo.tick_intrinsic_reporting(), None);
    let outcome = bo.tick_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn bo_event_enable_to_offnormal_bit_controls_distribution() {
    for (encoded, expected) in [(0x20, true), (0x00, false)] {
        let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
        set_detection_enabled(&mut bo, true);
        write_event_enable(&mut bo, encoded);
        write_enumerated(&mut bo, PropertyIdentifier::PRESENT_VALUE, 1);
        assert_eq!(
            bo.evaluate_intrinsic_reporting().unwrap().distribute,
            expected
        );
    }
}

#[test]
fn bo_detection_enable_is_a_disabled_by_default_invariant() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    write_enumerated(&mut bo, PropertyIdentifier::PRESENT_VALUE, 1);

    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(bo.tick_intrinsic_reporting(), None);
    assert_eq!(
        bo.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        }
    );

    bo.event_detector.time_delay = 2;
    set_detection_enabled(&mut bo, true);
    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    bo.event_detection_enable = false;
    let pending = bo.event_detector.pending.clone();
    assert_eq!(bo.tick_intrinsic_reporting(), None);
    assert_eq!(bo.event_detector.pending, pending);

    set_detection_enabled(&mut bo, true);
    bo.event_detector.time_delay = 0;
    assert_eq!(
        bo.evaluate_intrinsic_reporting().unwrap().change.to,
        EventState::OFFNORMAL
    );
    bo.event_detector.acked_transitions = 0;
    set_detection_enabled(&mut bo, false);

    assert_eq!(
        bo.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe0],
        }
    );
    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(bo.tick_intrinsic_reporting(), None);

    bo.reliability = 1;
    assert_eq!(
        bo.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x40],
        }
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert!(bo
        .property_list()
        .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
    assert!(bo.is_writable_property(PropertyIdentifier::EVENT_DETECTION_ENABLE));
}

#[test]
fn bo_generic_event_properties_round_trip_and_match_pics() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let writes = [
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(42),
        ),
        (
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x80],
            },
        ),
    ];
    for (property, value) in writes {
        bo.write_property(property, None, value.clone(), None)
            .unwrap();
        assert_eq!(bo.read_property(property, None).unwrap(), value);
    }

    for property in [
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::TIME_DELAY,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyIdentifier::ACKED_TRANSITIONS,
    ] {
        assert!(bo.property_list().contains(&property));
        assert!(bo.is_writable_property(property));
    }
    assert!(bo
        .write_property(
            PropertyIdentifier::EVENT_STATE,
            None,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
            None,
        )
        .is_err());
    assert!(!bo.is_writable_property(PropertyIdentifier::EVENT_STATE));
}
