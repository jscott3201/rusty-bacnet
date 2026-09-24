use super::*;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;

fn remaining(frame: Bytes, kind: CovNotificationKind) -> u32 {
    let apdu = decode_apdu(decode_npdu(frame).unwrap().payload).unwrap();
    let bytes = match apdu {
        Apdu::ConfirmedRequest(request) => request.service_request,
        Apdu::UnconfirmedRequest(request) => request.service_request,
        other => panic!("unexpected {other:?}"),
    };
    match kind {
        CovNotificationKind::Single => {
            COVNotificationRequest::decode(&bytes)
                .unwrap()
                .time_remaining
        }
        CovNotificationKind::Multiple => {
            COVNotificationMultipleRequest::decode(&bytes)
                .unwrap()
                .time_remaining
        }
    }
}

async fn wait_until_expired(expiry: Instant) {
    // Runtime interleaving only: boundary arithmetic is tested with supplied now.
    tokio::time::timeout(Duration::from_secs(2), async {
        while Instant::now() < expiry {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cov_lifetime_held_initial_and_fanout_expiry_admit_nothing() {
    for initial in [true, false] {
        for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
            for confirmed in [false, true] {
                let fixture = Fixture::new(false);
                let mut sub = proposal(kind, confirmed, PropertyIdentifier::PRESENT_VALUE);
                let expiry = Instant::now() + Duration::from_millis(100);
                sub.expires_at = Some(expiry);
                let snapshots = vec![fixture.table.write().await.subscribe(sub).unwrap()];
                let db_guard = fixture.db.write().await;
                let mut work = Box::pin(fixture.fire(initial, &snapshots));
                assert!(futures_util::poll!(work.as_mut()).is_pending());
                wait_until_expired(expiry).await;
                drop(db_guard);
                tokio::time::timeout(Duration::from_secs(2), work)
                    .await
                    .unwrap();
                assert!(fixture.sent.lock().unwrap().is_empty());
                assert_eq!(fixture.transactions.active_count(), 0);
                assert_eq!(fixture.permits.available_permits(), 8);
                let table = fixture.table.read().await;
                assert_eq!(table.counters().snapshot().notifications_sent, 0);
                assert_eq!(table.counters().snapshot().notification_bytes_sent, 0);
                assert_eq!(
                    table
                        .get_subscription(snapshots[0].key())
                        .unwrap()
                        .last_notified_value,
                    Some(1.0)
                );
                drop(table);
                fixture.finish(false).await;
            }
        }
    }
}

#[tokio::test]
async fn cov_lifetime_context_refresh_uses_live_expiry_without_replacing_snapshot() {
    for initial in [true, false] {
        for confirmed in [false, true] {
            let fixture = Fixture::new(false);
            let mut sub = proposal(
                CovNotificationKind::Multiple,
                confirmed,
                PropertyIdentifier::PRESENT_VALUE,
            );
            sub.expires_at = Some(Instant::now() + Duration::from_secs(1));
            let snapshots = vec![fixture.table.write().await.subscribe(sub).unwrap()];
            let old_expiry = snapshots[0].expires_at;
            let db_guard = fixture.db.write().await;
            let mut work = Box::pin(fixture.fire(initial, &snapshots));
            assert!(futures_util::poll!(work.as_mut()).is_pending());
            {
                let mut table = fixture.table.write().await;
                table
                    .subscribe_multiple(
                        snapshots[0].key().multiple_context().unwrap(),
                        Instant::now() + Duration::from_secs(1000),
                        vec![],
                    )
                    .unwrap();
                assert!(
                    table.is_current(&snapshots[0]),
                    "context-only refresh retains its generation"
                );
                assert_ne!(
                    table
                        .get_subscription(snapshots[0].key())
                        .unwrap()
                        .expires_at,
                    old_expiry
                );
            }
            drop(db_guard);
            work.await;
            tokio::time::timeout(Duration::from_secs(2), async {
                while fixture.sent.lock().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!((999..=1000).contains(&remaining(
                fixture.sent.lock().unwrap()[0].clone(),
                CovNotificationKind::Multiple
            )));
            assert_eq!(snapshots[0].expires_at, old_expiry);
            fixture.finish(confirmed).await;
        }
    }
}

#[tokio::test]
async fn cov_lifetime_admitted_confirmed_retry_survives_expiry_and_ack_drains() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let mut fixture = Fixture::new(false);
        fixture.config.cov_retry_timeout_ms = 200;
        let mut sub = proposal(kind, true, PropertyIdentifier::PRESENT_VALUE);
        let expiry = Instant::now() + Duration::from_millis(100);
        sub.expires_at = Some(expiry);
        let snapshots = vec![fixture.table.write().await.subscribe(sub.clone()).unwrap()];
        fixture.fire(true, &snapshots).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.sent.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fixture.transactions.active_count(), 1);
        wait_until_expired(expiry).await;
        assert!(!fixture.table.read().await.is_current(&snapshots[0]));
        tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.sent.lock().unwrap().len() < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        {
            let frames = fixture.sent.lock().unwrap();
            assert_eq!(frames[0], frames[1], "retry retains the admitted APDU");
            assert_eq!(remaining(frames[0].clone(), kind), 1);
        }
        sub.expires_at = None;
        sub.last_notified_value = Some(99.0);
        let replacement = fixture.table.write().await.subscribe(sub).unwrap();
        fixture.finish(true).await;
        assert_eq!(fixture.transactions.active_count(), 0);
        assert_eq!(
            fixture
                .table
                .read()
                .await
                .get_subscription(replacement.key())
                .unwrap()
                .last_notified_value,
            Some(99.0)
        );
    }
}
