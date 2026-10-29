use super::boundary::*;
use super::fixture::*;
use super::*;
use bacnet_types::error::RemoteError;

#[tokio::test(start_paused = true)]
async fn local_authorization_nonterminal_intermediate_intents_never_publish_even_on_retry() {
    for selected in [
        AuthMode::VERIFICATION_REQUIRED,
        AuthMode::AUTHORIZATION_DELAYED,
        AuthMode::NONE,
    ] {
        for unresolved in [false, true] {
            let mut h = Harness::start_with(ServerConfig::default(), |db| {
                configured(db, selected, false, true)
            })
            .await;
            let oid = identifier(ObjectType::ACCESS_POINT);
            subscribed(&mut h, oid).await;
            h.subscribe_with_delay(false, vec![(oid, vec![(P::ACCESS_EVENT_TIME, true)])], 0)
                .await;
            assert_eq!(response(&h).await, Ok(()));
            h.notification().await;
            h.set_clock(20);
            let before = snapshot(&h).await;
            let mut req = request(false);
            soft(&mut req);
            if !unresolved {
                phase(&mut req, AuthorizationVerdict::Waiting);
            }
            let expected = {
                let db = h.server.database().read().await;
                evaluate_authorization(&db, req).unwrap()
            };
            if selected == AuthMode::NONE {
                assert_eq!(expected.outcome, AuthorizationOutcome::NoAuthorization);
                assert!(expected.events.is_empty());
            } else {
                assert!(
                    expected
                        .events
                        .iter()
                        .any(|e| e.access_event == AccessEvent::PASSBACK_DETECTED)
                );
                assert!(if unresolved {
                    matches!(expected.outcome, AuthorizationOutcome::Unresolved { .. })
                } else {
                    matches!(expected.outcome, AuthorizationOutcome::Pending(_))
                });
            }
            for _ in 0..2 {
                let actual = h.server.authorize_access_local(req, 42).await.unwrap();
                assert_eq!(actual.evaluation, expected);
                assert!(actual.committed_events.is_empty());
                assert_eq!(snapshot(&h).await, before);
                h.no_notification().await;
            }
            h.server.stop().await.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_preserves_rights_policy_optional_reporting_and_retry_evidence() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        configured(db, AuthMode::AUTHORIZE, false, true);
        let mut card = AccessCredentialObject::new(1, "card-with-unresolved-rights").unwrap();
        card.set_assigned_access_rights(
            [99, 1]
                .map(|n| BACnetAssignedAccessRights {
                    assigned_access_rights: ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, n)
                        .unwrap()
                        .into(),
                    enable: true,
                })
                .to_vec(),
        )
        .unwrap();
        card.set_authorization_exemptions(Some(vec![Exemption::PASSBACK]))
            .unwrap();
        db.add(Box::new(card)).unwrap();
    })
    .await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    subscribed(&mut h, oid).await;
    h.set_clock(20);
    let mut req = request(false);
    req.passback = AuthorizationPassback::Unknown;
    for policy in [
        AuthorizationRightsPolicy::RequireCompleteLocalEvidence,
        AuthorizationRightsPolicy::HonorExistingDecision,
    ] {
        req.rights_policy = policy;
        let expected = {
            let db = h.server.database().read().await;
            evaluate_authorization(&db, req).unwrap()
        };
        assert_eq!(expected.rights.as_ref().unwrap().unresolved.len(), 1);
        let before = snapshot(&h).await;
        let actual = h.server.authorize_access_local(req, 42).await.unwrap();
        assert_eq!(actual.evaluation, expected);
        if policy == AuthorizationRightsPolicy::RequireCompleteLocalEvidence {
            assert!(actual.committed_events.is_empty());
            assert_eq!(snapshot(&h).await, before);
            h.no_notification().await;
        } else {
            assert_eq!(expected.outcome, AuthorizationOutcome::Granted);
            assert_eq!(
                expected.passback_reporting_problem,
                Some(AuthorizationProblem::MissingFact("passback applicability"))
            );
            assert_eq!(
                values(&h.cov_notification().await, oid),
                expected_values(AccessEvent::GRANTED, 42, 0, 1)
            );
            let retry = h.server.authorize_access_local(req, 42).await.unwrap();
            assert_eq!(retry.evaluation, expected);
            assert_ne!(
                retry.committed_events[0].time,
                actual.committed_events[0].time
            );
            assert_eq!(
                values(&h.cov_notification().await, oid),
                expected_values(AccessEvent::GRANTED, 42, 1, 1)
            );
        }
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_order_controls_the_first_denial_without_early_rights_grant() {
    for lockout_first in [false, true] {
        let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
        let oid = identifier(ObjectType::ACCESS_POINT);
        subscribed(&mut h, oid).await;
        h.set_clock(20);
        let mut req = request(true);
        req.fallback_threat_level = AuthorizationFact::Known(1);
        if lockout_first {
            use AuthorizationCheck::*;
            req.order = AuthorizationOrder::new([
                Credential,
                AccessRights,
                Lockout,
                Threat,
                Occupancy,
                Passback,
                Mode,
            ])
            .unwrap();
        }
        let event = if lockout_first {
            AccessEvent::DENIED_LOCKOUT
        } else {
            AccessEvent::DENIED_THREAT_LEVEL
        };
        let actual = h.server.authorize_access_local(req, 42).await.unwrap();
        assert!(matches!(
            actual.evaluation.rights.as_ref().unwrap().decision,
            AccessRightsDecision::Granted { .. }
        ));
        assert_eq!(
            actual
                .committed_events
                .iter()
                .map(|e| e.event)
                .collect::<Vec<_>>(),
            [event]
        );
        assert_eq!(
            values(&h.cov_notification().await, oid),
            expected_values(event, 42, 0, 1)
        );
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_admission_refuses_before_evaluation_and_healthy_control_progresses() {
    for failure in 0..5 {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = Arc::clone(&reads);
        let mut h = Harness::start_with(ServerConfig::default(), move |db| {
            native_objects(db);
            let mut wrapper = Boundary::wrap(db, identifier(ObjectType::ACCESS_POINT));
            wrapper.mode_reads = observed;
            match failure {
                0 => wrapper.supports = false,
                1 => wrapper
                    .overrides
                    .push((P::OUT_OF_SERVICE, PropertyValue::Boolean(true))),
                2 => wrapper.overrides.push((
                    P::RELIABILITY,
                    PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw()),
                )),
                3 => wrapper
                    .overrides
                    .push((P::OUT_OF_SERVICE, PropertyValue::Null)),
                _ => wrapper
                    .overrides
                    .push((P::RELIABILITY, PropertyValue::Boolean(false))),
            }
            db.add(Box::new(wrapper)).unwrap();
        })
        .await;
        let oid = identifier(ObjectType::ACCESS_POINT);
        subscribed(&mut h, oid).await;
        h.set_clock(20);
        let before = snapshot(&h).await;
        let expected_code = match failure {
            0 => ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            1 | 2 => ErrorCode::WRITE_ACCESS_DENIED,
            _ => ErrorCode::INVALID_DATA_TYPE,
        };
        assert!(
            matches!(h.server.authorize_access_local(request(false),42).await,Err(Error::Remote(RemoteError::Error {code,..})) if code==expected_code)
        );
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot(&h).await, before);
        h.no_notification().await;
        // Replace this test-only reader with a healthy native object; the next
        // operation must progress, so refusal cannot leave a guard or waiter.
        {
            let mut db = h.server.database().write().await;
            db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
        }
        assert_eq!(
            h.server
                .authorize_access_local(request(false), 42)
                .await
                .unwrap()
                .evaluation
                .outcome,
            AuthorizationOutcome::Granted
        );
        h.cov_notification().await;
        h.server.stop().await.unwrap();
    }
    let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
    for oid in [
        ObjectIdentifier::new(ObjectType::ACCESS_POINT, 999).unwrap(),
        av1(),
    ] {
        let mut req = request(false);
        req.original.point = oid;
        assert!(h.server.authorize_access_local(req, 42).await.is_err());
    }
    h.server.stop().await.unwrap();
    assert!(
        h.server
            .authorize_access_local(request(false), 42)
            .await
            .is_err()
    );
}
