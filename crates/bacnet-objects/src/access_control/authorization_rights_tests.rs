//! Original rights decisions, diagnostics and selected companion requirement.
use super::authorization_test_support::*;
use super::*;

#[test]
fn access_rights_exemption_skips_only_assigned_rights_and_selected_accompaniment() {
    let mut db = database(Some(credential(2).into()));
    set_assignments(&mut db, 1, vec![], true);
    override_read(
        &mut db,
        credential(1),
        P::ASSIGNED_ACCESS_RIGHTS,
        Read::Forbidden,
        true,
    );
    let result = eval(&db, request());
    granted(&result);
    assert_eq!(result.rights, None);
    assert_eq!(result.accompaniment, None);
    let mut req = request();
    req.fallback_threat_level = AuthorizationFact::Known(1);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    req.fallback_threat_level = AuthorizationFact::Known(0);
    req.fallback_lockout = AuthorizationFact::Known(true);
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
}
#[test]
fn other_exemptions_do_not_grant_missing_rights() {
    for raw in [0, 1, 3, 4, 5, 6] {
        let mut db = database(None);
        set_assignments(&mut db, 1, vec![], false);
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        denied(&eval(&db, request()), AccessEvent::DENIED_NO_ACCESS_RIGHTS);
    }
}
#[test]
fn original_excluded_assignments_follow_explicit_policy_and_remain_diagnostic() {
    let mut db = database(None);
    set_assignments(
        &mut db,
        1,
        vec![assignment(rights(99), true), assignment(rights(1), true)],
        false,
    );
    let mut req = request();
    let honor = eval(&db, req);
    granted(&honor);
    assert_eq!(honor.rights.as_ref().unwrap().unresolved.len(), 1);
    req.rights_policy = AuthorizationRightsPolicy::RequireCompleteLocalEvidence;
    let strict = eval(&db, req);
    unresolved(&strict);
    assert_eq!(strict.rights, honor.rights);
    assert!(matches!(strict.outcome, AuthorizationOutcome::Unresolved {
        problem: AuthorizationProblem::ExcludedRights { presentation }, ..
    } if presentation == original()));
    assert!(strict.accompaniment.is_none());
    assert!(strict.events.is_empty());
}
#[test]
fn explicit_completeness_policy_also_precedes_excluded_only_denial() {
    let mut db = database(None);
    set_assignments(&mut db, 1, vec![assignment(rights(99), true)], false);
    denied(&eval(&db, request()), AccessEvent::DENIED_NO_ACCESS_RIGHTS);
    let mut req = request();
    req.rights_policy = AuthorizationRightsPolicy::RequireCompleteLocalEvidence;
    unresolved(&eval(&db, req));
}
#[test]
fn granted_rights_does_not_override_later_known_denial() {
    let db = database(None);
    let mut req = request();
    req.fallback_lockout = AuthorizationFact::Known(true);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    assert!(matches!(
        result.rights.unwrap().decision,
        AccessRightsDecision::Granted { .. }
    ));
}
#[test]
fn selected_first_rule_cannot_be_evaded_by_a_later_unaccompanied_grant() {
    let mut db = database(Some(credential(2).into()));
    set_assignments(
        &mut db,
        1,
        vec![assignment(rights(1), true), assignment(rights(3), true)],
        false,
    );
    let mut req = request();
    let missing = eval(&db, req);
    unresolved(&missing);
    assert_eq!(
        missing.accompaniment.unwrap().requirement,
        Some(credential(2).into())
    );
    let waiting = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Waiting,
    }];
    req.accompaniment_witnesses = &waiting;
    assert_eq!(
        eval(&db, req).outcome,
        AuthorizationOutcome::Pending(AuthorizationStage::Criterion(
            AuthorizationCheck::AccessRights
        ))
    );
    req.now = Duration::from_secs(111);
    denied(&eval(&db, req), AccessEvent::DENIED_NO_ACCOMPANIMENT);
}
#[test]
fn accompaniment_waiting_unknown_and_link_limit_never_grant() {
    let db = database(Some(credential(2).into()));
    let mut req = request();
    unresolved(&eval(&db, req));
    req.max_accompaniment_links = 0;
    unresolved(&eval(&db, req));
    let waiting = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Waiting,
    }];
    req.max_accompaniment_links = 1;
    req.accompaniment_witnesses = &waiting;
    let result = eval(&db, req);
    assert_eq!(
        result.outcome,
        AuthorizationOutcome::Pending(AuthorizationStage::Criterion(
            AuthorizationCheck::AccessRights
        ))
    );
    assert!(result.events.is_empty());
}
#[test]
fn by_retains_the_companion_and_precedes_later_root_denial() {
    let db = database(Some(credential(2).into()));
    let witness = [presented(2)];
    let authenticated = [CompanionAuthenticationAttestation {
        presentation: companion(2),
        authentication: CompanionAuthentication::Authenticated,
    }];
    let mut req = request();
    req.accompaniment_witnesses = &witness;
    req.companion_authentication = &authenticated;
    req.fallback_lockout = AuthorizationFact::Known(true);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_LOCKOUT);
    assert_eq!(
        events(&result),
        [AccessEvent::ACCOMPANIMENT_BY, AccessEvent::DENIED_LOCKOUT]
    );
    assert_eq!(result.events[0].original, original());
    assert_eq!(result.events[0].presentation, companion(2));
}
#[test]
fn soft_by_phase_and_terminal_intents_stay_bounded_and_in_check_order() {
    let mut db = database(Some(credential(2).into()));
    add_zone(&mut db, 1, 0, 10);
    mode(&mut db, Mode::VERIFICATION_REQUIRED);
    let witness = [presented(2)];
    let authenticated = [CompanionAuthenticationAttestation {
        presentation: companion(2),
        authentication: CompanionAuthentication::Authenticated,
    }];
    let mut req = request();
    req.accompaniment_witnesses = &witness;
    req.companion_authentication = &authenticated;
    let mut checks = CHECKS;
    checks.swap(1, 4);
    req.order = AuthorizationOrder::new(checks).unwrap();
    req.passback = AuthorizationPassback::Zone {
        zone: zone(),
        fallback_mode: AuthorizationFact::Known(
            bacnet_types::enums::AccessPassbackMode::SOFT_PASSBACK,
        ),
        fallback_timeout_minutes: None,
        observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
            presentation: original(),
            zone: zone(),
            violation: true,
            last_successful_entry: None,
        }),
    };
    req.external_phase = AuthorizationExternalPhase::Observed {
        presentation: original(),
        started: original().at,
        fallback_period: AuthorizationFact::Known(Duration::from_secs(10)),
        deadline: AccompanimentDeadline::Inclusive,
        verdict: AuthorizationVerdict::Rejected(req.now),
    };
    let before = snapshot(&db);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_VERIFICATION_FAILED);
    assert_eq!(
        events(&result),
        [
            AccessEvent::PASSBACK_DETECTED,
            AccessEvent::ACCOMPANIMENT_BY,
            AccessEvent::VERIFICATION_REQUIRED,
            AccessEvent::DENIED_VERIFICATION_FAILED
        ]
    );
    assert_eq!(result.events[1].presentation, companion(2));
    assert_eq!(result.events[0].presentation, original());
    assert_eq!(eval(&db, req), result);
    assert_eq!(snapshot(&db), before);
}
