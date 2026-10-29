//! Route-level refusal and observable clockless triggers.
use super::fixture::*;
use super::*;
use bacnet_types::error::RemoteError;

#[tokio::test(start_paused = true)]
async fn local_authorization_exhausted_second_stamp_refuses_without_mutation_or_capture() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        configured(db, AuthMode::AUTHORIZE, false, false);
        let end = bacnet_types::calendar::SpecificDate::new(2154, 12, 31)
            .unwrap()
            .to_date();
        db.get_mut(&identifier(ObjectType::ACCESS_POINT))
            .unwrap()
            .report_access_input_internal(AccessControlInput::AccessEvent(AccessEventReport {
                time: Some(BACnetTimeStamp::DateTime {
                    date: end,
                    time: bacnet_types::primitives::Time {
                        hour: 23,
                        minute: 59,
                        second: 59,
                        hundredths: 98,
                    },
                }),
                credential: Some(card_id(1).into()),
                ..AccessEventReport::new(AccessEvent::GRANTED, 1)
            }))
            .unwrap();
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
    let evaluation = {
        let db = h.server.database().read().await;
        evaluate_authorization(&db, req).unwrap()
    };
    assert_eq!(evaluation.outcome, AuthorizationOutcome::Granted);
    assert_eq!(evaluation.events.len(), 2);
    assert!(matches!(
        h.server.authorize_access_local(req, 42).await,
        Err(Error::Remote(RemoteError::Error {
            code: ErrorCode::VALUE_OUT_OF_RANGE,
            ..
        }))
    ));
    assert_eq!(snapshot(&h).await, before);
    h.no_notification().await;
    // One remaining distinct instant is still usable: the failed two-event
    // attempt did not consume it or poison a timed capture baseline.
    let result = h
        .server
        .authorize_access_local(request(false), 43)
        .await
        .unwrap();
    assert_eq!(result.committed_events.len(), 1);
    assert_eq!(result.committed_events[0].tag, 43);
    let ordinary = values(&h.cov_notification().await, oid);
    assert!(ordinary.contains(&(P::ACCESS_EVENT_TAG, unsigned(43))));
    let multiple = h.notification().await;
    assert!(
        multiple
            .list_of_cov_notifications
            .iter()
            .flat_map(|item| &item.list_of_values)
            .any(|v| v.property_identifier == P::ACCESS_EVENT_TIME
                && ordinary.contains(&(P::ACCESS_EVENT_TIME, v.value.clone())))
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_clockless_wrapped_same_event_code_still_notifies_each_attempt() {
    let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    {
        let mut db = h.server.database().write().await;
        db.set_clock_reader(None);
        db.get_mut(&oid)
            .unwrap()
            .report_access_input_internal(AccessControlInput::AccessEvent(AccessEventReport {
                time: Some(BACnetTimeStamp::SequenceNumber(65535)),
                credential: Some(card_id(1).into()),
                ..AccessEventReport::new(AccessEvent::GRANTED, 42)
            }))
            .unwrap();
    }
    subscribed(&mut h, oid).await;
    for sequence in [1, 2] {
        let result = h
            .server
            .authorize_access_local(request(false), 42)
            .await
            .unwrap();
        assert_eq!(result.committed_events[0].event, AccessEvent::GRANTED);
        assert_eq!(
            result.committed_events[0].time,
            Some(BACnetTimeStamp::SequenceNumber(sequence))
        );
        let actual = values(&h.cov_notification().await, oid);
        assert!(actual.contains(&(P::ACCESS_EVENT, enumerated(AccessEvent::GRANTED.to_raw()))));
        assert!(actual.contains(&(P::ACCESS_EVENT_TIME, vec![0x19, sequence as u8])));
    }
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
