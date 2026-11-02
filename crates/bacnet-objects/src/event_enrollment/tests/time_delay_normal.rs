//! `Time_Delay_Normal` on EventEnrollment (property 356, Table 12-14
//! conformance O) and the internal evaluation-state channel the server
//! evaluator drives (#163/#137/#166; ASHRAE 135-2020 Clauses 12.12, 13.3).

use super::super::*;
use crate::property_metadata::is_writable_property;
use bacnet_types::error::RemoteError;

/// Absent a write, the property reads back the `Event_Parameters`
/// `Time_Delay` — the Clause 13.3 fallback to pTimeDelay,
/// matching the intrinsic types' read arm.
#[test]
fn time_delay_normal_defaults_to_event_parameters_time_delay() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
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
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    assert!(is_writable_property(
        &ee,
        PropertyIdentifier::TIME_DELAY_NORMAL
    ));
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
    let ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    assert!(
        crate::property_metadata::has_property(
            &ee.property_metadata(),
            PropertyIdentifier::TIME_DELAY_NORMAL
        ),
        "Table 12-14 lists the property (O): it must be advertised"
    );
}

/// Write refusals keep the Clause 15.9.1.3 pairings and preserve the stored
/// value (mirroring the intrinsic TDN arm in
/// `common::write_generic_event_properties!`).
#[test]
fn time_delay_normal_write_validation() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
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
        bacnet_types::error::Error::Remote(RemoteError::Error { class, code }) => {
            assert_eq!(
                (class, code),
                (
                    bacnet_types::enums::ErrorClass::PROPERTY,
                    bacnet_types::enums::ErrorCode::INVALID_DATA_TYPE
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
        bacnet_types::error::Error::Remote(RemoteError::Error { class, code }) => {
            assert_eq!(
                (class, code),
                (
                    bacnet_types::enums::ErrorClass::PROPERTY,
                    bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE
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
#[cfg(feature = "event-reporting")]
#[test]
fn enrollment_eval_state_round_trip() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    assert_eq!(
        ee.enrollment_eval().map(|e| e.eval_state()),
        Some(EventEnrollmentEvalState::default()),
        "construction starts empty"
    );

    let source = (
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        Some(2),
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
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_state(state.clone())
        .unwrap();
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_source(Some(source))
        .unwrap();
    assert_eq!(ee.enrollment_eval().map(|e| e.eval_state()), Some(state));
    assert_eq!(
        ee.enrollment_eval().and_then(|e| e.eval_source()),
        Some(Some(source))
    );

    // Storing a cleared state clears.
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_state(EventEnrollmentEvalState::default())
        .unwrap();
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_source(None)
        .unwrap();
    assert_eq!(
        ee.enrollment_eval().map(|e| e.eval_state()),
        Some(EventEnrollmentEvalState::default())
    );
    assert_eq!(
        ee.enrollment_eval().and_then(|e| e.eval_source()),
        Some(None)
    );
}

#[cfg(feature = "event-reporting")]
#[test]
fn configuration_setters_cancel_pending_countdowns() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let pending = EventEnrollmentPending {
        state: EventState::OFFNORMAL,
        remaining: 2,
        condition: 1,
        params_fingerprint: 2,
    };
    let seed = |ee: &mut EventEnrollmentObject| {
        ee.enrollment_eval_mut()
            .unwrap()
            .set_eval_state(EventEnrollmentEvalState {
                pending: Some(pending.clone()),
                ..Default::default()
            })
            .unwrap();
    };

    seed(&mut ee);
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 2,
        low_limit: 0.0,
        high_limit: 1.0,
        deadband: 0.0,
    });
    assert!(
        ee.enrollment_eval()
            .map(|e| e.eval_state())
            .unwrap()
            .pending
            .is_none()
    );

    seed(&mut ee);
    ee.set_time_delay_normal(Some(3));
    assert!(
        ee.enrollment_eval()
            .map(|e| e.eval_state())
            .unwrap()
            .pending
            .is_none()
    );

    seed(&mut ee);
    ee.set_object_property_reference(None).unwrap();
    assert!(
        ee.enrollment_eval()
            .map(|e| e.eval_state())
            .unwrap()
            .pending
            .is_none()
    );

    seed(&mut ee);
    let before = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap();
    assert!(
        ee.write_property(
            PropertyIdentifier::EVENT_PARAMETERS,
            None,
            PropertyValue::Unsigned(1),
            None
        )
        .is_err()
    );
    assert_eq!(
        ee.read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
            .unwrap(),
        before
    );
    assert_eq!(
        ee.enrollment_eval()
            .map(|e| e.eval_state())
            .unwrap()
            .pending,
        Some(pending)
    );
}

/// Clause 13.2.2.1's disable reset covers the evaluation state: evaluation
/// is suspended, so a stale countdown or baseline must not
/// survive into the next enabled period.
#[cfg(feature = "event-reporting")]
#[test]
fn disabling_detection_clears_eval_state_and_refuses_writes() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let source = (
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        Some(1),
    );
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_source(Some(source))
        .unwrap();
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_state(EventEnrollmentEvalState {
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
        ee.enrollment_eval().map(|e| e.eval_state()),
        Some(EventEnrollmentEvalState::default()),
        "the disable reset clears the evaluation state"
    );
    assert_eq!(
        ee.enrollment_eval().and_then(|e| e.eval_source()),
        Some(None)
    );

    // And while disabled the internal write paths refuse — the invariant
    // holds by construction (#130).
    assert!(
        ee.enrollment_eval_mut()
            .unwrap()
            .set_eval_state(EventEnrollmentEvalState {
                pending: Some(EventEnrollmentPending {
                    state: EventState::OFFNORMAL,
                    remaining: 1,
                    condition: 0,
                    params_fingerprint: 0,
                }),
                ..EventEnrollmentEvalState::default()
            })
            .is_err()
    );
    assert!(
        ee.enrollment_eval_mut()
            .unwrap()
            .set_eval_source(Some(source))
            .is_err()
    );

    // Re-enabling both reopens the channel and evaluates afresh (the first
    // COV sample after re-enable seeds a new baseline, not a transition).
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ee.enrollment_eval_mut()
        .unwrap()
        .set_eval_state(EventEnrollmentEvalState {
            last_offnormal_value: Some(3),
            ..EventEnrollmentEvalState::default()
        })
        .unwrap();
    assert_eq!(
        ee.enrollment_eval()
            .map(|e| e.eval_state())
            .unwrap()
            .last_offnormal_value,
        Some(3)
    );
}

/// An object that lends no evaluation capability keeps no evaluation state:
/// the evaluator then has nothing to snapshot or store (#130).
#[test]
fn eval_state_non_lenders_keep_none() {
    let mut object = crate::averaging::AveragingObject::new(1, "AVG-1").unwrap();
    assert!(object.enrollment_eval().is_none());
    assert!(object.enrollment_eval_mut().is_none());
}
