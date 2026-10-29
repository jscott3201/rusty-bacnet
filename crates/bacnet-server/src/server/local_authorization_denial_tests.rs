//! Every concrete denial emitted by the ordered evaluator reaches the wire.
use super::boundary::*;
use super::fixture::*;
use super::*;

async fn assert_denial(
    h: &mut Harness,
    req: AuthorizationRequest<'_>,
    expected_event: AccessEvent,
) {
    let oid = req.original.point;
    subscribed(h, oid).await;
    h.set_clock(20);
    let expected = {
        let db = h.server.database().read().await;
        evaluate_authorization(&db, req).unwrap()
    };
    assert!(
        matches!(expected.outcome,AuthorizationOutcome::Denied{access_event,..} if access_event==expected_event),
        "{expected:?}"
    );
    let actual = h.server.authorize_access_local(req, 42).await.unwrap();
    assert_eq!(actual.evaluation, expected);
    assert_eq!(actual.committed_events.len(), expected.events.len());
    for (i, intent) in expected.events.iter().enumerate() {
        let credential = if intent.access_event == AccessEvent::ACCOMPANIMENT_BY {
            2
        } else {
            1
        };
        assert_eq!(
            values(&h.cov_notification().await, oid),
            expected_values(intent.access_event, 42, i as u8, credential)
        );
    }
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn local_authorization_every_credential_denial_preserves_its_specific_wire_event() {
    for (reason, event) in [
        (Reason::DISABLED, AccessEvent::DENIED_CREDENTIAL_DISABLED),
        (
            Reason::DISABLED_NEEDS_PROVISIONING,
            AccessEvent::DENIED_CREDENTIAL_NOT_PROVISIONED,
        ),
        (
            Reason::DISABLED_UNASSIGNED,
            AccessEvent::DENIED_CREDENTIAL_UNASSIGNED,
        ),
        (
            Reason::DISABLED_NOT_YET_ACTIVE,
            AccessEvent::DENIED_CREDENTIAL_NOT_YET_ACTIVE,
        ),
        (
            Reason::DISABLED_EXPIRED,
            AccessEvent::DENIED_CREDENTIAL_EXPIRED,
        ),
        (
            Reason::DISABLED_LOCKOUT,
            AccessEvent::DENIED_CREDENTIAL_LOCKOUT,
        ),
        (
            Reason::DISABLED_MAX_DAYS,
            AccessEvent::DENIED_CREDENTIAL_MAX_DAYS,
        ),
        (
            Reason::DISABLED_MAX_USES,
            AccessEvent::DENIED_CREDENTIAL_MAX_USES,
        ),
        (
            Reason::DISABLED_INACTIVITY,
            AccessEvent::DENIED_CREDENTIAL_INACTIVITY,
        ),
        (
            Reason::DISABLED_MANUAL,
            AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
        ),
    ] {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            native_objects(db);
            let mut overrides = vec![
                (P::CREDENTIAL_STATUS, PropertyValue::Enumerated(0)),
                (
                    P::REASON_FOR_DISABLE,
                    PropertyValue::List(vec![PropertyValue::Enumerated(reason.to_raw())]),
                ),
            ];
            if reason == Reason::DISABLED_NOT_YET_ACTIVE || reason == Reason::DISABLED_EXPIRED {
                let property = if reason == Reason::DISABLED_NOT_YET_ACTIVE {
                    P::ACTIVATION_TIME
                } else {
                    P::EXPIRATION_TIME
                };
                let frame = at(if reason == Reason::DISABLED_NOT_YET_ACTIVE {
                    30
                } else {
                    10
                });
                overrides.push((
                    property,
                    PropertyValue::List(vec![
                        PropertyValue::Date(frame.local_date),
                        PropertyValue::Time(frame.local_time),
                    ]),
                ));
            }
            override_values(db, card_id(1), overrides);
        })
        .await;
        let mut req = request(false);
        req.wall_time = Some(at(20));
        assert_denial(&mut h, req, event).await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_rights_threat_occupancy_passback_lockout_mode_denials_reach_wire() {
    for (case, event) in [
        (0, AccessEvent::DENIED_NO_ACCESS_RIGHTS),
        (1, AccessEvent::DENIED_THREAT_LEVEL),
        (2, AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT),
        (3, AccessEvent::DENIED_LOWER_OCCUPANCY_LIMIT),
        (4, AccessEvent::DENIED_PASSBACK),
        (5, AccessEvent::DENIED_LOCKOUT),
        (6, AccessEvent::DENIED_DENY_ALL),
    ] {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            configured(
                db,
                if case == 6 {
                    AuthMode::DENY_ALL
                } else {
                    AuthMode::AUTHORIZE
                },
                false,
                false,
            );
            if case == 0 {
                let card = AccessCredentialObject::new(1, "no rights").unwrap();
                db.add(Box::new(card)).unwrap();
            }
            if case == 3 {
                override_values(
                    db,
                    identifier(ObjectType::ACCESS_ZONE),
                    vec![(P::OCCUPANCY_COUNT, PropertyValue::Unsigned(1))],
                );
            }
        })
        .await;
        let mut req = request(case == 5);
        if case == 1 {
            req.fallback_threat_level = AuthorizationFact::Known(1);
        }
        let zones = [AuthorizationOccupancyZone {
            zone: identifier(ObjectType::ACCESS_ZONE),
            direction: if case == 3 {
                AuthorizationDirection::Leaving
            } else {
                AuthorizationDirection::Entering
            },
            fallback_enforced: AuthorizationFact::Known(true),
        }];
        if case == 2 || case == 3 {
            req.occupancy = AuthorizationOccupancy::Zones(&zones);
        }
        if case == 4 {
            req.passback = AuthorizationPassback::Zone {
                zone: identifier(ObjectType::ACCESS_ZONE),
                fallback_mode: AuthorizationFact::Known(Passback::HARD_PASSBACK),
                fallback_timeout_minutes: None,
                observation: AuthorizationFact::Known(AuthorizationPassbackObservation {
                    presentation: req.original,
                    zone: identifier(ObjectType::ACCESS_ZONE),
                    violation: true,
                    last_successful_entry: None,
                }),
            };
        }
        assert_denial(&mut h, req, event).await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_accompaniment_and_external_phase_denials_reach_wire() {
    for (case, event) in [
        (0, AccessEvent::DENIED_NO_ACCOMPANIMENT),
        (1, AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT),
        (2, AccessEvent::DENIED_VERIFICATION_FAILED),
        (3, AccessEvent::DENIED_VERIFICATION_TIMEOUT),
    ] {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            configured(
                db,
                if case >= 2 {
                    AuthMode::VERIFICATION_REQUIRED
                } else {
                    AuthMode::AUTHORIZE
                },
                case < 2,
                false,
            )
        })
        .await;
        let mut req = request(false);
        let witnesses = [AccompanimentWitness {
            original: 10,
            evidence: if case == 0 {
                AccompanimentEvidence::Waiting
            } else {
                AccompanimentEvidence::Presented(companion())
            },
        }];
        let authentication = [CompanionAuthenticationAttestation {
            presentation: companion(),
            authentication: CompanionAuthentication::Rejected,
        }];
        if case < 2 {
            req.accompaniment_witnesses = &witnesses;
            req.companion_authentication = &authentication;
            if case == 0 {
                req.now = Duration::from_secs(111);
            }
        } else {
            req.now = Duration::from_secs(if case == 2 { 105 } else { 111 });
            phase(
                &mut req,
                if case == 2 {
                    AuthorizationVerdict::Rejected(Duration::from_secs(104))
                } else {
                    AuthorizationVerdict::Waiting
                },
            );
        }
        assert_denial(&mut h, req, event).await;
        h.server.stop().await.unwrap();
    }
}
