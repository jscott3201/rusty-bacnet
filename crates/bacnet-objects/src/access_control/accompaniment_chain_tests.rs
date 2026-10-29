//! Child dependencies are evidence for one original transaction.
use super::accompaniment_test_support::*;
use super::*;
use std::time::Duration;

fn chain() -> (crate::database::ObjectDatabase, [AccompanimentWitness; 2]) {
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
        at: Duration::from_secs(102),
        ..companion(3)
    };
    (
        db,
        [
            presented(2),
            AccompanimentWitness {
                original: 11,
                evidence: AccompanimentEvidence::Presented(third),
            },
        ],
    )
}
fn at_102(witnesses: &[AccompanimentWitness]) -> AccompanimentRequest<'_> {
    AccompanimentRequest {
        now: Duration::from_secs(102),
        ..request(witnesses)
    }
}
#[test]
fn accompaniment_chain_satisfied_emits_only_original_by() {
    let (db, witnesses) = chain();
    let result = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
    assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
    assert_eq!(result.original, original());
    assert_eq!(result.events, [by(companion(2))]);
    assert_eq!(result.companions.len(), 2);
    assert_eq!(result.companions[0].requirement, Some(credential(3).into()));
    assert_eq!(result.companions[1].presentation.sequence, 12);
    assert_eq!(result.companions[1].requirement, None);
}
#[test]
fn accompaniment_chain_wait_missing_and_expiry_keep_original_transaction_identity() {
    let (db, mut witnesses) = chain();
    let missing = evaluate_accompaniment(&db, at_102(&witnesses[..1])).unwrap();
    assert_eq!(
        missing.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::MissingWitness { presentation: 11 })
    );
    assert_eq!(missing.events, [by(companion(2))]);
    witnesses[1].evidence = AccompanimentEvidence::Waiting;
    let waiting = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
    assert_eq!(
        waiting.outcome,
        AccompanimentOutcome::Waiting { presentation: 11 }
    );
    assert_eq!(waiting.events, [by(companion(2))]);
    for evidence in [
        AccompanimentEvidence::Waiting,
        AccompanimentEvidence::DeadlineReached,
    ] {
        witnesses[1].evidence = evidence;
        let result = evaluate_accompaniment(
            &db,
            AccompanimentRequest {
                now: Duration::from_secs(112),
                ..request(&witnesses)
            },
        )
        .unwrap();
        assert_eq!(
            result.outcome,
            AccompanimentOutcome::DeniedIncorrectCompanion { presentation: 11 }
        );
        assert_eq!(result.events, [by(companion(2)), incorrect()]);
    }
}
#[test]
fn accompaniment_child_wrong_criterion_or_rights_denial_invalidates_only_root() {
    for wrong_criterion in [true, false] {
        let (mut db, witnesses) = chain();
        if wrong_criterion {
            set_rights(
                &mut db,
                2,
                Some(credential(1).into()),
                vec![anytime()],
                vec![],
                true,
            );
        } else {
            set_rights(&mut db, 3, None, vec![anytime()], vec![anytime()], true);
        }
        let result = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
        assert_eq!(
            result.outcome,
            AccompanimentOutcome::DeniedIncorrectCompanion { presentation: 11 }
        );
        assert_eq!(result.events, [by(companion(2)), incorrect()]);
    }
}
#[test]
fn accompaniment_child_remote_or_unreadable_requirement_stays_unresolved() {
    for remote in [true, false] {
        let (mut db, witnesses) = chain();
        if remote {
            set_rights(
                &mut db,
                2,
                Some(qualified(credential(3), device(200))),
                vec![anytime()],
                vec![],
                true,
            );
        } else {
            override_read(
                &mut db,
                rights(2),
                PropertyIdentifier::ACCOMPANIMENT,
                Read::Value(PropertyValue::Boolean(false)),
                true,
            );
        }
        let result = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
        assert!(matches!(
            result.outcome,
            AccompanimentOutcome::Unresolved(_)
        ));
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_direct_self_attempt_is_incorrect_but_unattempted_self_waits() {
    let db = database(Some(credential(1).into()));
    assert_incorrect(&evaluate(&db, &[presented(1)]), companion(1));
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Waiting,
    }];
    let waiting = evaluate(&db, &witnesses);
    assert_eq!(
        waiting.outcome,
        AccompanimentOutcome::Waiting { presentation: 10 }
    );
    assert!(waiting.events.is_empty());
    let expired = evaluate_accompaniment(
        &db,
        AccompanimentRequest {
            now: Duration::from_secs(111),
            ..request(&witnesses)
        },
    )
    .unwrap();
    assert_eq!(expired.outcome, AccompanimentOutcome::DeniedNoCompanion);
    assert_eq!(
        expired.events,
        [AccompanimentEventIntent {
            original: original(),
            access_event: AccessEvent::DENIED_NO_ACCOMPANIMENT,
            presentation: original()
        }]
    );
}
#[test]
fn accompaniment_reentered_credential_or_presentation_is_cycle_not_denial() {
    for repeat_exact_presentation in [false, true] {
        let (mut db, mut witnesses) = chain();
        set_rights(
            &mut db,
            2,
            Some(credential(1).into()),
            vec![anytime()],
            vec![],
            true,
        );
        let repeated = if repeat_exact_presentation {
            original()
        } else {
            CredentialPresentation {
                sequence: 12,
                at: Duration::from_secs(102),
                ..original()
            }
        };
        witnesses[1].evidence = AccompanimentEvidence::Presented(repeated);
        let result = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
        assert_eq!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::CyclicRequirement {
                path: vec![original(), companion(2), repeated]
            })
        );
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_inconsistent_reused_sequence_is_not_a_cycle_or_attempt() {
    let (mut db, mut witnesses) = chain();
    set_rights(
        &mut db,
        2,
        Some(credential(1).into()),
        vec![anytime()],
        vec![],
        true,
    );
    witnesses[1].evidence = AccompanimentEvidence::Presented(CredentialPresentation {
        at: Duration::from_secs(101),
        ..original()
    });
    let result = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
            presentation: 10,
            reason: AccompanimentPresentationFailure::ReusedSequence
        })
    );
    assert_eq!(result.events, [by(companion(2))]);
}
#[test]
fn accompaniment_traversal_budget_never_becomes_success() {
    let (db, witnesses) = chain();
    for max_links in 0..=2 {
        let result = evaluate_accompaniment(
            &db,
            AccompanimentRequest {
                max_links,
                ..at_102(&witnesses)
            },
        )
        .unwrap();
        assert_eq!(
            result.outcome,
            if max_links == 2 {
                AccompanimentOutcome::Satisfied
            } else {
                AccompanimentOutcome::Unresolved(AccompanimentProblem::LinkLimit { max_links })
            }
        );
        assert_eq!(
            result.events,
            if max_links == 0 {
                vec![]
            } else {
                vec![by(companion(2))]
            }
        );
    }
}
#[test]
fn accompaniment_long_acyclic_chain_is_bounded_and_iterative() {
    const LAST: u32 = 128;
    let mut db = database(Some(credential(2).into()));
    let mut witnesses = Vec::new();
    for n in 1..=LAST {
        if n > 3 {
            db.add(Box::new(
                AccessCredentialObject::new(n, format!("card-{n}")).unwrap(),
            ))
            .unwrap();
            db.add(Box::new(
                AccessRightsObject::new(n, format!("rights-{n}")).unwrap(),
            ))
            .unwrap();
        }
        set_assignments(&mut db, n, vec![assignment(rights(n), true)], false);
        set_rights(
            &mut db,
            n,
            (n < LAST).then(|| credential(n + 1).into()),
            vec![anytime()],
            vec![],
            true,
        );
        if n < LAST {
            witnesses.push(AccompanimentWitness {
                original: u64::from(n + 9),
                evidence: AccompanimentEvidence::Presented(CredentialPresentation {
                    sequence: u64::from(n + 10),
                    credential: credential(n + 1),
                    at: Duration::from_secs(u64::from(n + 100)),
                    point: point(),
                }),
            });
        }
    }
    let result = evaluate_accompaniment(
        &db,
        AccompanimentRequest {
            now: Duration::from_secs(227),
            max_links: 127,
            ..request(&witnesses)
        },
    )
    .unwrap();
    assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
    assert_eq!(result.companions.len(), 127);
    assert_eq!(result.events, [by(companion(2))]);
}
#[test]
fn accompaniment_missing_duplicate_and_unused_witnesses_are_distinguished() {
    let db = database(Some(credential(2).into()));
    let missing = evaluate(&db, &[]);
    assert_eq!(
        missing.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::MissingWitness { presentation: 10 })
    );
    assert!(missing.events.is_empty());
    let duplicate = evaluate(&db, &[presented(2), presented(3)]);
    assert_eq!(
        duplicate.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::AmbiguousWitness {
            presentation: 10
        })
    );
    assert!(duplicate.events.is_empty());
    let extra = AccompanimentWitness {
        original: 999,
        evidence: AccompanimentEvidence::Presented(companion(1)),
    };
    assert_eq!(
        evaluate(&db, &[presented(2), extra, extra]),
        evaluate(&db, &[presented(2)])
    );
}
#[test]
fn accompaniment_repeated_evaluation_preserves_every_served_property() {
    let (db, witnesses) = chain();
    let snapshot = || {
        [
            point(),
            credential(1),
            credential(2),
            credential(3),
            rights(1),
            rights(2),
            rights(3),
        ]
        .into_iter()
        .map(|oid| {
            let object = db.get(&oid).unwrap();
            let properties = object
                .property_list()
                .iter()
                .map(|property| {
                    (
                        *property,
                        format!("{:?}", object.read_property(*property, None)),
                    )
                })
                .collect::<Vec<_>>();
            (oid, properties)
        })
        .collect::<Vec<_>>()
    };
    let before = snapshot();
    let first = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
    let second = evaluate_accompaniment(&db, at_102(&witnesses)).unwrap();
    assert_eq!(first, second);
    assert_eq!(snapshot(), before);
    assert_eq!(first.events, [by(companion(2))]);
}

#[test]
fn accompaniment_detached_intents_preserve_explicit_original_identity() {
    for candidate in [2, 3] {
        let db = database(Some(credential(2).into()));
        let first = CredentialPresentation {
            sequence: 77,
            at: Duration::from_secs(200),
            ..original()
        };
        let second = CredentialPresentation {
            sequence: 78,
            at: Duration::from_secs(201),
            ..companion(candidate)
        };
        let witnesses = [AccompanimentWitness {
            original: first.sequence,
            evidence: AccompanimentEvidence::Presented(second),
        }];
        let result = evaluate_accompaniment(
            &db,
            AccompanimentRequest {
                original: first,
                now: second.at,
                ..request(&witnesses)
            },
        )
        .unwrap();
        let intents = result.events;
        assert_eq!(intents[0].access_event, AccessEvent::ACCOMPANIMENT_BY);
        assert_eq!(intents[0].presentation, second);
        assert!(intents.iter().all(|intent| intent.original == first));
        if candidate == 2 {
            assert_eq!(intents.len(), 1);
        } else {
            assert_eq!(intents.len(), 2);
            assert_eq!(
                intents[1].access_event,
                AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT
            );
            assert_eq!(intents[1].presentation, first);
        }
    }
}

fn chronology_case(
    first: (u64, u64),
    second: (u64, u64),
    third: (u64, u64),
) -> AccompanimentEvaluation {
    let (db, _) = chain();
    let presentation = |credential, (sequence, seconds)| CredentialPresentation {
        sequence,
        credential,
        point: point(),
        at: Duration::from_secs(seconds),
    };
    let first = presentation(credential(1), first);
    let second = presentation(credential(2), second);
    let third = presentation(credential(3), third);
    let witnesses = [
        AccompanimentWitness {
            original: first.sequence,
            evidence: AccompanimentEvidence::Presented(second),
        },
        AccompanimentWitness {
            original: second.sequence,
            evidence: AccompanimentEvidence::Presented(third),
        },
    ];
    let mut request = request(&witnesses);
    request.original = first;
    request.now = first.at.max(second.at).max(third.at);
    request.timing.order = AccompanimentOrder::Either;
    request.max_links = 2;
    let result = evaluate_accompaniment(&db, request).unwrap();
    assert_eq!(
        result.events,
        [AccompanimentEventIntent {
            original: first,
            access_event: AccessEvent::ACCOMPANIMENT_BY,
            presentation: second,
        }]
    );
    result
}
#[test]
fn accompaniment_chain_rejects_nonadjacent_chronology_conflicts() {
    for (first, second, third) in [
        ((10, 100), (20, 102), (5, 101)),
        ((20, 102), (10, 100), (25, 101)),
    ] {
        let result = chronology_case(first, second, third);
        assert_eq!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
                presentation: third.0,
                reason: AccompanimentPresentationFailure::NonMonotonic,
            },)
        );
        assert_eq!(result.companions.len(), 1);
    }
}
#[test]
fn accompaniment_chain_accepts_globally_consistent_zigzag_chronology() {
    for (first, second, third) in [
        ((10, 100), (20, 102), (15, 101)),
        ((20, 102), (10, 100), (15, 101)),
    ] {
        let result = chronology_case(first, second, third);
        assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        assert_eq!(result.companions.len(), 2);
    }
}
#[test]
fn accompaniment_chain_accepts_equal_time_distinct_sequences() {
    for (first, second, third) in [
        ((10, 101), (20, 102), (5, 101)),
        ((20, 101), (10, 100), (25, 101)),
        ((10, 100), (20, 100), (5, 100)),
    ] {
        let result = chronology_case(first, second, third);
        assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        assert_eq!(result.companions.len(), 2);
    }
}
