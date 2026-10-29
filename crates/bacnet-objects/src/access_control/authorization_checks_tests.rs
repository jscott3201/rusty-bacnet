//! Threat, lockout, ordering, narrow fallback and read-only guarantees.
use super::authorization_test_support::*;
use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

#[test]
fn absent_credential_authority_means_zero_and_threat_equality_passes() {
    let mut db = database(None);
    let mut req = request();
    req.fallback_threat_level = AuthorizationFact::Known(1);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    value(
        &mut db,
        credential(1),
        P::THREAT_AUTHORITY,
        PropertyValue::Unsigned(1),
    );
    granted(&eval(&db, req));
    value(
        &mut db,
        point(),
        P::THREAT_LEVEL,
        PropertyValue::Unsigned(100),
    );
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    value(
        &mut db,
        credential(1),
        P::THREAT_AUTHORITY,
        PropertyValue::Unsigned(100),
    );
    granted(&eval(&db, req));
}
#[test]
fn unknown_point_threat_and_out_of_range_values_are_unresolved() {
    let mut db = database(None);
    let mut req = request();
    req.fallback_threat_level = AuthorizationFact::Unknown;
    unresolved(&eval(&db, req));
    req.fallback_threat_level = AuthorizationFact::Known(101);
    unresolved(&eval(&db, req));
    value(
        &mut db,
        point(),
        P::THREAT_LEVEL,
        PropertyValue::Unsigned(1),
    );
    for (id, p) in [
        (credential(1), P::THREAT_AUTHORITY),
        (point(), P::THREAT_LEVEL),
    ] {
        value(&mut db, id, p, PropertyValue::Unsigned(101));
        unresolved(&eval(&db, request()));
        value(&mut db, id, p, PropertyValue::Unsigned(0));
    }
}
#[test]
fn no_standard_exemption_bypasses_threat() {
    for raw in 0..=6 {
        let mut db = database(None);
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        let mut req = request();
        req.fallback_threat_level = AuthorizationFact::Known(1);
        denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    }
}
#[test]
fn lockout_exemption_only_bypasses_point_lockout() {
    let mut db = database(None);
    let mut req = request();
    req.fallback_lockout = AuthorizationFact::Known(true);
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
    exempt(&mut db, 1, &[Exemption::LOCKOUT]);
    granted(&eval(&db, req));
    assert!(
        eval(&db, req)
            .checks
            .iter()
            .any(|check| check.outcome == AuthorizationCheckOutcome::Exempt(Exemption::LOCKOUT))
    );
    reasons(&mut db, 1, &[Reason::DISABLED_LOCKOUT]);
    denied(&eval(&db, req), AccessEvent::DENIED_CREDENTIAL_LOCKOUT);
}
#[test]
fn unrelated_exemptions_do_not_remove_point_lockout() {
    for raw in [0, 1, 2, 4, 5, 6] {
        let mut db = database(None);
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        let mut req = request();
        req.fallback_lockout = AuthorizationFact::Known(true);
        denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
    }
}
#[test]
fn explicit_order_selects_the_first_of_two_known_failures() {
    let db = database(None);
    let mut req = request();
    req.fallback_threat_level = AuthorizationFact::Known(1);
    req.fallback_lockout = AuthorizationFact::Known(true);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    let mut checks = CHECKS;
    checks.swap(2, 5);
    req.order = AuthorizationOrder::new(checks).unwrap();
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
}
#[test]
fn earlier_unknown_stops_before_later_known_denial_and_exemption_reveals_it() {
    let mut db = database(None);
    let mut req = request();
    req.fallback_lockout = AuthorizationFact::Unknown;
    req.fallback_threat_level = AuthorizationFact::Known(1);
    let mut checks = CHECKS;
    checks.swap(2, 5);
    req.order = AuthorizationOrder::new(checks).unwrap();
    let result = eval(&db, req);
    unresolved(&result);
    assert_eq!(
        result.checks.last().unwrap().stage,
        AuthorizationStage::Criterion(AuthorizationCheck::Lockout)
    );
    assert!(
        !result
            .checks
            .iter()
            .any(|c| c.stage == AuthorizationStage::Criterion(AuthorizationCheck::Threat))
    );
    exempt(&mut db, 1, &[Exemption::LOCKOUT]);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
}
#[test]
fn served_property_wins_and_only_genuine_absence_uses_fallback() {
    let mut db = database(None);
    let mut req = request();
    req.fallback_lockout = AuthorizationFact::Known(false);
    value(&mut db, point(), P::LOCKOUT, PropertyValue::Boolean(true));
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
    for (read, advertised) in [
        (Read::Value(PropertyValue::Null), false),
        (
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
            false,
        ),
        (
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            true,
        ),
        (
            Read::Failure(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
            false,
        ),
    ] {
        override_read(&mut db, point(), P::LOCKOUT, read, advertised);
        unresolved(&eval(&db, req));
    }
    override_read(
        &mut db,
        point(),
        P::LOCKOUT,
        Read::Failure(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        false,
    );
    granted(&eval(&db, req));
}
#[test]
fn property_protocol_failure_preserves_class_and_code() {
    let mut db = database(None);
    override_read(
        &mut db,
        point(),
        P::THREAT_LEVEL,
        Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
        false,
    );
    let result = eval(&db, request());
    unresolved(&result);
    assert!(matches!(result.outcome, AuthorizationOutcome::Unresolved {
        problem: AuthorizationProblem::Property { object, property: P::THREAT_LEVEL,
            failure: AuthorizationReadFailure::Protocol { class: ErrorClass::PROPERTY, code: ErrorCode::READ_ACCESS_DENIED } }, ..
    } if object == point()));
}
#[test]
fn malformed_unknown_and_duplicate_exemptions_are_not_silently_ignored() {
    for exemptions in [
        PropertyValue::Null,
        PropertyValue::List(vec![PropertyValue::Boolean(true)]),
        PropertyValue::List(vec![PropertyValue::Enumerated(64)]),
        PropertyValue::List(vec![
            PropertyValue::Enumerated(2),
            PropertyValue::Enumerated(2),
        ]),
    ] {
        let mut db = database(None);
        value(
            &mut db,
            credential(1),
            P::AUTHORIZATION_EXEMPTIONS,
            exemptions,
        );
        unresolved(&eval(&db, request()));
    }
}
#[test]
fn evaluation_is_repeatable_and_leaves_all_served_state_unchanged() {
    let mut db = database(None);
    add_zone(&mut db, 4, 1, 10);
    // This wrapper panics on any attempted property write; snapshot also covers
    // ordinary objects, including point event metadata and occupancy state.
    value(&mut db, point(), P::LOCKOUT, PropertyValue::Boolean(false));
    let before = snapshot(&db);
    let first = eval(&db, request());
    granted(&first);
    assert_eq!(eval(&db, request()), first);
    assert_eq!(snapshot(&db), before);
    let mut req = request();
    req.fallback_threat_level = AuthorizationFact::Known(1);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    assert_eq!(snapshot(&db), before);
}
#[test]
fn invalid_original_identifiers_error_and_future_presentation_is_unresolved() {
    let db = database(None);
    let mut req = request();
    req.original.credential = rights(1);
    assert!(evaluate_authorization(&db, req).is_err());
    req = request();
    req.original.point = oid(ObjectType::ACCESS_POINT, 99);
    assert!(evaluate_authorization(&db, req).is_err());
    req = request();
    req.now = Duration::from_secs(99);
    unresolved(&eval(&db, req));
}

#[test]
fn point_threat_zero_disables_comparison_without_requiring_credential_authority() {
    let mut db = database(None);
    override_read(
        &mut db,
        credential(1),
        P::THREAT_AUTHORITY,
        Read::Forbidden,
        false,
    );
    granted(&eval(&db, request()));
}
