//! `Time_Delay_Normal` on EventEnrollment (property 356, Table 12-14
//! conformance O) and the internal evaluation-state channel the server
//! evaluator drives (#163/#137/#166; ASHRAE 135-2020 Clauses 12.12, 13.3).

use super::super::*;

/// Absent a write, the property reads back the `Event_Parameters`
/// `Time_Delay` — the Clause 13.3 fallback ("it takes on the value of the
/// pTimeDelay parameter"), matching the intrinsic types' read arm.
#[test]
fn time_delay_normal_defaults_to_event_parameters_time_delay() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    // Legacy Opaque parameters carry no Time_Delay: fallback reads 0.
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 12,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    });
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(12),
        "unwritten Time_Delay_Normal reads as the pTimeDelay fallback"
    );
}

#[test]
fn time_delay_normal_write_round_trips_and_is_writable() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert!(ee.is_writable_property(PropertyIdentifier::TIME_DELAY_NORMAL));
    ee.write_property(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        None,
        PropertyValue::Unsigned(45),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(45),
        "a configured value reads back verbatim, no fallback"
    );
    // The inherent setter can restore the absent (fallback) case.
    ee.set_time_delay_normal(None);
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(0),
        "absent again: back to the (empty-params) fallback"
    );
}

#[test]
fn time_delay_normal_in_property_list() {
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert!(
        ee.property_list()
            .contains(&PropertyIdentifier::TIME_DELAY_NORMAL),
        "Table 12-14 lists the property (O): it must be advertised"
    );
}

/// Write refusals keep the Clause 15.9.1.3 pairings and preserve the stored
/// value (mirroring the intrinsic TDN arm in
/// `common::write_generic_event_properties!`).
#[test]
fn time_delay_normal_write_validation() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.write_property(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();

    match ee
        .write_property(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            None,
            PropertyValue::Real(1.5),
            None,
        )
        .unwrap_err()
    {
        bacnet_types::error::Error::Protocol { class, code } => {
            assert_eq!(
                (class, code),
                (
                    bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                    bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE.to_raw() as u32
                ),
                "Unsigned-typed property fed a Real: PROPERTY/INVALID_DATA_TYPE"
            );
        }
        other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
    }
    match ee
        .write_property(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            None,
            PropertyValue::Unsigned(u64::MAX),
            None,
        )
        .unwrap_err()
    {
        bacnet_types::error::Error::Protocol { class, code } => {
            assert_eq!(
                (class, code),
                (
                    bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32,
                    bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32
                ),
                "unrepresentable Unsigned: PROPERTY/VALUE_OUT_OF_RANGE"
            );
        }
        other => panic!("expected PROPERTY/VALUE_OUT_OF_RANGE, got {other:?}"),
    }
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(9),
        "refused writes must not disturb the stored value"
    );
}

/// The evaluation-state round trip: pending countdown, COV baseline, and the
/// last offnormal-causing value are owned by the object and reachable only
/// through the internal channel.
#[test]
fn enrollment_eval_state_round_trip() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    assert_eq!(
        ee.enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default()),
        "construction starts empty"
    );

    let state = EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::HIGH_LIMIT,
            remaining: 4,
            condition: 0,
            params_fingerprint: 0xDEAD_BEEF,
        }),
        cov_baseline: Some(PropertyValue::Real(2.5)),
        last_offnormal_value: Some(7),
    };
    ee.set_enrollment_eval_state_internal(state.clone())
        .unwrap();
    assert_eq!(ee.enrollment_eval_state_internal(), Some(state));

    // Storing a cleared state clears.
    ee.set_enrollment_eval_state_internal(EventEnrollmentEvalState::default())
        .unwrap();
    assert_eq!(
        ee.enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default())
    );
}

/// Clause 13.2.2.1's disable reset covers the evaluation state: "this state
/// machine is not evaluated" — a stale countdown or baseline must not
/// survive into the next enabled period.
#[test]
fn disabling_detection_clears_eval_state_and_refuses_writes() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_enrollment_eval_state_internal(EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::OFFNORMAL,
            remaining: 1,
            condition: 3,
            params_fingerprint: 1,
        }),
        cov_baseline: Some(PropertyValue::Real(1.0)),
        last_offnormal_value: Some(3),
    })
    .unwrap();

    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(
        ee.enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default()),
        "the disable reset clears the evaluation state"
    );

    // And while disabled the internal write paths refuse — the invariant
    // holds by construction, as with set_event_state_internal (#130).
    assert!(ee
        .set_enrollment_eval_state_internal(EventEnrollmentEvalState {
            pending: Some(EventEnrollmentPending {
                state: EventState::OFFNORMAL,
                remaining: 1,
                condition: 0,
                params_fingerprint: 0,
            }),
            ..EventEnrollmentEvalState::default()
        })
        .is_err());
    assert!(ee.set_acked_transitions_internal(0x01, false).is_err());

    // Re-enabling both reopens the channel and evaluates afresh (the first
    // COV sample after re-enable seeds a new baseline, not a transition).
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ee.set_enrollment_eval_state_internal(EventEnrollmentEvalState {
        last_offnormal_value: Some(3),
        ..EventEnrollmentEvalState::default()
    })
    .unwrap();
    assert_eq!(
        ee.enrollment_eval_state_internal()
            .unwrap()
            .last_offnormal_value,
        Some(3)
    );
}

/// Clause 13.2.3's bit maintenance through the internal channel: set and
/// clear per direction, never touching the other bits.
#[test]
fn acked_transitions_internal_set_and_clear() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let read = |ee: &EventEnrollmentObject| match ee
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => bacnet_types::bitstring::unpack_octet(&data, 3),
        other => panic!("BitString expected, got {other:?}"),
    };

    assert_eq!(read(&ee), 0b111);
    ee.set_acked_transitions_internal(0x01, false).unwrap();
    assert_eq!(read(&ee), 0b110, "TO_OFFNORMAL cleared (ack owed)");
    ee.set_acked_transitions_internal(0x04, false).unwrap();
    assert_eq!(read(&ee), 0b010, "TO_NORMAL cleared");
    ee.set_acked_transitions_internal(0x01, true).unwrap();
    assert_eq!(read(&ee), 0b011, "TO_OFFNORMAL re-set (acknowledged)");
}

/// The trait defaults keep custom downstream objects out of the channel:
/// evaluation-state writes and ack maintenance fail closed
/// (`OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`), like `set_event_state_internal`
/// (#130).
#[test]
fn eval_state_trait_defaults_reject() {
    let mut ee = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    assert!(ee.enrollment_eval_state_internal().is_none());
    assert!(ee
        .set_enrollment_eval_state_internal(EventEnrollmentEvalState::default())
        .is_err());
    assert!(ee.set_acked_transitions_internal(0x01, false).is_err());
}
