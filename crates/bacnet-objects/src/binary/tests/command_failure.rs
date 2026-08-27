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
/// detection on an untouched object does not immediately report a command failure. This
/// states that property directly rather than relying on it incidentally: several other tests
/// in this module also fail if the initializer changes, but each does so as a side effect of
/// asserting something else, which is a fragile thing to depend on.
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

/// Clause 13.2.2.1 requires that while detection is disabled "no transitions shall occur".
/// Discarding an in-flight `Time_Delay` countdown is part of that: without it, a
/// disable-then-enable cycle would leave a stale `PendingTransition` so the next tick fires
/// immediately instead of restarting the full delay. Deleting `pending = None` from the write
/// arm otherwise leaves the whole suite green.
#[test]
fn bo_disabling_detection_discards_an_in_flight_time_delay_countdown() {
    /// Ticks a disagreeing object until it fires, returning how many ticks it took.
    /// Counting rather than hardcoding keeps this independent of whether the countdown is
    /// seeded by the probe or by the first tick.
    fn ticks_until_offnormal(bo: &mut BinaryOutputObject) -> usize {
        for n in 1..20 {
            if let Some(outcome) = bo.tick_intrinsic_reporting() {
                assert_eq!(outcome.change.to, EventState::OFFNORMAL);
                return n;
            }
        }
        panic!("never fired");
    }

    let mut baseline = BinaryOutputObject::new(1, "BO-baseline").unwrap();
    set_detection_enabled(&mut baseline, true);
    baseline.event_detector.time_delay = 3;
    write_enumerated(&mut baseline, PropertyIdentifier::PRESENT_VALUE, 1);
    let fresh = ticks_until_offnormal(&mut baseline);

    let mut cycled = BinaryOutputObject::new(2, "BO-cycled").unwrap();
    set_detection_enabled(&mut cycled, true);
    cycled.event_detector.time_delay = 3;
    write_enumerated(&mut cycled, PropertyIdentifier::PRESENT_VALUE, 1);

    // Spend part of the countdown, then disable and re-enable.
    assert_eq!(cycled.evaluate_intrinsic_reporting(), None);
    assert_eq!(cycled.tick_intrinsic_reporting(), None);
    set_detection_enabled(&mut cycled, false);
    set_detection_enabled(&mut cycled, true);

    // The full delay must run again from scratch. If the reset failed to discard the
    // in-flight PendingTransition, the partially-spent countdown would survive and this
    // object would fire sooner than a fresh one.
    assert_eq!(
        ticks_until_offnormal(&mut cycled),
        fresh,
        "a disable/enable cycle must restart Time_Delay, not resume it"
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
    let proposal = bo.tick_intrinsic_reporting().unwrap();
    let outcome = crate::event::commit_test_proposal(&mut bo, proposal);
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

/// Clause 13.3.4 applies the two delays asymmetrically: the OFFNORMAL
/// indication waits pTimeDelay (condition (a)), the NORMAL indication waits
/// pTimeDelayNormal (condition (b)). Both are commissioned as properties here
/// — no direct field access — so the wiring from Time_Delay_Normal through
/// the detector is exercised end to end at object level.
#[test]
fn bo_time_delay_normal_selects_the_return_to_normal_delay() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    set_detection_enabled(&mut bo, true);
    bo.write_property(
        PropertyIdentifier::TIME_DELAY,
        None,
        PropertyValue::Unsigned(2),
        None,
    )
    .unwrap();
    bo.write_property(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        None,
        PropertyValue::Unsigned(4),
        None,
    )
    .unwrap();
    write_enumerated(&mut bo, PropertyIdentifier::PRESENT_VALUE, 1);

    assert_eq!(bo.evaluate_intrinsic_reporting(), None);
    assert_eq!(bo.tick_intrinsic_reporting(), None);
    let proposal = bo.tick_intrinsic_reporting().unwrap();
    let outcome = crate::event::commit_test_proposal(&mut bo, proposal);
    assert_eq!(
        outcome.change.to,
        EventState::OFFNORMAL,
        "condition (a): OFFNORMAL after Time_Delay = 2"
    );
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

    write_enumerated(&mut bo, PropertyIdentifier::FEEDBACK_VALUE, 1);
    assert_eq!(
        bo.evaluate_intrinsic_reporting(),
        None,
        "condition (b): agreement only seeds the NORMAL countdown"
    );
    for _ in 0..3 {
        assert_eq!(
            bo.tick_intrinsic_reporting(),
            None,
            "Time_Delay_Normal = 4 must hold the NORMAL state off"
        );
    }
    let outcome = bo.tick_intrinsic_reporting().unwrap();
    assert_eq!(outcome.change.from, EventState::OFFNORMAL);
    assert_eq!(outcome.change.to, EventState::NORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn bo_event_enable_to_offnormal_bit_controls_distribution() {
    for (encoded, expected) in [(0x80, true), (0x00, false)] {
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
    ];
    for (property, value) in writes {
        bo.write_property(property, None, value.clone(), None)
            .unwrap();
        assert_eq!(bo.read_property(property, None).unwrap(), value);
    }

    // Acked_Transitions is readable but NOT writable: only the AcknowledgeAlarm service may
    // change it. A property write would assign where the service ORs, so it could both
    // fabricate and erase acknowledgments, and it would break the Clause 12.7 requirement
    // that the field sit at its initial condition while Event_Detection_Enable is FALSE.
    assert!(bo
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
    assert!(!bo.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));

    for property in [
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::TIME_DELAY,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyIdentifier::NOTIFICATION_CLASS,
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
