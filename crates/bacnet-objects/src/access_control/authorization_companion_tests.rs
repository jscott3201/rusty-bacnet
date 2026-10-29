//! Companion authentication is exact-tuple evidence plus local prerequisites.
use super::authorization_test_support::*;
use super::*;
use std::sync::Arc;

fn authenticated(presentation: CredentialPresentation) -> CompanionAuthenticationAttestation {
    CompanionAuthenticationAttestation {
        presentation,
        authentication: CompanionAuthentication::Authenticated,
    }
}
fn with_companion<'a>(
    witnesses: &'a [AccompanimentWitness],
    attestations: &'a [CompanionAuthenticationAttestation],
) -> AuthorizationRequest<'a> {
    let mut req = request();
    req.accompaniment_witnesses = witnesses;
    req.companion_authentication = attestations;
    req
}
fn assert_by(result: &AuthorizationEvaluation) {
    assert_eq!(result.events[0].access_event, AccessEvent::ACCOMPANIMENT_BY);
    assert_eq!(result.events[0].original, original());
    assert_eq!(result.events[0].presentation, companion(2));
}
#[test]
fn matching_authenticated_active_companion_establishes_prerequisites() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    granted(&result);
    assert_by(&result);
    assert_eq!(
        result
            .credentials
            .iter()
            .map(|c| c.presentation)
            .collect::<Vec<_>>(),
        [original(), companion(2)]
    );
    assert!(result.companion_diagnostics.is_empty());
}
#[test]
fn missing_unknown_and_duplicate_authentication_have_distinct_unresolved_reasons() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    let known = authenticated(companion(2));
    for (attestations, expected) in [
        (vec![], CompanionAuthenticationProblem::Missing),
        (
            vec![CompanionAuthenticationAttestation {
                authentication: CompanionAuthentication::Unknown,
                ..known
            }],
            CompanionAuthenticationProblem::Unknown,
        ),
        (
            vec![known, known],
            CompanionAuthenticationProblem::Duplicate,
        ),
        (
            vec![
                known,
                CompanionAuthenticationAttestation {
                    authentication: CompanionAuthentication::Rejected,
                    ..known
                },
            ],
            CompanionAuthenticationProblem::Duplicate,
        ),
    ] {
        let result = eval(&db, with_companion(&witnesses, &attestations));
        unresolved(&result);
        assert_by(&result);
        assert_eq!(events(&result), [AccessEvent::ACCOMPANIMENT_BY]);
        assert!(matches!(result.outcome, AuthorizationOutcome::Unresolved {
            problem: AuthorizationProblem::CompanionAuthentication { presentation, reason }, ..
        } if presentation == companion(2) && reason == expected));
    }
}
#[test]
fn every_companion_tuple_member_must_match_and_unrelated_attestations_do_not_select_a_path() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    for field in 0..4 {
        let mut wrong = companion(2);
        match field {
            0 => wrong.sequence += 1,
            1 => wrong.credential = credential(3),
            2 => wrong.point = oid(ObjectType::ACCESS_POINT, 2),
            _ => wrong.at += Duration::from_secs(1),
        }
        let attestations = [authenticated(wrong)];
        let result = eval(&db, with_companion(&witnesses, &attestations));
        unresolved(&result);
        assert_by(&result);
    }
    let attestations = [authenticated(companion(3)), authenticated(companion(2))];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    granted(&result);
    assert_eq!(result.accompaniment.unwrap().companions.len(), 1);
}
#[test]
fn rejected_authentication_yields_root_incorrect_with_child_diagnostic_only() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    let attestations = [CompanionAuthenticationAttestation {
        presentation: companion(2),
        authentication: CompanionAuthentication::Rejected,
    }];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    denied(&result, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT);
    assert_by(&result);
    assert_eq!(
        result.companion_diagnostics,
        [CompanionAuthorizationDiagnostic {
            presentation: companion(2),
            failure: CompanionPrerequisiteFailure::AuthenticationRejected,
        }]
    );
    assert_eq!(
        events(&result),
        [
            AccessEvent::ACCOMPANIMENT_BY,
            AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT
        ]
    );
}
#[test]
fn disabled_companion_yields_root_incorrect_and_keeps_its_manual_reason() {
    let mut db = database(Some(credential(2).into()));
    reasons(&mut db, 2, &[Reason::DISABLED_MANUAL]);
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    denied(&result, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT);
    assert_by(&result);
    assert_eq!(
        result.companion_diagnostics,
        [CompanionAuthorizationDiagnostic {
            presentation: companion(2),
            failure: CompanionPrerequisiteFailure::CredentialDenied {
                reason: Reason::DISABLED_MANUAL,
                access_event: AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
            },
        }]
    );
}
#[test]
fn companion_expiry_uses_the_same_supplied_wall_time_and_does_not_publish_child_denial() {
    let mut db = database(Some(credential(2).into()));
    install_window(&mut db, 2);
    db.set_clock_reader(Some(Arc::new(FixedClock(Some(frame(13))))));
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let mut req = with_companion(&witnesses, &attestations);
    req.wall_time = Some(frame(13));
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT);
    assert!(matches!(
        result.companion_diagnostics[0].failure,
        CompanionPrerequisiteFailure::CredentialDenied {
            reason: Reason::DISABLED_EXPIRED,
            access_event: AccessEvent::DENIED_CREDENTIAL_EXPIRED,
        }
    ));
    assert_eq!(
        events(&result),
        [
            AccessEvent::ACCOMPANIMENT_BY,
            AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT
        ]
    );
    req.wall_time = Some(frame(11));
    unresolved(&eval(&db, req));
}
#[test]
fn contradictory_companion_status_is_unresolved_without_child_or_terminal_event() {
    let mut db = database(Some(credential(2).into()));
    reasons(&mut db, 2, &[Reason::DISABLED_MANUAL]);
    value(
        &mut db,
        credential(2),
        P::CREDENTIAL_STATUS,
        PropertyValue::Enumerated(1),
    );
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    unresolved(&result);
    assert_eq!(events(&result), [AccessEvent::ACCOMPANIMENT_BY]);
}
#[test]
fn rights_exempt_companion_still_needs_authentication_and_activity() {
    let mut db = database(Some(credential(2).into()));
    set_assignments(&mut db, 2, vec![], true);
    let witnesses = [presented(2)];
    unresolved(&eval(&db, with_companion(&witnesses, &[])));
    let attestations = [authenticated(companion(2))];
    granted(&eval(&db, with_companion(&witnesses, &attestations)));
    reasons(&mut db, 2, &[Reason::DISABLED_LOCKOUT]);
    denied(
        &eval(&db, with_companion(&witnesses, &attestations)),
        AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
    );
}
#[test]
fn authenticated_active_companion_cannot_evade_wrong_criterion_or_denied_own_rights() {
    let mut db = database(Some(credential(2).into()));
    let wrong = [presented(3)];
    let attestations = [authenticated(companion(3))];
    denied(
        &eval(&db, with_companion(&wrong, &attestations)),
        AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
    );
    set_assignments(&mut db, 2, vec![], false);
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let result = eval(&db, with_companion(&witnesses, &attestations));
    denied(&result, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT);
    assert_by(&result);
    // Known rights denial needs no attestation because that authority is not used.
    denied(
        &eval(&db, with_companion(&witnesses, &[])),
        AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
    );
}
#[test]
fn companion_excluded_assignments_apply_both_explicit_policies() {
    let mut db = database(Some(credential(2).into()));
    set_assignments(
        &mut db,
        2,
        vec![assignment(rights(99), true), assignment(rights(2), true)],
        false,
    );
    let witnesses = [presented(2)];
    let attestations = [authenticated(companion(2))];
    let mut req = with_companion(&witnesses, &attestations);
    let honor = eval(&db, req);
    granted(&honor);
    assert_eq!(
        honor.accompaniment.as_ref().unwrap().companions[0]
            .rights
            .unresolved
            .len(),
        1
    );
    req.rights_policy = AuthorizationRightsPolicy::RequireCompleteLocalEvidence;
    let strict = eval(&db, req);
    unresolved(&strict);
    assert_by(&strict);
    assert_eq!(strict.accompaniment, honor.accompaniment);
    assert!(matches!(strict.outcome, AuthorizationOutcome::Unresolved {
        problem: AuthorizationProblem::ExcludedRights { presentation }, ..
    } if presentation == companion(2)));
}
#[test]
fn two_link_chain_requires_each_exact_authentication_and_local_prerequisite() {
    let mut db = database(Some(credential(2).into()));
    set_rights(
        &mut db,
        2,
        Some(credential(3).into()),
        vec![anytime()],
        vec![],
        true,
    );
    let third = CredentialPresentation {
        sequence: 12,
        credential: credential(3),
        point: point(),
        at: Duration::from_secs(102),
    };
    let witnesses = [
        presented(2),
        AccompanimentWitness {
            original: 11,
            evidence: AccompanimentEvidence::Presented(third),
        },
    ];
    let both = [authenticated(companion(2)), authenticated(third)];
    let mut req = with_companion(&witnesses, &both);
    req.now = third.at;
    let result = eval(&db, req);
    granted(&result);
    assert_by(&result);
    assert_eq!(result.credentials.len(), 3);
    assert_eq!(
        events(&result),
        [AccessEvent::ACCOMPANIMENT_BY, AccessEvent::GRANTED]
    );
    req.companion_authentication = &both[..1];
    unresolved(&eval(&db, req));
    req.companion_authentication = &both;
    reasons(&mut db, 3, &[Reason::DISABLED_MAX_USES]);
    let result = eval(&db, req);
    denied(&result, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT);
    assert_eq!(result.companion_diagnostics[0].presentation, third);
    assert_eq!(
        events(&result),
        [
            AccessEvent::ACCOMPANIMENT_BY,
            AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT
        ]
    );
    assert!(
        result
            .events
            .iter()
            .all(|event| event.original == original())
    );
}
#[test]
fn cycles_and_link_bounds_stay_unresolved_without_recursive_child_transactions() {
    let mut db = database(Some(credential(2).into()));
    set_rights(
        &mut db,
        2,
        Some(credential(1).into()),
        vec![anytime()],
        vec![],
        true,
    );
    let return_to_original = CredentialPresentation {
        sequence: 12,
        at: Duration::from_secs(102),
        ..original()
    };
    let witnesses = [
        presented(2),
        AccompanimentWitness {
            original: 11,
            evidence: AccompanimentEvidence::Presented(return_to_original),
        },
    ];
    let attestations = [authenticated(companion(2))];
    let mut req = with_companion(&witnesses, &attestations);
    req.now = Duration::from_secs(102);
    let result = eval(&db, req);
    unresolved(&result);
    assert_by(&result);
    assert_eq!(events(&result), [AccessEvent::ACCOMPANIMENT_BY]);
    assert!(matches!(
        result.outcome,
        AuthorizationOutcome::Unresolved {
            problem: AuthorizationProblem::Accompaniment(
                AccompanimentProblem::CyclicRequirement { .. }
            ),
            ..
        }
    ));
    req.max_accompaniment_links = 1;
    let result = eval(&db, req);
    unresolved(&result);
    assert!(matches!(
        result.outcome,
        AuthorizationOutcome::Unresolved {
            problem: AuthorizationProblem::Accompaniment(AccompanimentProblem::LinkLimit {
                max_links: 1
            }),
            ..
        }
    ));
}
