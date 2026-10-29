//! Supported modes and externally observed verdict/deadline contracts.
use super::authorization_test_support::*;
use super::*;

fn phase(
    now: u64,
    verdict: AuthorizationVerdict,
    deadline: AccompanimentDeadline,
) -> AuthorizationRequest<'static> {
    let mut req = request();
    req.now = Duration::from_secs(now);
    req.external_phase = AuthorizationExternalPhase::Observed {
        presentation: original(),
        started: original().at,
        fallback_period: AuthorizationFact::Known(Duration::from_secs(10)),
        deadline,
        verdict,
    };
    req
}
fn pending(result: &AuthorizationEvaluation, stage: AuthorizationStage) {
    assert_eq!(
        result.outcome,
        AuthorizationOutcome::Pending(stage),
        "{result:?}"
    );
    assert!(
        result
            .events
            .iter()
            .all(|event| event.access_event != AccessEvent::GRANTED)
    );
}
#[test]
fn authorize_requires_rights_and_grant_active_skips_only_rights_and_accompaniment() {
    let mut db = database(Some(credential(2).into()));
    set_assignments(&mut db, 1, vec![], false);
    denied(&eval(&db, request()), AccessEvent::DENIED_NO_ACCESS_RIGHTS);
    mode(&mut db, Mode::GRANT_ACTIVE);
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
    req.fallback_lockout = AuthorizationFact::Known(true);
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
    req.fallback_lockout = AuthorizationFact::Known(false);
    req.fallback_threat_level = AuthorizationFact::Known(1);
    denied(&eval(&db, req), AccessEvent::DENIED_THREAT_LEVEL);
    reasons(&mut db, 1, &[Reason::DISABLED_MANUAL]);
    denied(
        &eval(&db, request()),
        AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
    );
}
#[test]
fn none_has_no_authorization_checks_or_event_intents() {
    let mut db = database(None);
    mode(&mut db, Mode::NONE);
    for property in [
        P::CREDENTIAL_STATUS,
        P::AUTHORIZATION_EXEMPTIONS,
        P::ASSIGNED_ACCESS_RIGHTS,
    ] {
        override_read(&mut db, credential(1), property, Read::Forbidden, true);
    }
    let result = eval(&db, request());
    assert_eq!(result.outcome, AuthorizationOutcome::NoAuthorization);
    assert!(result.events.is_empty());
    assert!(result.checks.is_empty());
    assert!(result.rights.is_none());
    assert!(result.credentials.is_empty());
}
#[test]
fn deny_exemption_only_removes_deny_all_and_keeps_other_requirements() {
    let mut db = database(None);
    mode(&mut db, Mode::DENY_ALL);
    denied(&eval(&db, request()), AccessEvent::DENIED_DENY_ALL);
    for raw in [0, 1, 2, 3, 5, 6] {
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        denied(&eval(&db, request()), AccessEvent::DENIED_DENY_ALL);
    }
    exempt(&mut db, 1, &[Exemption::DENY]);
    granted(&eval(&db, request()));
    set_assignments(&mut db, 1, vec![], false);
    exempt(&mut db, 1, &[Exemption::DENY]);
    denied(&eval(&db, request()), AccessEvent::DENIED_NO_ACCESS_RIGHTS);
}
#[test]
fn unsupported_or_malformed_mode_is_unresolved_at_admission() {
    for served in [
        PropertyValue::Enumerated(6),
        PropertyValue::Enumerated(64),
        PropertyValue::Null,
    ] {
        let mut db = database(None);
        value(&mut db, point(), P::AUTHORIZATION_MODE, served);
        let result = eval(&db, request());
        unresolved(&result);
        assert!(result.events.is_empty());
        assert_eq!(
            result.checks.last().unwrap().stage,
            AuthorizationStage::Admission
        );
    }
}
#[test]
fn verification_approval_and_rejection_have_distinct_terminal_events() {
    let mut db = database(None);
    mode(&mut db, Mode::VERIFICATION_REQUIRED);
    let result = eval(
        &db,
        phase(
            105,
            AuthorizationVerdict::Approved(Duration::from_secs(104)),
            AccompanimentDeadline::Inclusive,
        ),
    );
    granted(&result);
    assert_eq!(
        events(&result),
        [AccessEvent::VERIFICATION_REQUIRED, AccessEvent::GRANTED]
    );
    let result = eval(
        &db,
        phase(
            105,
            AuthorizationVerdict::Rejected(Duration::from_secs(104)),
            AccompanimentDeadline::Inclusive,
        ),
    );
    denied(&result, AccessEvent::DENIED_VERIFICATION_FAILED);
    assert_eq!(
        events(&result),
        [
            AccessEvent::VERIFICATION_REQUIRED,
            AccessEvent::DENIED_VERIFICATION_FAILED
        ]
    );
}
#[test]
fn verification_waiting_and_verdicts_obey_both_deadline_edges() {
    let mut db = database(None);
    mode(&mut db, Mode::VERIFICATION_REQUIRED);
    for boundary in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        pending(
            &eval(&db, phase(109, AuthorizationVerdict::Waiting, boundary)),
            AuthorizationStage::Verification,
        );
        let exact = eval(&db, phase(110, AuthorizationVerdict::Waiting, boundary));
        let approved = eval(
            &db,
            phase(
                110,
                AuthorizationVerdict::Approved(Duration::from_secs(110)),
                boundary,
            ),
        );
        let rejected = eval(
            &db,
            phase(
                110,
                AuthorizationVerdict::Rejected(Duration::from_secs(110)),
                boundary,
            ),
        );
        if boundary == AccompanimentDeadline::Inclusive {
            pending(&exact, AuthorizationStage::Verification);
            granted(&approved);
            denied(&rejected, AccessEvent::DENIED_VERIFICATION_FAILED);
        } else {
            denied(&exact, AccessEvent::DENIED_VERIFICATION_TIMEOUT);
            denied(&approved, AccessEvent::DENIED_VERIFICATION_TIMEOUT);
            denied(&rejected, AccessEvent::DENIED_VERIFICATION_TIMEOUT);
        }
        denied(
            &eval(&db, phase(111, AuthorizationVerdict::Waiting, boundary)),
            AccessEvent::DENIED_VERIFICATION_TIMEOUT,
        );
        denied(
            &eval(
                &db,
                phase(111, AuthorizationVerdict::DeadlineReached, boundary),
            ),
            AccessEvent::DENIED_VERIFICATION_TIMEOUT,
        );
        denied(
            &eval(
                &db,
                phase(
                    111,
                    AuthorizationVerdict::Approved(Duration::from_secs(111)),
                    boundary,
                ),
            ),
            AccessEvent::DENIED_VERIFICATION_TIMEOUT,
        );
        // A timely approval remains established when evaluated after expiry.
        granted(&eval(
            &db,
            phase(
                111,
                AuthorizationVerdict::Approved(Duration::from_secs(109)),
                boundary,
            ),
        ));
    }
}
#[test]
fn delay_cannot_be_shortened_by_approval_and_finishes_at_its_chosen_edge() {
    let mut db = database(None);
    mode(&mut db, Mode::AUTHORIZATION_DELAYED);
    for boundary in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        for verdict in [
            AuthorizationVerdict::Waiting,
            AuthorizationVerdict::Approved(Duration::from_secs(101)),
        ] {
            let early = eval(&db, phase(109, verdict, boundary));
            pending(&early, AuthorizationStage::Delay);
            assert_eq!(events(&early), [AccessEvent::AUTHORIZATION_DELAYED]);
            let exact = eval(&db, phase(110, verdict, boundary));
            if boundary == AccompanimentDeadline::Inclusive {
                pending(&exact, AuthorizationStage::Delay);
            } else {
                granted(&exact);
            }
            granted(&eval(&db, phase(111, verdict, boundary)));
        }
        granted(&eval(
            &db,
            phase(111, AuthorizationVerdict::DeadlineReached, boundary),
        ));
    }
}
#[test]
fn delay_rejection_only_applies_inside_the_interval() {
    let mut db = database(None);
    mode(&mut db, Mode::AUTHORIZATION_DELAYED);
    for boundary in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        denied(
            &eval(
                &db,
                phase(
                    111,
                    AuthorizationVerdict::Rejected(Duration::from_secs(109)),
                    boundary,
                ),
            ),
            AccessEvent::DENIED_VERIFICATION_FAILED,
        );
        let exact = eval(
            &db,
            phase(
                110,
                AuthorizationVerdict::Rejected(Duration::from_secs(110)),
                boundary,
            ),
        );
        if boundary == AccompanimentDeadline::Inclusive {
            denied(&exact, AccessEvent::DENIED_VERIFICATION_FAILED);
        } else {
            granted(&exact);
        }
        granted(&eval(
            &db,
            phase(
                111,
                AuthorizationVerdict::Rejected(Duration::from_secs(111)),
                boundary,
            ),
        ));
    }
}
#[test]
fn missing_and_inconsistent_external_phase_never_grants() {
    for selected in [Mode::VERIFICATION_REQUIRED, Mode::AUTHORIZATION_DELAYED] {
        let mut db = database(None);
        mode(&mut db, selected);
        unresolved(&eval(&db, request()));
        for verdict in [
            AuthorizationVerdict::DeadlineReached,
            AuthorizationVerdict::Approved(Duration::from_secs(99)),
            AuthorizationVerdict::Rejected(Duration::from_secs(111)),
        ] {
            unresolved(&eval(
                &db,
                phase(105, verdict, AccompanimentDeadline::Inclusive),
            ));
        }
        for field in 0..5 {
            let mut presentation = original();
            let mut started = original().at;
            match field {
                0 => presentation.sequence += 1,
                1 => presentation.point = oid(ObjectType::ACCESS_POINT, 2),
                2 => presentation.credential = credential(2),
                3 => started = Duration::from_secs(99),
                _ => started = Duration::from_secs(106),
            }
            let mut req = phase(
                105,
                AuthorizationVerdict::Waiting,
                AccompanimentDeadline::Inclusive,
            );
            req.external_phase = AuthorizationExternalPhase::Observed {
                presentation,
                started,
                fallback_period: AuthorizationFact::Known(Duration::from_secs(10)),
                deadline: AccompanimentDeadline::Inclusive,
                verdict: AuthorizationVerdict::Waiting,
            };
            unresolved(&eval(&db, req));
        }
    }
}
#[test]
fn verification_and_delay_exemptions_only_skip_the_matching_phase() {
    for (selected, matching, other) in [
        (
            Mode::VERIFICATION_REQUIRED,
            Exemption::VERIFICATION,
            Exemption::AUTHORIZATION_DELAY,
        ),
        (
            Mode::AUTHORIZATION_DELAYED,
            Exemption::AUTHORIZATION_DELAY,
            Exemption::VERIFICATION,
        ),
    ] {
        let mut db = database(None);
        mode(&mut db, selected);
        for exemption in [
            other,
            Exemption::ACCESS_RIGHTS,
            Exemption::DENY,
            Exemption::LOCKOUT,
            Exemption::PASSBACK,
            Exemption::OCCUPANCY_CHECK,
        ] {
            exempt(&mut db, 1, &[exemption]);
            unresolved(&eval(&db, request()));
        }
        exempt(&mut db, 1, &[matching]);
        let result = eval(&db, request());
        granted(&result);
        assert_eq!(events(&result), [AccessEvent::GRANTED]);
        let mut req = request();
        req.fallback_lockout = AuthorizationFact::Known(true);
        denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
        set_assignments(&mut db, 1, vec![], false);
        exempt(&mut db, 1, &[matching]);
        denied(&eval(&db, request()), AccessEvent::DENIED_NO_ACCESS_RIGHTS);
    }
}
#[test]
fn served_verification_period_wins_and_invalid_values_never_fall_back() {
    let mut db = database(None);
    mode(&mut db, Mode::VERIFICATION_REQUIRED);
    value(
        &mut db,
        point(),
        P::VERIFICATION_TIME,
        PropertyValue::Unsigned(2),
    );
    denied(
        &eval(
            &db,
            phase(
                105,
                AuthorizationVerdict::Waiting,
                AccompanimentDeadline::Inclusive,
            ),
        ),
        AccessEvent::DENIED_VERIFICATION_TIMEOUT,
    );
    for served in [PropertyValue::Null, PropertyValue::Unsigned(u64::MAX)] {
        value(&mut db, point(), P::VERIFICATION_TIME, served);
        unresolved(&eval(
            &db,
            phase(
                105,
                AuthorizationVerdict::Waiting,
                AccompanimentDeadline::Inclusive,
            ),
        ));
    }
}
#[test]
fn zero_external_period_and_maximum_monotonic_start_do_not_overflow() {
    let mut db = database(None);
    mode(&mut db, Mode::AUTHORIZATION_DELAYED);
    for boundary in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        let mut req = request();
        req.original.at = Duration::MAX;
        req.now = Duration::MAX;
        req.external_phase = AuthorizationExternalPhase::Observed {
            presentation: req.original,
            started: Duration::MAX,
            fallback_period: AuthorizationFact::Known(Duration::ZERO),
            deadline: boundary,
            verdict: AuthorizationVerdict::Waiting,
        };
        let result = eval(&db, req);
        if boundary == AccompanimentDeadline::Inclusive {
            pending(&result, AuthorizationStage::Delay);
        } else {
            assert_eq!(result.outcome, AuthorizationOutcome::Granted);
        }
    }
}
