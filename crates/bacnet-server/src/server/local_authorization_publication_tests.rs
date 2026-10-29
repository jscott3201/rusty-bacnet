//! Retained event states travel through each existing COV service.
use super::fixture::*;
use super::*;
use bacnet_services::cov::SubscribeCOVPropertyRequest;

async fn selected(h: &mut Harness) {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1569,
        monitored_object_identifier: identifier(ObjectType::ACCESS_POINT),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: P::ACCESS_EVENT_TIME,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    assert_eq!(response(h).await, Ok(()));
    h.cov_notification().await;
}

#[tokio::test(start_paused = true)]
async fn local_authorization_four_intents_retain_fields_in_every_cov_flavor() {
    for flavor in 0..4 {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            configured(db, AuthMode::VERIFICATION_REQUIRED, true, false)
        })
        .await;
        let oid = identifier(ObjectType::ACCESS_POINT);
        match flavor {
            0 => {
                subscribed(&mut h, oid).await;
            }
            1 => selected(&mut h).await,
            _ => {
                let refs = expected_values(AccessEvent::GRANTED, 1, 0, 1)
                    .into_iter()
                    .map(|(p, _)| (p, flavor == 3))
                    .collect();
                h.subscribe_with_delay(false, vec![(oid, refs)], 0).await;
                assert_eq!(response(&h).await, Ok(()));
                h.notification().await;
            }
        }
        h.set_clock(20);
        let witnesses = [AccompanimentWitness {
            original: 10,
            evidence: AccompanimentEvidence::Presented(companion()),
        }];
        let authentication = [CompanionAuthenticationAttestation {
            presentation: companion(),
            authentication: CompanionAuthentication::Authenticated,
        }];
        let mut req = request(false);
        req.now = Duration::from_secs(105);
        // Make SOFT precede BY, independently of the ordinary request fixture.
        use AuthorizationCheck::*;
        req.order = AuthorizationOrder::new([
            Credential,
            Passback,
            AccessRights,
            Threat,
            Occupancy,
            Lockout,
            Mode,
        ])
        .unwrap();
        req.accompaniment_witnesses = &witnesses;
        req.companion_authentication = &authentication;
        soft(&mut req);
        phase(
            &mut req,
            AuthorizationVerdict::Approved(Duration::from_secs(104)),
        );
        let (entered, release) = h.hold_notification_barrier(flavor < 2);
        let mut route = Box::pin(h.server.authorize_access_local(req, 42));
        use std::future::Future;
        assert!(
            route
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        // A transport barrier proves the batch committed and its task owns all
        // remaining snapshots before another transaction mutates the live point.
        entered.acquire().await.unwrap().forget();
        {
            let mut db = h.server.database().write().await;
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(P::ACCESS_EVENT_TAG, None)
                    .unwrap(),
                PropertyValue::Unsigned(42)
            );
            db.get_mut(&oid)
                .unwrap()
                .report_access_input_internal(AccessControlInput::AccessEvent(access_event(
                    AccessEvent::DENIED_OTHER,
                    99,
                    30,
                )))
                .unwrap();
        }
        release.add_permits(1);
        let result = route.await.unwrap();
        let expected = [
            AccessEvent::PASSBACK_DETECTED,
            AccessEvent::ACCOMPANIMENT_BY,
            AccessEvent::VERIFICATION_REQUIRED,
            AccessEvent::GRANTED,
        ];
        assert_eq!(result.evaluation.outcome, AuthorizationOutcome::Granted);
        assert_eq!(
            result
                .committed_events
                .iter()
                .map(|r| r.event)
                .collect::<Vec<_>>(),
            expected
        );
        for (index, record) in result.committed_events.iter().enumerate() {
            let credential = if index == 1 { 2 } else { 1 };
            let wanted = expected_values(expected[index], 42, index as u8, credential);
            assert_eq!(record.credential, Some(card_id(credential.into()).into()));
            assert_eq!(record.authentication_factor, None);
            assert_eq!(record.tag, 42);
            let mut expected_time = time(20);
            expected_time.hundredths = index as u8;
            assert_eq!(
                record.time,
                Some(BACnetTimeStamp::DateTime {
                    date: at(0).local_date,
                    time: expected_time
                })
            );
            if flavor < 2 {
                let actual = values(&h.cov_notification().await, oid);
                if flavor == 0 {
                    assert_eq!(actual, wanted);
                } else {
                    assert!(actual.contains(&wanted[3]), "{actual:?}");
                }
            }
        }
        if flavor >= 2 {
            let mut all = Vec::new();
            while all
                .iter()
                .filter(|v: &&bacnet_services::cov_multiple::COVNotificationValue| {
                    v.property_identifier == P::ACCESS_EVENT_TIME
                })
                .count()
                < 4
            {
                let report = h.notification().await;
                for item in report.list_of_cov_notifications {
                    assert_eq!(item.monitored_object_identifier, oid);
                    for value in &item.list_of_values {
                        assert_eq!(
                            value.time_of_change,
                            if flavor == 3 { Some(time(20)) } else { None }
                        );
                    }
                    all.extend(item.list_of_values);
                }
            }
            // Timed queues retain changes only: the unchanged tag/status/factor
            // appear once. Every event, trigger, and changing credential remains.
            for property in [P::ACCESS_EVENT, P::ACCESS_EVENT_TIME] {
                let actual: Vec<_> = all
                    .iter()
                    .filter(|v| v.property_identifier == property)
                    .map(|v| v.value.clone())
                    .collect();
                let wanted: Vec<_> = expected
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        expected_values(*e, 42, i as u8, if i == 1 { 2 } else { 1 })
                            .into_iter()
                            .find(|(p, _)| *p == property)
                            .unwrap()
                            .1
                    })
                    .collect();
                assert_eq!(actual, wanted);
            }
            let actual: Vec<_> = all
                .iter()
                .filter(|v| v.property_identifier == P::ACCESS_EVENT_CREDENTIAL)
                .map(|v| v.value.clone())
                .collect();
            let wanted = vec![credential_bytes(2), credential_bytes(1)];
            assert_eq!(actual, wanted);
        }
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_by_and_exempt_soft_precede_later_denial() {
    for accompaniment in [false, true] {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            configured(db, AuthMode::AUTHORIZE, accompaniment, !accompaniment)
        })
        .await;
        let oid = identifier(ObjectType::ACCESS_POINT);
        subscribed(&mut h, oid).await;
        h.set_clock(20);
        let witnesses = [AccompanimentWitness {
            original: 10,
            evidence: AccompanimentEvidence::Presented(companion()),
        }];
        let authentication = [CompanionAuthenticationAttestation {
            presentation: companion(),
            authentication: CompanionAuthentication::Authenticated,
        }];
        let mut req = request(true);
        if accompaniment {
            req.accompaniment_witnesses = &witnesses;
            req.companion_authentication = &authentication;
        } else {
            soft(&mut req);
        }
        let result = h.server.authorize_access_local(req, 42).await.unwrap();
        let intermediate = if accompaniment {
            AccessEvent::ACCOMPANIMENT_BY
        } else {
            AccessEvent::PASSBACK_DETECTED
        };
        assert_eq!(
            result
                .committed_events
                .iter()
                .map(|e| e.event)
                .collect::<Vec<_>>(),
            [intermediate, AccessEvent::DENIED_LOCKOUT]
        );
        for (i, event) in [intermediate, AccessEvent::DENIED_LOCKOUT]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                values(&h.cov_notification().await, oid),
                expected_values(
                    event,
                    42,
                    i as u8,
                    if accompaniment && i == 0 { 2 } else { 1 }
                )
            );
        }
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}
