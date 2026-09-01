use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState, PropertyIdentifier, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::traits::{BACnetObject, LifeSafetyOperationEffect};

use super::*;

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> u32 {
    match object.read_property(property, None).unwrap() {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected enumerated value, got {other:?}"),
    }
}

fn point_state(point: &LifeSafetyPointObject) -> (u32, u32, u32, u32) {
    (
        read(point, PropertyIdentifier::PRESENT_VALUE),
        read(point, PropertyIdentifier::TRACKING_VALUE),
        read(point, PropertyIdentifier::SILENCED),
        read(point, PropertyIdentifier::OPERATION_EXPECTED),
    )
}

fn zone_state(zone: &LifeSafetyZoneObject) -> (u32, u32, u32) {
    (
        read(zone, PropertyIdentifier::PRESENT_VALUE),
        read(zone, PropertyIdentifier::SILENCED),
        read(zone, PropertyIdentifier::OPERATION_EXPECTED),
    )
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn point_reset_variants_receive_exact_context_and_commit_atomically() {
    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let mut point = LifeSafetyPointObject::new(operation.to_raw(), "point").unwrap();
        let oid = point.object_identifier();
        point.set_present_value(LifeSafetyState::ALARM.to_raw());
        point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
        point.set_silenced(SilencedState::ALL_SILENCED);
        point.set_operation_expected(operation);
        point.set_reset_executor(Arc::new(move |context| {
            assert_eq!(
                *context,
                LifeSafetyPointResetContext {
                    object_identifier: oid,
                    operation,
                    present_value: LifeSafetyState::ALARM,
                    tracking_value: LifeSafetyState::FAULT,
                    silenced: SilencedState::ALL_SILENCED,
                    operation_expected: operation,
                }
            );
            Ok(LifeSafetyPointResetCommit {
                present_value: Some(LifeSafetyState::ACTIVE),
                tracking_value: Some(LifeSafetyState::PRE_ALARM),
                silenced: Some(SilencedState::AUDIBLE_SILENCED),
            })
        }));

        assert_eq!(
            point.apply_life_safety_operation(operation).unwrap(),
            LifeSafetyOperationEffect::Applied
        );
        assert_eq!(
            point_state(&point),
            (
                LifeSafetyState::ACTIVE.to_raw(),
                LifeSafetyState::PRE_ALARM.to_raw(),
                SilencedState::AUDIBLE_SILENCED.to_raw(),
                LifeSafetyOperation::NONE.to_raw(),
            )
        );
    }
}

#[test]
fn point_reset_detailed_outcome_reports_exact_committed_deltas() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
    point.set_silenced(SilencedState::ALL_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            tracking_value: Some(LifeSafetyState::QUIET),
            silenced: Some(SilencedState::UNSILENCED),
        })
    }));

    let outcome = point
        .apply_life_safety_operation_detailed(LifeSafetyOperation::RESET)
        .unwrap();

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
}

#[test]
fn zone_reset_variants_receive_exact_context_and_commit_atomically() {
    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let mut zone = LifeSafetyZoneObject::new(operation.to_raw(), "zone").unwrap();
        let oid = zone.object_identifier();
        zone.set_present_value(LifeSafetyState::FAULT_ALARM.to_raw());
        zone.set_silenced(SilencedState::VISIBLE_SILENCED);
        zone.set_operation_expected(operation);
        zone.set_reset_executor(Arc::new(move |context| {
            assert_eq!(
                *context,
                LifeSafetyZoneResetContext {
                    object_identifier: oid,
                    operation,
                    present_value: LifeSafetyState::FAULT_ALARM,
                    silenced: SilencedState::VISIBLE_SILENCED,
                    operation_expected: operation,
                }
            );
            Ok(LifeSafetyZoneResetCommit {
                present_value: Some(LifeSafetyState::SUPERVISORY),
                silenced: Some(SilencedState::UNSILENCED),
            })
        }));

        assert_eq!(
            zone.apply_life_safety_operation(operation).unwrap(),
            LifeSafetyOperationEffect::Applied
        );
        assert_eq!(
            zone_state(&zone),
            (
                LifeSafetyState::SUPERVISORY.to_raw(),
                SilencedState::UNSILENCED.to_raw(),
                LifeSafetyOperation::NONE.to_raw(),
            )
        );
    }
}

#[test]
fn no_delta_reset_success_preserves_values_and_clears_expected_operation() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
    point.set_silenced(SilencedState::ALL_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(|_| Ok(LifeSafetyPointResetCommit::default())));
    let before = point_state(&point);

    point
        .apply_life_safety_operation(LifeSafetyOperation::RESET)
        .unwrap();

    let after = point_state(&point);
    assert_eq!((after.0, after.1, after.2), (before.0, before.1, before.2));
    assert_eq!(
        read(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );
}

#[test]
fn same_value_reset_commit_reports_only_expected_operation() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
    point.set_silenced(SilencedState::ALL_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::ALARM),
            tracking_value: Some(LifeSafetyState::FAULT),
            silenced: Some(SilencedState::ALL_SILENCED),
        })
    }));

    let outcome = point
        .apply_life_safety_operation_detailed(LifeSafetyOperation::RESET)
        .unwrap();

    assert_eq!(
        outcome.changed_properties,
        vec![PropertyIdentifier::OPERATION_EXPECTED]
    );
}

#[test]
fn zone_reset_detailed_outcome_never_invents_tracking_value() {
    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_present_value(LifeSafetyState::ALARM.to_raw());
    zone.set_silenced(SilencedState::ALL_SILENCED);
    zone.set_operation_expected(LifeSafetyOperation::RESET);
    zone.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyZoneResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            silenced: Some(SilencedState::UNSILENCED),
        })
    }));

    let outcome = zone
        .apply_life_safety_operation_detailed(LifeSafetyOperation::RESET)
        .unwrap();

    assert_eq!(
        outcome.changed_properties,
        vec![
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
        ]
    );
    assert!(!outcome
        .changed_properties
        .contains(&PropertyIdentifier::TRACKING_VALUE));
}

#[test]
fn wrong_expected_reset_never_calls_executor_or_changes_state() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
    point.set_silenced(SilencedState::ALL_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::RESET_FAULT);
    point.set_reset_executor(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::AcqRel);
        Ok(LifeSafetyPointResetCommit::default())
    }));
    let before = point_state(&point);

    let error = point
        .apply_life_safety_operation(LifeSafetyOperation::RESET_ALARM)
        .unwrap_err();

    assert_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
    assert_eq!(calls.load(Ordering::Acquire), 0);
    assert_eq!(point_state(&point), before);
}

#[test]
fn missing_point_and_zone_executors_fail_without_mutation() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_operation_expected(LifeSafetyOperation::RESET);
    let point_before = point_state(&point);
    assert_error(
        point
            .apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(point_state(&point), point_before);

    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_present_value(LifeSafetyState::FAULT.to_raw());
    zone.set_operation_expected(LifeSafetyOperation::RESET_FAULT);
    let zone_before = zone_state(&zone);
    assert_error(
        zone.apply_life_safety_operation(LifeSafetyOperation::RESET_FAULT)
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(zone_state(&zone), zone_before);
}

#[test]
fn executor_failures_and_panics_map_exactly_without_mutation() {
    let cases = [
        (
            LifeSafetyResetError::UnsupportedVariant,
            ErrorClass::OBJECT,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            LifeSafetyResetError::InvalidOperationInThisState,
            ErrorClass::OBJECT,
            ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
        ),
        (
            LifeSafetyResetError::ServiceRequestDenied,
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED,
        ),
    ];
    for (executor_error, class, code) in cases {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_present_value(LifeSafetyState::ALARM.to_raw());
        point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
        point.set_silenced(SilencedState::ALL_SILENCED);
        point.set_operation_expected(LifeSafetyOperation::RESET);
        point.set_reset_executor(Arc::new(move |_| Err(executor_error)));
        let before = point_state(&point);

        assert_error(
            point
                .apply_life_safety_operation(LifeSafetyOperation::RESET)
                .unwrap_err(),
            class,
            code,
        );
        assert_eq!(point_state(&point), before);
    }

    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_present_value(LifeSafetyState::ALARM.to_raw());
    zone.set_operation_expected(LifeSafetyOperation::RESET);
    zone.set_reset_executor(Arc::new(|_| panic!("executor bug")));
    let before = zone_state(&zone);
    assert_error(
        zone.apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap_err(),
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
    assert_eq!(zone_state(&zone), before);
}

#[test]
fn invalid_commits_cannot_partially_mutate_point_or_zone() {
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM.to_raw());
    point.set_tracking_value(LifeSafetyState::FAULT.to_raw());
    point.set_silenced(SilencedState::ALL_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            tracking_value: Some(LifeSafetyState::from_raw(35)),
            silenced: Some(SilencedState::UNSILENCED),
        })
    }));
    let before = point_state(&point);
    assert_error(
        point
            .apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(point_state(&point), before);

    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_present_value(LifeSafetyState::FAULT.to_raw());
    zone.set_silenced(SilencedState::ALL_SILENCED);
    zone.set_operation_expected(LifeSafetyOperation::RESET);
    zone.set_reset_executor(Arc::new(|_| {
        Ok(LifeSafetyZoneResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            silenced: Some(SilencedState::from_raw(4)),
        })
    }));
    let before = zone_state(&zone);
    assert_error(
        zone.apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(zone_state(&zone), before);
}

#[test]
fn trusted_local_rearm_executes_two_reset_cycles() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let mut point = LifeSafetyPointObject::new(1, "point")
        .unwrap()
        .with_reset_executor(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            Ok(LifeSafetyPointResetCommit::default())
        }));

    for _ in 0..2 {
        point.set_operation_expected(LifeSafetyOperation::RESET);
        point
            .apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap();
    }

    assert_eq!(calls.load(Ordering::Acquire), 2);
    assert_eq!(
        read(&point, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::NONE.to_raw()
    );
}
