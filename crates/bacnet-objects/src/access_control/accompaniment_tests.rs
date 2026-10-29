//! Pure accompaniment-stage behavior, distinct from complete authorization.
use super::*;

use super::accompaniment_test_support::*;
#[test]
fn accompaniment_absent_requirement_is_a_stage_only_result() {
    let db = database(None);
    let result = evaluate_accompaniment(&db, request(&[])).unwrap();
    assert!(matches!(
        result.original_rights.decision,
        AccessRightsDecision::Granted { .. }
    ));
    assert_eq!(result.outcome, AccompanimentOutcome::NotRequired);
    assert!(
        result.requirement.is_none() && result.events.is_empty() && result.companions.is_empty()
    );
}

#[test]
fn accompaniment_required_waits_without_inventing_an_event() {
    let db = database(Some(credential(2).into()));
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Waiting,
    }];
    let result = evaluate_accompaniment(&db, request(&witnesses)).unwrap();
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Waiting { presentation: 10 }
    );
    assert!(result.events.is_empty());
    assert_eq!(result.requirement, Some(credential(2).into()));
}
#[test]
fn accompaniment_exact_credential_needs_its_own_rights_and_emits_only_by() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(2)];
    let result = evaluate_accompaniment(&db, request(&witnesses)).unwrap();
    assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
    assert_eq!(
        result.events,
        [AccompanimentEventIntent {
            original: original(),
            access_event: AccessEvent::ACCOMPANIMENT_BY,
            presentation: companion(2)
        }]
    );
    assert_eq!(result.companions.len(), 1);
    assert!(matches!(
        result.companions[0].rights.decision,
        AccessRightsDecision::Granted { .. }
    ));
    assert_eq!(result.original, original());
}
#[test]
fn accompaniment_wrong_credential_emits_by_then_original_incorrect() {
    let db = database(Some(credential(2).into()));
    let witnesses = [presented(3)];
    let result = evaluate_accompaniment(&db, request(&witnesses)).unwrap();
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::DeniedIncorrectCompanion { presentation: 10 }
    );
    assert_eq!(
        result.events,
        [
            AccompanimentEventIntent {
                original: original(),
                access_event: AccessEvent::ACCOMPANIMENT_BY,
                presentation: companion(3)
            },
            AccompanimentEventIntent {
                original: original(),
                access_event: AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
                presentation: original()
            },
        ]
    );
}

#[test]
fn accompaniment_primary_denial_and_exemption_do_not_read_requirement() {
    for exempt in [false, true] {
        let mut db = database(Some(credential(2).into()));
        if exempt {
            set_assignments(&mut db, 1, vec![assignment(rights(1), true)], true);
        } else {
            set_rights(
                &mut db,
                1,
                Some(credential(2).into()),
                vec![anytime()],
                vec![anytime()],
                true,
            );
        }
        override_read(
            &mut db,
            rights(1),
            PropertyIdentifier::ACCOMPANIMENT,
            Read::Forbidden,
            true,
        );
        let result = evaluate(&db, &[]);
        assert_eq!(
            result.outcome,
            if exempt {
                AccompanimentOutcome::NotRequired
            } else {
                AccompanimentOutcome::NotEvaluated
            }
        );
        assert!(result.events.is_empty() && result.requirement.is_none());
        assert_eq!(result.original_rights.decision.allows(), exempt);
    }
}
#[test]
fn accompaniment_first_winning_rights_requirement_cannot_be_bypassed() {
    let mut db = database(Some(credential(3).into()));
    set_assignments(
        &mut db,
        1,
        vec![assignment(rights(1), true), assignment(rights(2), true)],
        false,
    );
    let result = evaluate(&db, &[presented(2)]);
    assert_eq!(
        result
            .original_rights
            .decision
            .rule()
            .unwrap()
            .access_rights,
        rights(1)
    );
    assert_eq!(result.requirement, Some(credential(3).into()));
    assert_incorrect(&result, companion(2));
}
#[test]
fn accompaniment_fully_unspecified_is_distinct_from_a_partial_wildcard() {
    let wildcard = ObjectIdentifier::WILDCARD_INSTANCE;
    for device_member in [None, Some(device(wildcard))] {
        let reference = BACnetDeviceObjectReference {
            object_identifier: credential(wildcard),
            device_identifier: device_member,
        };
        let result = evaluate(&database(Some(reference)), &[]);
        assert_eq!(result.outcome, AccompanimentOutcome::NotRequired);
        assert!(result.requirement.is_none() && result.events.is_empty());
    }
    let reference = qualified(credential(wildcard), device(100));
    let result = evaluate(&database(Some(reference.clone())), &[presented(2)]);
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
            reference,
            reason: AccompanimentReferenceFailure::Wildcard
        })
    );
    assert_eq!(result.events, [by(companion(2))]);
}
#[test]
fn accompaniment_rights_criterion_requires_effective_assignment_and_own_rights() {
    for (assigned, target_enabled, expected) in [
        (true, true, true),
        (false, true, false),
        (true, false, false),
    ] {
        let mut db = database(Some(rights(3).into()));
        set_rights(
            &mut db,
            3,
            None,
            vec![anytime()],
            Vec::new(),
            target_enabled,
        );
        set_assignments(
            &mut db,
            2,
            vec![assignment(rights(2), true), assignment(rights(3), assigned)],
            false,
        );
        let result = evaluate(&db, &[presented(2)]);
        if expected {
            assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        } else {
            assert_incorrect(&result, companion(2));
        }
    }
    let db = database(Some(rights(3).into()));
    assert_incorrect(&evaluate(&db, &[presented(2)]), companion(2));
}
#[test]
fn accompaniment_matching_assignment_does_not_bypass_negative_time_or_location_rules() {
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
    for required in [rights(2), credential(2), user(1)] {
        for failure in 0..3 {
            let mut db = database(Some(required.into()));
            add_user(&mut db, vec![credential(2).into()]);
            let (positive, negative) = match failure {
                0 => (vec![anytime()], vec![anytime()]),
                1 => {
                    let schedule = crate::schedule::ScheduleObject::new(
                        50,
                        "closed",
                        PropertyValue::Boolean(false),
                    )
                    .unwrap();
                    db.add(Box::new(schedule)).unwrap();
                    let time = BACnetDeviceObjectPropertyReference::new_local(
                        oid(ObjectType::SCHEDULE, 50),
                        PropertyIdentifier::PRESENT_VALUE.to_raw(),
                    );
                    (
                        vec![bacnet_types::constructed::BACnetAccessRule::new(
                            Some(time),
                            Some(point().into()),
                            true,
                        )],
                        Vec::new(),
                    )
                }
                _ => (
                    vec![bacnet_types::constructed::BACnetAccessRule::new(
                        None,
                        Some(oid(ObjectType::ACCESS_POINT, 2).into()),
                        true,
                    )],
                    Vec::new(),
                ),
            };
            set_rights(&mut db, 2, None, positive, negative, true);
            let result = evaluate(&db, &[presented(2)]);
            assert_incorrect(&result, companion(2));
            assert!(matches!(
                result.companions[0].rights.decision,
                AccessRightsDecision::Denied { .. }
            ));
        }
    }
}
#[test]
fn accompaniment_companion_exemption_passes_rights_but_never_the_wrong_criterion() {
    for required in [2, 3] {
        let mut db = database(Some(credential(required).into()));
        set_assignments(&mut db, 2, Vec::new(), true);
        let result = evaluate(&db, &[presented(2)]);
        if required == 2 {
            assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
            assert_eq!(
                result.companions[0].rights.decision,
                AccessRightsDecision::Exempt
            );
        } else {
            assert_incorrect(&result, companion(2));
        }
    }
}
#[test]
fn accompaniment_user_criterion_uses_direct_credentials_only() {
    for owns in [true, false] {
        let mut db = database(Some(user(1).into()));
        add_user(&mut db, vec![credential(if owns { 2 } else { 3 }).into()]);
        let result = evaluate(&db, &[presented(2)]);
        if owns {
            assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        } else {
            assert_incorrect(&result, companion(2));
        }
    }
    let mut db = database(Some(user(1).into()));
    let mut parent = AccessUserObject::new(1, "parent").unwrap();
    parent.set_members([user(2)]).unwrap();
    db.add(Box::new(parent)).unwrap();
    let mut child = AccessUserObject::new(2, "child").unwrap();
    child.set_credentials([credential(2)]).unwrap();
    db.add(Box::new(child)).unwrap();
    assert_incorrect(&evaluate(&db, &[presented(2)]), companion(2));
}
#[test]
fn accompaniment_explicit_own_device_and_unqualified_references_normalize() {
    for kind in [
        ObjectType::ACCESS_RIGHTS,
        ObjectType::ACCESS_CREDENTIAL,
        ObjectType::ACCESS_USER,
    ] {
        for qualify_requirement in [false, true] {
            for qualify_member in [false, true] {
                let target = oid(
                    kind,
                    if kind == ObjectType::ACCESS_USER {
                        1
                    } else {
                        2
                    },
                );
                let requirement = if qualify_requirement {
                    qualified(target, device(100))
                } else {
                    target.into()
                };
                let mut db = database(Some(requirement));
                add_device(&mut db);
                let member = |object| {
                    if qualify_member {
                        qualified(object, device(100))
                    } else {
                        object.into()
                    }
                };
                set_assignments(&mut db, 2, vec![assignment(member(rights(2)), true)], false);
                if kind == ObjectType::ACCESS_USER {
                    add_user(&mut db, vec![member(credential(2))]);
                }
                assert_eq!(
                    evaluate(&db, &[presented(2)]).outcome,
                    AccompanimentOutcome::Satisfied,
                    "{kind:?}/{qualify_requirement}/{qualify_member}"
                );
            }
        }
    }
}
#[test]
fn accompaniment_rights_diagnostics_preserve_existing_decision_policy() {
    let mut db = database(Some(credential(2).into()));
    let absent = assignment(rights(90), true);
    let remote = assignment(qualified(rights(91), device(99)), true);
    override_read(
        &mut db,
        rights(3),
        PropertyIdentifier::POSITIVE_ACCESS_RULES,
        Read::Failure(
            bacnet_types::enums::ErrorClass::PROPERTY,
            bacnet_types::enums::ErrorCode::READ_ACCESS_DENIED,
        ),
        true,
    );
    for n in [1, 2] {
        set_assignments(
            &mut db,
            n,
            vec![
                absent.clone(),
                remote.clone(),
                assignment(rights(3), true),
                assignment(rights(n), true),
            ],
            false,
        );
    }
    let original_rights = evaluate_access_rights(&db, credential(1), point()).unwrap();
    let companion_rights = evaluate_access_rights(&db, credential(2), point()).unwrap();
    assert_eq!(original_rights.unresolved.len(), 3);
    assert_eq!(companion_rights.unresolved.len(), 3);
    let result = evaluate(&db, &[presented(2)]);
    assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
    assert_eq!(result.original_rights, original_rights);
    assert_eq!(result.companions[0].rights, companion_rights);
    assert_eq!(result.events, [by(companion(2))]);
}

#[test]
fn accompaniment_remote_unused_or_disabled_assignments_cannot_match_local_rights() {
    for ineffective in [
        assignment(qualified(rights(3), device(99)), true),
        assignment(
            qualified(rights(3), device(ObjectIdentifier::WILDCARD_INSTANCE)),
            true,
        ),
        assignment(rights(3), false),
        assignment(rights(ObjectIdentifier::WILDCARD_INSTANCE), true),
    ] {
        let mut db = database(Some(rights(3).into()));
        add_device(&mut db);
        set_assignments(
            &mut db,
            2,
            vec![assignment(rights(2), true), ineffective],
            false,
        );
        assert!(
            evaluate_access_rights(&db, credential(2), point())
                .unwrap()
                .decision
                .allows()
        );
        assert_incorrect(&evaluate(&db, &[presented(2)]), companion(2));
    }
}
#[test]
fn accompaniment_denied_rights_with_diagnostics_remains_denied() {
    for denied in [1, 2] {
        let mut db = database(Some(credential(2).into()));
        set_assignments(
            &mut db,
            denied,
            vec![
                assignment(rights(90), true),
                assignment(rights(denied), true),
            ],
            false,
        );
        set_rights(
            &mut db,
            denied,
            Some(credential(2).into()),
            vec![anytime()],
            vec![anytime()],
            true,
        );
        let result = evaluate(&db, &[presented(2)]);
        if denied == 1 {
            assert_eq!(result.outcome, AccompanimentOutcome::NotEvaluated);
            assert_eq!(result.original_rights.unresolved.len(), 1);
            assert!(result.events.is_empty());
        } else {
            assert_incorrect(&result, companion(2));
            assert_eq!(result.companions[0].rights.unresolved.len(), 1);
        }
    }
}

#[test]
fn accompaniment_unavailable_negative_time_or_location_remains_existing_false_rule() {
    use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectPropertyReference};
    for unavailable_time in [true, false] {
        let mut db = database(Some(credential(2).into()));
        let time = unavailable_time.then(|| {
            BACnetDeviceObjectPropertyReference::new_local(
                oid(ObjectType::SCHEDULE, 90),
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            )
        });
        let location = if unavailable_time {
            point().into()
        } else {
            qualified(point(), device(99))
        };
        let negative = BACnetAccessRule::new(time, Some(location), true);
        for n in [1, 2] {
            set_rights(
                &mut db,
                n,
                (n == 1).then(|| credential(2).into()),
                vec![anytime()],
                vec![negative.clone()],
                true,
            );
        }
        let original_rights = evaluate_access_rights(&db, credential(1), point()).unwrap();
        let companion_rights = evaluate_access_rights(&db, credential(2), point()).unwrap();
        assert!(original_rights.decision.allows() && companion_rights.decision.allows());
        assert!(original_rights.unresolved.is_empty() && companion_rights.unresolved.is_empty());
        let result = evaluate(&db, &[presented(2)]);
        assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
        assert_eq!(result.original_rights, original_rights);
        assert_eq!(result.companions[0].rights, companion_rights);
        assert_eq!(result.events, [by(companion(2))]);
    }
}
