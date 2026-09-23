use super::*;

use std::sync::Arc;

use bacnet_types::enums::LifeSafetyState;

/// Read Event_State as a raw enumeration value.
fn read_enumerated(object: &dyn BACnetObject, property: PropertyIdentifier) -> u32 {
    match object.read_property(property, None).unwrap() {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected enumerated value, got {other:?}"),
    }
}

/// Read Event_State as a raw enumeration value.
fn read_event_state(object: &dyn BACnetObject) -> u32 {
    read_enumerated(object, PropertyIdentifier::EVENT_STATE)
}

/// Assert the Status_Flags IN_ALARM bit (bit 7 of the first octet) is clear.
///
/// IN_ALARM mirrors a non-NORMAL Event_State per Clauses 12.15/12.16; with the
/// R1 verdict (no intrinsic reporting, Event_State frozen NORMAL) it must stay
/// clear across every LifeSafetyOperation.
fn assert_in_alarm_clear(object: &dyn BACnetObject) {
    match object
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => {
            assert!(!data.is_empty(), "Status_Flags must carry flag octets");
            assert_eq!(
                data[0] & 0x80,
                0,
                "IN_ALARM must stay clear while Event_State is NORMAL"
            );
        }
        other => panic!("expected Status_Flags bitstring, got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// PR-0803 sub-slice 1 (R1 outcome b): LSO leaves Event_State/IN_ALARM alone.
// -----------------------------------------------------------------------

#[test]
fn point_silence_unsilence_leaves_event_state_and_in_alarm_untouched() {
    // (initial Silenced, operation, expected Silenced)
    let cases = [
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE,
            SilencedState::ALL_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_AUDIBLE,
            SilencedState::AUDIBLE_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_VISUAL,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE,
            SilencedState::UNSILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_AUDIBLE,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_VISUAL,
            SilencedState::AUDIBLE_SILENCED,
        ),
    ];

    for (initial, operation, expected) in cases {
        let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        point.set_silenced(initial);
        point.set_operation_expected(operation);
        assert_eq!(read_event_state(&point), 0);
        assert_in_alarm_clear(&point);

        let outcome = point.apply_life_safety_operation(operation).unwrap();

        assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
        assert_eq!(
            read_enumerated(&point, PropertyIdentifier::SILENCED),
            expected.to_raw(),
            "operation {} from state {}",
            operation.to_raw(),
            initial.to_raw()
        );
        // R1 pins: intrinsic-only Event_State is untouched, so the snapshot
        // diff never reports EVENT_STATE or STATUS_FLAGS.
        assert_eq!(read_event_state(&point), 0);
        assert_in_alarm_clear(&point);
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::EVENT_STATE),
            "operation {} must not report EVENT_STATE",
            operation.to_raw()
        );
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::STATUS_FLAGS),
            "operation {} must not report STATUS_FLAGS",
            operation.to_raw()
        );
    }
}

#[test]
fn zone_silence_unsilence_leaves_event_state_and_in_alarm_untouched() {
    let cases = [
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE,
            SilencedState::ALL_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_AUDIBLE,
            SilencedState::AUDIBLE_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_VISUAL,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE,
            SilencedState::UNSILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_AUDIBLE,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_VISUAL,
            SilencedState::AUDIBLE_SILENCED,
        ),
    ];

    for (initial, operation, expected) in cases {
        let mut zone = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        zone.set_silenced(initial);
        zone.set_operation_expected(operation);
        assert_eq!(read_event_state(&zone), 0);
        assert_in_alarm_clear(&zone);

        let outcome = zone.apply_life_safety_operation(operation).unwrap();

        assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
        assert_eq!(
            read_enumerated(&zone, PropertyIdentifier::SILENCED),
            expected.to_raw(),
            "operation {} from state {}",
            operation.to_raw(),
            initial.to_raw()
        );
        assert_eq!(read_event_state(&zone), 0);
        assert_in_alarm_clear(&zone);
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::EVENT_STATE),
            "operation {} must not report EVENT_STATE",
            operation.to_raw()
        );
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::STATUS_FLAGS),
            "operation {} must not report STATUS_FLAGS",
            operation.to_raw()
        );
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::TRACKING_VALUE),
            "zone must never invent TRACKING_VALUE"
        );
    }
}

// -----------------------------------------------------------------------
// PR-0803 sub-slice 1 (R1 outcome b): resets leave Event_State/IN_ALARM
// alone. Even a PV-changing executor commit must not move the intrinsic-only
// Event_State, so the snapshot diff never reports EVENT_STATE/STATUS_FLAGS.
// -----------------------------------------------------------------------

#[test]
fn point_resets_leave_event_state_and_in_alarm_untouched() {
    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_present_value(LifeSafetyState::ALARM.to_raw());
        point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
        point.set_silenced(SilencedState::ALL_SILENCED);
        point.set_operation_expected(operation);
        point.set_reset_executor(Arc::new(|_| {
            Ok(LifeSafetyPointResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                tracking_value: Some(LifeSafetyState::QUIET),
                silenced: Some(SilencedState::UNSILENCED),
            })
        }));
        assert_eq!(read_event_state(&point), 0);
        assert_in_alarm_clear(&point);

        let outcome = point.apply_life_safety_operation(operation).unwrap();

        assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
        assert_eq!(
            outcome.changed_properties,
            vec![
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::TRACKING_VALUE,
                PropertyIdentifier::SILENCED,
                PropertyIdentifier::OPERATION_EXPECTED,
            ]
        );
        assert_eq!(read_event_state(&point), 0);
        assert_in_alarm_clear(&point);
    }
}

#[test]
fn zone_resets_leave_event_state_and_in_alarm_untouched() {
    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
        zone.set_present_value(LifeSafetyState::ALARM.to_raw());
        zone.set_silenced(SilencedState::ALL_SILENCED);
        zone.set_operation_expected(operation);
        zone.set_reset_executor(Arc::new(|_| {
            Ok(LifeSafetyZoneResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                silenced: Some(SilencedState::UNSILENCED),
            })
        }));
        assert_eq!(read_event_state(&zone), 0);
        assert_in_alarm_clear(&zone);

        let outcome = zone.apply_life_safety_operation(operation).unwrap();

        assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
        assert_eq!(
            outcome.changed_properties,
            vec![
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::SILENCED,
                PropertyIdentifier::OPERATION_EXPECTED,
            ]
        );
        assert_eq!(read_event_state(&zone), 0);
        assert_in_alarm_clear(&zone);
        assert!(
            !outcome
                .changed_properties
                .contains(&PropertyIdentifier::TRACKING_VALUE),
            "zone must never invent TRACKING_VALUE"
        );
    }
}

#[test]
fn point_silence_and_unsilence_report_only_actual_silenced_and_expected_deltas() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);

    let outcome = point
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        .unwrap();

    assert_eq!(outcome.effect, LifeSafetyOperationEffect::Applied);
    assert_eq!(
        outcome.changed_properties,
        vec![
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
        ]
    );

    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let outcome = point
        .apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        .unwrap();
    assert_eq!(
        outcome.changed_properties,
        vec![PropertyIdentifier::OPERATION_EXPECTED]
    );

    point.set_operation_expected(LifeSafetyOperation::UNSILENCE);
    let outcome = point
        .apply_life_safety_operation(LifeSafetyOperation::UNSILENCE)
        .unwrap();
    assert_eq!(
        outcome.changed_properties,
        vec![
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
        ]
    );
}
