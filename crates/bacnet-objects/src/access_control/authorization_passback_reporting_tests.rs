//! PASSBACK exemption removes enforcement, not established SOFT reporting.
use super::authorization_test_support::*;
use super::*;
use bacnet_types::enums::AccessPassbackMode as Passback;

fn soft_violation() -> AuthorizationPassback {
    AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Known(Passback::SOFT_PASSBACK),
        fallback_timeout_minutes: None,
        observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
            presentation: original(),
            zone: zone(),
            violation: true,
            last_successful_entry: None,
        }),
    }
}
fn exempt_fixture() -> crate::database::ObjectDatabase {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    exempt(&mut db, 1, &[Exemption::PASSBACK]);
    db
}
fn exempt_check(result: &AuthorizationEvaluation) {
    assert!(result.checks.iter().any(|check| {
        check.stage == AuthorizationStage::Criterion(AuthorizationCheck::Passback)
            && check.outcome == AuthorizationCheckOutcome::Exempt(Exemption::PASSBACK)
    }));
}
#[test]
fn exempt_known_soft_detection_precedes_grant() {
    let db = exempt_fixture();
    let mut req = request();
    req.passback = soft_violation();
    let before = snapshot(&db);
    let result = eval(&db, req);
    granted(&result);
    exempt_check(&result);
    assert_eq!(result.passback_reporting_problem, None);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::GRANTED]
    );
    assert!(
        result
            .events
            .iter()
            .all(|event| event.original == original() && event.presentation == original())
    );
    assert_eq!(eval(&db, req), result);
    assert_eq!(snapshot(&db), before);
}
#[test]
fn exempt_known_soft_detection_precedes_later_lockout_denial() {
    let db = exempt_fixture();
    let mut req = request();
    req.passback = soft_violation();
    req.fallback_lockout = AuthorizationFact::Known(true);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    exempt_check(&result);
    assert_eq!(result.passback_reporting_problem, None);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::DENIED_LOCKOUT]
    );
    assert!(
        result
            .events
            .iter()
            .all(|event| event.original == original() && event.presentation == original())
    );
}

fn known_observation() -> AuthorizationPassbackObservation {
    AuthorizationPassbackObservation {
        presentation: original(),
        zone: zone(),
        violation: true,
        last_successful_entry: None,
    }
}
fn configuration(
    mode: Passback,
    timeout: Option<u32>,
    observation: AuthorizationFact<AuthorizationPassbackObservation>,
) -> AuthorizationPassback {
    AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Known(mode),
        fallback_timeout_minutes: timeout,
        observation,
    }
}
fn diagnostic_only(result: &AuthorizationEvaluation, expected: AuthorizationProblem) {
    granted(result);
    exempt_check(result);
    assert_eq!(events(result), [AccessEvent::GRANTED]);
    assert_eq!(result.passback_reporting_problem, Some(expected));
}
fn no_detection_or_problem(result: &AuthorizationEvaluation) {
    granted(result);
    exempt_check(result);
    assert_eq!(events(result), [AccessEvent::GRANTED]);
    assert_eq!(result.passback_reporting_problem, None);
}
#[test]
fn exempt_unknown_context_or_observation_is_a_typed_optional_diagnostic() {
    let db = exempt_fixture();
    let mut req = request();
    req.passback = AuthorizationPassback::Unknown;
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::MissingFact("passback applicability"),
    );
    req.passback = configuration(Passback::SOFT_PASSBACK, None, AuthorizationFact::Unknown);
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::MissingFact("passback violation observation"),
    );
    req.passback = AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Unknown,
        fallback_timeout_minutes: None,
        observation: AuthorizationFact::Known(known_observation()),
    };
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::MissingFact("passback mode"),
    );
}
#[test]
fn exempt_wrong_tuple_or_zone_cannot_create_a_detection() {
    let db = exempt_fixture();
    for field in 0..5 {
        let mut observation = known_observation();
        match field {
            0 => observation.presentation.sequence += 1,
            1 => observation.presentation.credential = credential(2),
            2 => observation.presentation.point = oid(ObjectType::ACCESS_POINT, 2),
            3 => observation.presentation.at += Duration::from_secs(1),
            _ => observation.zone = oid(ObjectType::ACCESS_ZONE, 2),
        }
        let mut req = request();
        req.passback = configuration(
            Passback::SOFT_PASSBACK,
            None,
            AuthorizationFact::Known(observation),
        );
        diagnostic_only(
            &eval(&db, req),
            AuthorizationProblem::Context(
                "passback observation belongs to another presentation or zone",
            ),
        );
    }
}
#[test]
fn exempt_nonzero_timeout_needs_entry_evidence_for_detection_but_not_permission() {
    let db = exempt_fixture();
    let mut req = request();
    req.passback = configuration(
        Passback::SOFT_PASSBACK,
        Some(1),
        AuthorizationFact::Known(known_observation()),
    );
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::MissingFact("last successful entry for passback expiry"),
    );
    let future = AuthorizationPassbackObservation {
        last_successful_entry: Some(Duration::from_secs(101)),
        ..known_observation()
    };
    req.passback = configuration(
        Passback::SOFT_PASSBACK,
        Some(1),
        AuthorizationFact::Known(future),
    );
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::Context("successful entry is after the presentation"),
    );
}
#[test]
fn exempt_failed_mode_or_timeout_reads_never_use_reporting_fallback() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    for property in [P::PASSBACK_MODE, P::PASSBACK_TIMEOUT] {
        let malformed = if property == P::PASSBACK_MODE {
            "expected passback enumeration"
        } else {
            "expected Unsigned timeout minutes"
        };
        for (read, advertised, failure) in [
            (
                Read::Value(PropertyValue::Null),
                false,
                AuthorizationReadFailure::Malformed(malformed),
            ),
            (
                Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
                false,
                AuthorizationReadFailure::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::READ_ACCESS_DENIED,
                },
            ),
            (
                Read::Failure(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
                true,
                AuthorizationReadFailure::Protocol {
                    class: ErrorClass::PROPERTY,
                    code: ErrorCode::UNKNOWN_PROPERTY,
                },
            ),
            (
                Read::Failure(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                false,
                AuthorizationReadFailure::Protocol {
                    class: ErrorClass::OBJECT,
                    code: ErrorCode::UNKNOWN_OBJECT,
                },
            ),
        ] {
            let mut db = exempt_fixture();
            override_read(&mut db, zone(), property, read, advertised);
            let mut req = request();
            req.passback = soft_violation();
            diagnostic_only(
                &eval(&db, req),
                AuthorizationProblem::Property {
                    object: zone(),
                    property,
                    failure,
                },
            );
        }
    }
}
#[test]
fn exempt_unsupported_mode_or_timeout_is_diagnostic_without_detection() {
    let mut db = exempt_fixture();
    let mut req = request();
    req.passback = soft_violation();
    value(
        &mut db,
        zone(),
        P::PASSBACK_MODE,
        PropertyValue::Enumerated(64),
    );
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::UnsupportedValue {
            object: zone(),
            property: P::PASSBACK_MODE,
            value: 64,
        },
    );
    value(
        &mut db,
        zone(),
        P::PASSBACK_MODE,
        PropertyValue::Enumerated(Passback::SOFT_PASSBACK.to_raw()),
    );
    value(
        &mut db,
        zone(),
        P::PASSBACK_TIMEOUT,
        PropertyValue::Unsigned(u64::MAX),
    );
    diagnostic_only(
        &eval(&db, req),
        AuthorizationProblem::UnsupportedValue {
            object: zone(),
            property: P::PASSBACK_TIMEOUT,
            value: u64::MAX,
        },
    );
}
#[test]
fn exempt_hard_and_off_need_no_observation_timeout_or_invented_event() {
    let mut db = exempt_fixture();
    override_read(&mut db, zone(), P::PASSBACK_TIMEOUT, Read::Forbidden, false);
    for mode in [Passback::HARD_PASSBACK, Passback::PASSBACK_OFF] {
        for observation in [
            AuthorizationFact::Unknown,
            AuthorizationFact::Known(known_observation()),
        ] {
            let mut req = request();
            req.passback = configuration(mode, None, observation);
            no_detection_or_problem(&eval(&db, req));
        }
    }
}
#[test]
fn exempt_inapplicable_or_matching_nonviolation_needs_no_expiry_evidence() {
    let mut db = exempt_fixture();
    override_read(&mut db, zone(), P::PASSBACK_TIMEOUT, Read::Forbidden, false);
    let mut req = request();
    no_detection_or_problem(&eval(&db, req));
    req.passback = configuration(
        Passback::SOFT_PASSBACK,
        Some(1),
        AuthorizationFact::Known(AuthorizationPassbackObservation {
            violation: false,
            ..known_observation()
        }),
    );
    no_detection_or_problem(&eval(&db, req));
}
#[test]
fn exempt_soft_timeout_boundary_and_served_values_control_detection() {
    let mut db = exempt_fixture();
    let observation = AuthorizationPassbackObservation {
        last_successful_entry: Some(Duration::from_secs(50)),
        ..known_observation()
    };
    let mut req = request();
    req.passback = configuration(
        Passback::SOFT_PASSBACK,
        Some(1),
        AuthorizationFact::Known(observation),
    );
    req.now = Duration::from_secs(109);
    assert_eq!(
        events(&eval(&db, req)),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::GRANTED]
    );
    for now in [110, 111] {
        req.now = Duration::from_secs(now);
        no_detection_or_problem(&eval(&db, req));
    }
    value(
        &mut db,
        zone(),
        P::PASSBACK_TIMEOUT,
        PropertyValue::Unsigned(2),
    );
    let result = eval(&db, req);
    granted(&result);
    exempt_check(&result);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::GRANTED]
    );
    assert_eq!(result.passback_reporting_problem, None);
    value(
        &mut db,
        zone(),
        P::PASSBACK_MODE,
        PropertyValue::Enumerated(Passback::HARD_PASSBACK.to_raw()),
    );
    no_detection_or_problem(&eval(&db, req));
}
#[test]
fn optional_reporting_problem_neither_hides_later_denial_nor_resolves_required_unknown() {
    let db = exempt_fixture();
    let mut req = request();
    req.passback = configuration(Passback::SOFT_PASSBACK, None, AuthorizationFact::Unknown);
    req.fallback_lockout = AuthorizationFact::Known(true);
    let denied_result = eval(&db, req);
    denied(&denied_result, AccessEvent::DENIED_LOCKOUT);
    exempt_check(&denied_result);
    assert_eq!(events(&denied_result), [AccessEvent::DENIED_LOCKOUT]);
    assert_eq!(
        denied_result.passback_reporting_problem,
        Some(AuthorizationProblem::MissingFact(
            "passback violation observation"
        ))
    );
    req.fallback_lockout = AuthorizationFact::Unknown;
    let result = eval(&db, req);
    unresolved(&result);
    exempt_check(&result);
    assert!(result.events.is_empty());
    assert_eq!(
        result.passback_reporting_problem,
        denied_result.passback_reporting_problem
    );
    assert!(matches!(
        result.outcome,
        AuthorizationOutcome::Unresolved {
            stage: AuthorizationStage::Criterion(AuthorizationCheck::Lockout),
            problem: AuthorizationProblem::MissingFact("point lockout"),
        }
    ));
}
#[test]
fn earlier_denial_skips_exempt_reporting_and_its_optional_reads() {
    let mut db = exempt_fixture();
    override_read(&mut db, zone(), P::PASSBACK_MODE, Read::Forbidden, false);
    let mut req = request();
    req.passback = soft_violation();
    req.fallback_lockout = AuthorizationFact::Known(true);
    let mut checks = CHECKS;
    checks.swap(4, 5);
    req.order = AuthorizationOrder::new(checks).unwrap();
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    assert_eq!(events(&result), [AccessEvent::DENIED_LOCKOUT]);
    assert_eq!(result.passback_reporting_problem, None);
    assert!(!result.checks.iter().any(|check| check.stage == AuthorizationStage::Criterion(AuthorizationCheck::Passback)));
}
