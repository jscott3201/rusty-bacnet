//! Explicit monotonic order, period and deadline-edge qualification.
use super::accompaniment_test_support::*;
use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::time::Duration;

fn run(
    at: Duration,
    sequence: u64,
    now: Duration,
    timing: AccompanimentTiming,
) -> AccompanimentEvaluation {
    let db = database(Some(credential(2).into()));
    let candidate = CredentialPresentation {
        at,
        sequence,
        ..companion(2)
    };
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Presented(candidate),
    }];
    let mut request = request(&witnesses);
    request.now = now;
    request.timing = timing;
    evaluate_accompaniment(&db, request).unwrap()
}
fn policy(order: AccompanimentOrder, deadline: AccompanimentDeadline) -> AccompanimentTiming {
    AccompanimentTiming {
        order,
        deadline,
        fallback_period: Duration::from_secs(10),
    }
}
#[test]
fn accompaniment_before_after_and_either_are_explicit_policies() {
    for (order, at, sequence, valid) in [
        (AccompanimentOrder::Before, 99, 9, true),
        (AccompanimentOrder::Before, 101, 11, false),
        (AccompanimentOrder::After, 101, 11, true),
        (AccompanimentOrder::After, 99, 9, false),
        (AccompanimentOrder::Either, 99, 9, true),
        (AccompanimentOrder::Either, 101, 11, true),
    ] {
        let result = run(
            Duration::from_secs(at),
            sequence,
            Duration::from_secs(101),
            policy(order, AccompanimentDeadline::Inclusive),
        );
        if valid {
            assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        } else {
            assert!(matches!(
                result.outcome,
                AccompanimentOutcome::DeniedIncorrectCompanion { .. }
            ));
            assert_eq!(result.events.last(), Some(&incorrect()));
        }
        assert_eq!(result.events[0].access_event, AccessEvent::ACCOMPANIMENT_BY);
    }
}
#[test]
fn accompaniment_equal_time_uses_distinct_monotonic_sequence_for_order() {
    for sequence in [9, 11] {
        let result = run(
            Duration::from_secs(100),
            sequence,
            Duration::from_secs(100),
            policy(
                if sequence == 9 {
                    AccompanimentOrder::Before
                } else {
                    AccompanimentOrder::After
                },
                AccompanimentDeadline::Inclusive,
            ),
        );
        assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
    }
}
#[test]
fn accompaniment_deadline_edges_distinguish_before_exact_and_after() {
    for edge in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        for (at, valid) in [
            (Duration::from_secs(110) - Duration::from_nanos(1), true),
            (
                Duration::from_secs(110),
                edge == AccompanimentDeadline::Inclusive,
            ),
            (Duration::from_secs(110) + Duration::from_nanos(1), false),
        ] {
            let result = run(
                at,
                11,
                Duration::from_secs(111),
                policy(AccompanimentOrder::After, edge),
            );
            if valid {
                assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
                assert_eq!(result.events.len(), 1);
            } else {
                assert_eq!(result.outcome, AccompanimentOutcome::DeniedNoCompanion);
                assert_eq!(
                    result.events,
                    [AccompanimentEventIntent {
                        original: original(),
                        access_event: AccessEvent::DENIED_NO_ACCOMPANIMENT,
                        presentation: original()
                    }]
                );
            }
        }
    }
}
#[test]
fn accompaniment_waiting_and_deadline_witnesses_are_checked_against_time() {
    let db = database(Some(credential(2).into()));
    for edge in [
        AccompanimentDeadline::Inclusive,
        AccompanimentDeadline::Exclusive,
    ] {
        for evidence in [
            AccompanimentEvidence::Waiting,
            AccompanimentEvidence::DeadlineReached,
        ] {
            for (now, expired) in [
                (109, false),
                (110, edge == AccompanimentDeadline::Exclusive),
                (111, true),
            ] {
                let witnesses = [AccompanimentWitness {
                    original: 10,
                    evidence,
                }];
                let mut request = request(&witnesses);
                request.now = Duration::from_secs(now);
                request.timing.deadline = edge;
                let result = evaluate_accompaniment(&db, request).unwrap();
                if expired {
                    assert_eq!(result.outcome, AccompanimentOutcome::DeniedNoCompanion);
                } else if evidence == AccompanimentEvidence::Waiting {
                    assert_eq!(
                        result.outcome,
                        AccompanimentOutcome::Waiting { presentation: 10 }
                    );
                    assert!(result.events.is_empty());
                } else {
                    assert_eq!(
                        result.outcome,
                        AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
                            presentation: 10,
                            reason: AccompanimentPresentationFailure::PrematureDeadline
                        })
                    );
                    assert!(result.events.is_empty());
                }
            }
        }
    }
}
#[test]
fn accompaniment_zero_period_is_not_an_unlimited_wait() {
    let mut timing = policy(AccompanimentOrder::After, AccompanimentDeadline::Inclusive);
    timing.fallback_period = Duration::ZERO;
    let same = run(
        Duration::from_secs(100),
        11,
        Duration::from_secs(100),
        timing,
    );
    assert_eq!(same.outcome, AccompanimentOutcome::Satisfied);
    let later = run(
        Duration::from_secs(100) + Duration::from_nanos(1),
        11,
        Duration::from_secs(101),
        timing,
    );
    assert_eq!(later.outcome, AccompanimentOutcome::DeniedNoCompanion);
    timing.deadline = AccompanimentDeadline::Exclusive;
    assert_eq!(
        run(
            Duration::from_secs(100),
            11,
            Duration::from_secs(100),
            timing
        )
        .outcome,
        AccompanimentOutcome::DeniedNoCompanion
    );
}
#[test]
fn accompaniment_late_cannot_reopen_but_timely_evidence_can_be_evaluated_later() {
    let timing = policy(AccompanimentOrder::After, AccompanimentDeadline::Inclusive);
    let late = run(
        Duration::from_secs(111),
        11,
        Duration::from_secs(120),
        timing,
    );
    assert_eq!(late.outcome, AccompanimentOutcome::DeniedNoCompanion);
    assert!(
        !late
            .events
            .iter()
            .any(|event| event.access_event == AccessEvent::ACCOMPANIMENT_BY)
    );
    assert_eq!(
        run(
            Duration::from_secs(101),
            11,
            Duration::from_secs(120),
            timing
        )
        .outcome,
        AccompanimentOutcome::Satisfied
    );
}
#[test]
fn accompaniment_stale_before_future_and_nonmonotonic_candidates_are_unresolved() {
    for (at, sequence, now, reason) in [
        (89, 9, 101, AccompanimentPresentationFailure::OutsideWindow),
        (102, 11, 101, AccompanimentPresentationFailure::Future),
        (99, 11, 101, AccompanimentPresentationFailure::NonMonotonic),
        (101, 9, 101, AccompanimentPresentationFailure::NonMonotonic),
    ] {
        let result = run(
            Duration::from_secs(at),
            sequence,
            Duration::from_secs(now),
            policy(AccompanimentOrder::Either, AccompanimentDeadline::Inclusive),
        );
        assert_eq!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
                presentation: sequence,
                reason
            })
        );
        assert!(result.events.is_empty());
    }
}
#[test]
fn accompaniment_future_original_and_wrong_point_never_create_by() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    let mut before = request(&witnesses);
    before.now = Duration::from_secs(99);
    let result = evaluate_accompaniment(&db, before).unwrap();
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
            presentation: 10,
            reason: AccompanimentPresentationFailure::Future
        })
    );
    assert!(result.events.is_empty());
    let elsewhere = CredentialPresentation {
        point: oid(ObjectType::ACCESS_POINT, 2),
        ..companion(2)
    };
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Presented(elsewhere),
    }];
    let result = evaluate(&db, &witnesses);
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
            presentation: 11,
            reason: AccompanimentPresentationFailure::WrongPoint
        })
    );
    assert!(result.events.is_empty());
}
#[test]
fn accompaniment_served_period_uses_seconds_and_overrides_fallback() {
    for (millis, valid) in [(1500, true), (2000, true), (2001, false)] {
        let mut db = database(Some(credential(2).into()));
        override_read(
            &mut db,
            point(),
            PropertyIdentifier::ACCOMPANIMENT_TIME,
            Read::Value(PropertyValue::Unsigned(2)),
            true,
        );
        let candidate = CredentialPresentation {
            at: original().at + Duration::from_millis(millis),
            ..companion(2)
        };
        let witnesses = [AccompanimentWitness {
            original: 10,
            evidence: AccompanimentEvidence::Presented(candidate),
        }];
        let mut request = request(&witnesses);
        request.now = Duration::from_secs(103);
        let result = evaluate_accompaniment(&db, request).unwrap();
        assert_eq!(
            result.outcome,
            if valid {
                AccompanimentOutcome::Satisfied
            } else {
                AccompanimentOutcome::DeniedNoCompanion
            }
        );
    }
}
#[test]
fn accompaniment_only_absent_optional_period_uses_fallback() {
    for (read, advertised, absent) in [
        (
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            false,
            true,
        ),
        (
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            true,
            false,
        ),
        (
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
            false,
            false,
        ),
        (Read::Value(PropertyValue::Signed(2)), true, false),
        (Read::Value(PropertyValue::Null), true, false),
    ] {
        let mut db = database(Some(credential(2).into()));
        override_read(
            &mut db,
            point(),
            PropertyIdentifier::ACCOMPANIMENT_TIME,
            read,
            advertised,
        );
        let result = evaluate(&db, &[presented(2)]);
        if absent {
            assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        } else {
            assert!(matches!(
                result.outcome,
                AccompanimentOutcome::Unresolved(AccompanimentProblem::Property {
                    property: PropertyIdentifier::ACCOMPANIMENT_TIME,
                    ..
                })
            ));
            assert!(result.events.is_empty());
        }
    }
}
#[test]
fn accompaniment_monotonic_extremes_do_not_add_or_overflow_deadlines() {
    let db = database(Some(credential(2).into()));
    let mut first = original();
    first.at = Duration::MAX - Duration::from_secs(1);
    let mut second = companion(2);
    second.at = Duration::MAX;
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Presented(second),
    }];
    let mut request = request(&witnesses);
    request.original = first;
    request.now = Duration::MAX;
    request.timing.fallback_period = Duration::MAX;
    assert_eq!(
        evaluate_accompaniment(&db, request).unwrap().outcome,
        AccompanimentOutcome::Satisfied
    );
    request.now = Duration::ZERO;
    assert!(matches!(
        evaluate_accompaniment(&db, request).unwrap().outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Presentation {
            reason: AccompanimentPresentationFailure::Future,
            ..
        })
    ));
}
