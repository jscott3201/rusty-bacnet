//! Established passback violations use successful-entry time and exact identity.
use super::authorization_test_support::*;
use super::*;
use bacnet_types::enums::AccessPassbackMode as Passback;

fn passback(
    mode: Passback,
    timeout: Option<u32>,
    violation: bool,
    entry: Option<u64>,
) -> AuthorizationPassback {
    AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Known(mode),
        fallback_timeout_minutes: timeout,
        observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
            presentation: original(),
            zone: zone(),
            violation,
            last_successful_entry: entry.map(Duration::from_secs),
        }),
    }
}
#[test]
fn hard_passback_denies_only_an_established_unexpired_violation() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::HARD_PASSBACK, None, true, None);
    denied(&eval(&db, req), AccessEvent::DENIED_PASSBACK);
    req.passback = passback(Passback::HARD_PASSBACK, None, false, None);
    granted(&eval(&db, req));
}
#[test]
fn soft_passback_emits_detection_before_grant_or_later_denial() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::SOFT_PASSBACK, None, true, None);
    let result = eval(&db, req);
    granted(&result);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::GRANTED]
    );
    req.fallback_lockout = AuthorizationFact::Known(true);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::DENIED_LOCKOUT]
    );
}
#[test]
fn off_needs_no_violation_or_timeout_evidence() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    override_read(&mut db, zone(), P::PASSBACK_TIMEOUT, Read::Forbidden, false);
    let mut req = request();
    req.passback = AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Known(Passback::PASSBACK_OFF),
        fallback_timeout_minutes: None,
        observation: AuthorizationFact::Unknown,
    };
    granted(&eval(&db, req));
}
#[test]
fn timeout_expires_at_minutes_from_successful_entry() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::HARD_PASSBACK, Some(1), true, Some(50));
    req.now = Duration::from_secs(109);
    denied(&eval(&db, req), AccessEvent::DENIED_PASSBACK);
    req.now = Duration::from_secs(110);
    granted(&eval(&db, req));
    req.now = Duration::from_secs(111);
    granted(&eval(&db, req));
    for timeout in [None, Some(0)] {
        req.passback = passback(Passback::HARD_PASSBACK, timeout, true, Some(0));
        req.now = Duration::from_secs(1_000_000);
        denied(&eval(&db, req), AccessEvent::DENIED_PASSBACK);
    }
}
#[test]
fn missing_entry_for_nonzero_timeout_is_not_inferred_from_last_use() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::HARD_PASSBACK, Some(1), true, None);
    unresolved(&eval(&db, req));
    req.passback = passback(Passback::HARD_PASSBACK, Some(1), true, Some(101));
    unresolved(&eval(&db, req));
}
#[test]
fn passback_observation_is_bound_to_every_presentation_member_and_zone() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    for field in 0..5 {
        let mut presentation = original();
        let mut observed_zone = zone();
        match field {
            0 => presentation.sequence += 1,
            1 => presentation.credential = credential(2),
            2 => presentation.point = oid(ObjectType::ACCESS_POINT, 2),
            3 => presentation.at += Duration::from_secs(1),
            _ => observed_zone = oid(ObjectType::ACCESS_ZONE, 2),
        }
        let mut req = request();
        req.passback = AuthorizationPassback::Zone {
            zone: zone(),
            fallback_mode: AuthorizationFact::Known(Passback::HARD_PASSBACK),
            fallback_timeout_minutes: None,
            observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
                presentation,
                zone: observed_zone,
                violation: false,
                last_successful_entry: None,
            }),
        };
        unresolved(&eval(&db, req));
    }
}
#[test]
fn unknown_and_vendor_passback_evidence_is_unresolved() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = AuthorizationPassback::Unknown;
    unresolved(&eval(&db, req));
    for mode in [
        AuthorizationFact::Unknown,
        AuthorizationFact::Known(Passback::from_raw(64)),
        AuthorizationFact::Known(Passback::HARD_PASSBACK),
    ] {
        req.passback = AuthorizationPassback::Zone {
            zone: zone(),
            fallback_mode: mode,
            fallback_timeout_minutes: None,
            observation: AuthorizationFact::Unknown,
        };
        unresolved(&eval(&db, req));
    }
}
#[test]
fn served_passback_mode_and_timeout_win_without_malformed_fallback() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::PASSBACK_OFF, Some(0), true, Some(0));
    value(
        &mut db,
        zone(),
        P::PASSBACK_MODE,
        PropertyValue::Enumerated(Passback::HARD_PASSBACK.to_raw()),
    );
    denied(&eval(&db, req), AccessEvent::DENIED_PASSBACK);
    value(
        &mut db,
        zone(),
        P::PASSBACK_TIMEOUT,
        PropertyValue::Unsigned(1),
    );
    granted(&eval(&db, req));
    value(&mut db, zone(), P::PASSBACK_TIMEOUT, PropertyValue::Null);
    unresolved(&eval(&db, req));
    value(&mut db, zone(), P::PASSBACK_MODE, PropertyValue::Null);
    unresolved(&eval(&db, req));
}
#[test]
fn passback_exemption_only_bypasses_enforcement_and_preserves_soft_reporting() {
    let mut db = database(None);
    add_zone(&mut db, 1, 0, 10);
    let mut req = request();
    req.passback = passback(Passback::HARD_PASSBACK, None, true, None);
    for raw in 1..=6 {
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        denied(&eval(&db, req), AccessEvent::DENIED_PASSBACK);
    }
    exempt(&mut db, 1, &[Exemption::PASSBACK]);
    granted(&eval(&db, req));
    req.passback = passback(Passback::SOFT_PASSBACK, None, true, None);
    assert_eq!(
        events(&eval(&db, req)),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::GRANTED]
    );
    req.fallback_lockout = AuthorizationFact::Known(true);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    assert_eq!(
        events(&result),
        [AccessEvent::PASSBACK_DETECTED, AccessEvent::DENIED_LOCKOUT]
    );
}
