//! Unavailable local evidence is distinct from mismatch, absence and expiry.
use super::accompaniment_test_support::*;
use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use std::time::Duration;

#[test]
fn accompaniment_all_requirement_kinds_reject_remote_wildcard_and_missing_targets() {
    for kind in [
        ObjectType::ACCESS_RIGHTS,
        ObjectType::ACCESS_CREDENTIAL,
        ObjectType::ACCESS_USER,
    ] {
        let local = oid(
            kind,
            if kind == ObjectType::ACCESS_USER {
                1
            } else {
                2
            },
        );
        for (reference, reason) in [
            (
                qualified(local, device(99)),
                AccompanimentReferenceFailure::Remote,
            ),
            (
                qualified(local, device(ObjectIdentifier::WILDCARD_INSTANCE)),
                AccompanimentReferenceFailure::Wildcard,
            ),
            (oid(kind, 90).into(), AccompanimentReferenceFailure::Missing),
        ] {
            let mut db = database(Some(reference.clone()));
            add_device(&mut db);
            add_user(&mut db, vec![credential(2).into()]);
            let result = evaluate(&db, &[presented(2)]);
            assert_eq!(
                result.outcome,
                AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
                    reference,
                    reason
                })
            );
            assert_eq!(result.events, [by(companion(2))]);
        }
    }
}
#[test]
fn accompaniment_unresolvable_requirement_cannot_turn_waiting_or_expiry_into_denial() {
    for kind in [
        ObjectType::ACCESS_RIGHTS,
        ObjectType::ACCESS_CREDENTIAL,
        ObjectType::ACCESS_USER,
    ] {
        for (reference, reason) in [
            (
                qualified(oid(kind, 2), device(99)),
                AccompanimentReferenceFailure::Remote,
            ),
            (
                qualified(oid(kind, 2), device(ObjectIdentifier::WILDCARD_INSTANCE)),
                AccompanimentReferenceFailure::Wildcard,
            ),
            (oid(kind, 90).into(), AccompanimentReferenceFailure::Missing),
        ] {
            for (evidence, now) in [
                (AccompanimentEvidence::Waiting, 101),
                (AccompanimentEvidence::Waiting, 120),
                (AccompanimentEvidence::DeadlineReached, 120),
                (
                    AccompanimentEvidence::Presented(CredentialPresentation {
                        at: Duration::from_secs(120),
                        ..companion(2)
                    }),
                    120,
                ),
            ] {
                let db = database(Some(reference.clone()));
                let witnesses = [AccompanimentWitness {
                    original: 10,
                    evidence,
                }];
                let result = evaluate_accompaniment(
                    &db,
                    AccompanimentRequest {
                        now: Duration::from_secs(now),
                        ..request(&witnesses)
                    },
                )
                .unwrap();
                assert_eq!(
                    result.outcome,
                    AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
                        reference: reference.clone(),
                        reason
                    })
                );
                assert!(result.events.is_empty());
            }
        }
    }
}
#[test]
fn accompaniment_no_local_device_never_substitutes_an_explicit_device_reference() {
    let requirement = qualified(credential(2), device(100));
    let result = evaluate(&database(Some(requirement.clone())), &[presented(2)]);
    assert_eq!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
            reference: requirement,
            reason: AccompanimentReferenceFailure::Remote
        })
    );
    assert_eq!(
        evaluate(&database(Some(credential(2).into())), &[presented(2)]).outcome,
        AccompanimentOutcome::Satisfied
    );
}
#[test]
fn accompaniment_unreadable_selected_property_is_not_absence() {
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
        (Read::Value(PropertyValue::Boolean(false)), true, false),
        (
            Read::Value(PropertyValue::ApplicationData(vec![0x1c])),
            true,
            false,
        ),
    ] {
        let mut db = database(None);
        override_read(
            &mut db,
            rights(1),
            PropertyIdentifier::ACCOMPANIMENT,
            read,
            advertised,
        );
        let result = evaluate(&db, &[presented(2)]);
        if absent {
            assert_eq!(result.outcome, AccompanimentOutcome::NotRequired);
        } else {
            assert!(matches!(
                result.outcome,
                AccompanimentOutcome::Unresolved(AccompanimentProblem::Property {
                    property: PropertyIdentifier::ACCOMPANIMENT,
                    ..
                })
            ));
        }
        assert!(result.events.is_empty());
    }
}
#[test]
fn accompaniment_reference_decode_rejects_trailing_data_wrong_kind_and_wrong_device() {
    let valid = encoded_reference(&credential(2).into());
    let mut trailing = valid.clone();
    let PropertyValue::ApplicationData(ref mut bytes) = trailing else {
        panic!()
    };
    bytes.push(0);
    let invalid_device = encoded_reference(&qualified(credential(2), point()));
    for value in [trailing, invalid_device] {
        let mut db = database(None);
        override_read(
            &mut db,
            rights(1),
            PropertyIdentifier::ACCOMPANIMENT,
            Read::Value(value),
            true,
        );
        assert!(matches!(
            evaluate(&db, &[presented(2)]).outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Property { .. })
        ));
    }
    let mut db = database(None);
    override_read(
        &mut db,
        rights(1),
        PropertyIdentifier::ACCOMPANIMENT,
        Read::Value(encoded_reference(&point().into())),
        true,
    );
    assert!(matches!(
        evaluate(&db, &[presented(2)]).outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
            reason: AccompanimentReferenceFailure::WrongType,
            ..
        })
    ));
}
#[test]
fn accompaniment_unreadable_rights_criterion_enable_or_assignment_is_unresolved() {
    for (object, property, read) in [
        (
            rights(3),
            PropertyIdentifier::LOG_ENABLE,
            Read::Value(PropertyValue::Unsigned(1)),
        ),
        (
            rights(3),
            PropertyIdentifier::LOG_ENABLE,
            Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
        ),
        (
            credential(2),
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            Read::Value(PropertyValue::Boolean(true)),
        ),
        (
            credential(2),
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            Read::Value(PropertyValue::ApplicationData(vec![0x0e])),
        ),
    ] {
        let mut db = database(Some(rights(3).into()));
        override_read(&mut db, object, property, read, true);
        let result = evaluate(&db, &[presented(2)]);
        assert!(matches!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Property { .. })
        ));
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_user_credentials_must_be_readable_complete_and_correctly_typed() {
    for read in [
        Read::Value(PropertyValue::Boolean(true)),
        Read::Value(PropertyValue::ApplicationData(vec![0x1c])),
        Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
        Read::Value(encoded_reference(&rights(2).into())),
    ] {
        let mut db = database(Some(user(1).into()));
        add_user(&mut db, vec![credential(2).into()]);
        override_read(
            &mut db,
            user(1),
            PropertyIdentifier::CREDENTIALS,
            read,
            true,
        );
        let result = evaluate(&db, &[presented(2)]);
        assert!(matches!(
            result.outcome,
            AccompanimentOutcome::Unresolved(_)
        ));
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_user_remote_or_wildcard_membership_never_matches_same_local_oid() {
    for device_id in [device(99), device(ObjectIdentifier::WILDCARD_INSTANCE)] {
        let mut db = database(Some(user(1).into()));
        add_device(&mut db);
        add_user(&mut db, vec![qualified(credential(2), device_id)]);
        let result = evaluate(&db, &[presented(2)]);
        assert!(matches!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference { .. })
        ));
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_missing_or_unreadable_companion_rights_remains_unresolved() {
    for missing in [false, true] {
        let mut db = database(Some(user(1).into()));
        add_user(&mut db, vec![credential(2).into()]);
        if missing {
            db.remove(&credential(2)).unwrap();
        } else {
            override_read(
                &mut db,
                credential(2),
                PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
                Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
                true,
            );
        }
        let result = evaluate(&db, &[presented(2)]);
        assert!(matches!(
            result.outcome,
            AccompanimentOutcome::Unresolved(AccompanimentProblem::Rights {
                presentation: 11,
                ..
            })
        ));
        assert_eq!(result.events, [by(companion(2))]);
    }
}
#[test]
fn accompaniment_wrong_type_candidate_cannot_create_an_invalid_credential_event() {
    let db = database(Some(credential(2).into()));
    let candidate = CredentialPresentation {
        credential: rights(2),
        ..companion(2)
    };
    let witnesses = [AccompanimentWitness {
        original: 10,
        evidence: AccompanimentEvidence::Presented(candidate),
    }];
    let result = evaluate(&db, &witnesses);
    assert!(matches!(
        result.outcome,
        AccompanimentOutcome::Unresolved(AccompanimentProblem::Reference {
            reason: AccompanimentReferenceFailure::WrongType,
            ..
        })
    ));
    assert!(result.events.is_empty());
}
#[test]
fn accompaniment_original_rights_errors_are_preserved() {
    let mut db = database(None);
    override_read(
        &mut db,
        credential(1),
        PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
        Read::Failure(ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED),
        true,
    );
    assert!(matches!(
        evaluate_accompaniment(&db, request(&[])),
        Err(Error::Remote(RemoteError::Error {
            code: ErrorCode::READ_ACCESS_DENIED,
            ..
        }))
    ));
    let mut wrong = request(&[]);
    wrong.original.credential = rights(1);
    assert!(matches!(
        evaluate_accompaniment(&db, wrong),
        Err(Error::Remote(RemoteError::Error {
            code: ErrorCode::UNKNOWN_OBJECT,
            ..
        }))
    ));
    db.remove(&point()).unwrap();
    assert!(matches!(
        evaluate_accompaniment(&db, request(&[])),
        Err(Error::Remote(RemoteError::Error {
            code: ErrorCode::UNKNOWN_OBJECT,
            ..
        }))
    ));
}
