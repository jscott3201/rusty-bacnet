//! Deterministic lock/waiter barriers for the evaluation-to-capture boundary.
use super::boundary::*;
use super::fixture::*;
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Waker};

fn pending<F: Future>(future: Pin<&mut F>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

fn counted(db: &mut ObjectDatabase, reads: Arc<AtomicUsize>) {
    configured(db, AuthMode::DENY_ALL, false, false);
    let oid = identifier(ObjectType::ACCESS_POINT);
    db.get_mut(&oid)
        .unwrap()
        .write_property(
            P::AUTHORIZATION_MODE,
            None,
            PropertyValue::Enumerated(AuthMode::AUTHORIZE.to_raw()),
            None,
        )
        .unwrap();
    let mut wrapper = Boundary::wrap(db, oid);
    wrapper.mode_reads = reads;
    db.add(Box::new(wrapper)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_cov_collision_releases_db_and_retries_fresh_until_quiescent() {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        counted(db, Arc::clone(&reads))
    })
    .await;
    h.set_clock(20);
    let oid = identifier(ObjectType::ACCESS_POINT);
    let before = snapshot(&h).await;
    let mut table = Arc::clone(&h.server.cov_table).write_owned().await;
    let mut route = Box::pin(h.server.authorize_access_local(request(false), 42));
    for _ in 0..3 {
        pending(route.as_mut()); // Failed pair, now queued on COV readiness.
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(h.server.database().try_write().is_ok());
        assert_eq!(snapshot(&h).await, before);
        drop(table);
        pending(route.as_mut()); // Readiness acquired/released, cooperative yield.
        table = Arc::clone(&h.server.cov_table).try_write_owned().unwrap();
    }
    pending(route.as_mut());
    {
        let mut db = h.server.database().try_write().unwrap();
        db.get_mut(&oid)
            .unwrap()
            .write_property(
                P::AUTHORIZATION_MODE,
                None,
                PropertyValue::Enumerated(AuthMode::DENY_ALL.to_raw()),
                None,
            )
            .unwrap();
    }
    // Admit a subscriber after failed attempts. No cached subscription set from
    // one of those attempts may replace the table used at the successful pair.
    table
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&PEER),
            subscriber_network: None,
            subscriber_process_identifier: 1569,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
    let context = crate::cov::MultipleContextKey {
        recipient: crate::cov::CovRecipient::from_endpoint(&MacAddr::from_slice(&PEER), None),
        process_id: 1570,
        confirmed: false,
    };
    let expires_at = std::time::Instant::now() + Duration::from_secs(300);
    let fresh = table
        .subscribe_multiple(
            &context,
            &crate::cov::SubscriberEndpoint::new(&PEER, None),
            expires_at,
            0,
            None,
            vec![CovSubscription {
                subscriber_mac: MacAddr::from_slice(&PEER),
                subscriber_network: None,
                subscriber_process_identifier: 1570,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: false,
                expires_at: Some(expires_at),
                last_notified_observation: None,
                monitored_property: Some(P::ACCESS_EVENT_TIME),
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Multiple,
                timestamped: true,
            }],
        )
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        table
            .timed()
            .lock()
            .baseline(fresh.key(), fresh.generation())
            .is_none()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    drop(table);
    pending(route.as_mut()); // Complete readiness, then guard-free yield.
    pending(route.as_mut()); // Commit/capture; owned post-commit task not yet polled.
    assert!(
        h.server
            .cov_table
            .try_read()
            .unwrap()
            .timed()
            .lock()
            .baseline(fresh.key(), fresh.generation())
            .is_some()
    );
    let result = route.await.unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(matches!(
        result.evaluation.outcome,
        AuthorizationOutcome::Denied {
            access_event: AccessEvent::DENIED_DENY_ALL,
            ..
        }
    ));
    assert_eq!(
        values(&h.cov_notification().await, oid),
        expected_values(AccessEvent::DENIED_DENY_ALL, 42, 0, 1)
    );
    let timed = h.notification().await;
    assert!(
        timed
            .list_of_cov_notifications
            .iter()
            .flat_map(|item| &item.list_of_values)
            .any(|value| value.property_identifier == P::ACCESS_EVENT_TIME
                && value.value == stamp_bytes(20)
                && value.time_of_change == Some(time(20)))
    );
    assert!(h.server.database().try_write().is_ok());
    assert!(h.server.cov_table.try_write().is_ok());
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_cancellation_waiting_for_either_guard_never_evaluates_or_mutates() {
    for cov in [false, true] {
        let reads = Arc::new(AtomicUsize::new(0));
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            counted(db, Arc::clone(&reads))
        })
        .await;
        let oid = identifier(ObjectType::ACCESS_POINT);
        subscribed(&mut h, oid).await;
        h.set_clock(20);
        let before = snapshot(&h).await;
        let db_guard = if cov {
            None
        } else {
            Some(Arc::clone(&h.server.db).write_owned().await)
        };
        let cov_guard = if cov {
            Some(Arc::clone(&h.server.cov_table).write_owned().await)
        } else {
            None
        };
        let mut route = Box::pin(h.server.authorize_access_local(request(false), 42));
        pending(route.as_mut());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        if cov {
            assert!(h.server.database().try_write().is_ok());
        }
        drop(route);
        drop(db_guard);
        drop(cov_guard);
        assert!(h.server.database().try_write().is_ok());
        assert!(h.server.cov_table.try_write().is_ok());
        assert_eq!(snapshot(&h).await, before);
        h.no_notification().await;
        assert_eq!(
            h.server
                .authorize_access_local(request(false), 42)
                .await
                .unwrap()
                .evaluation
                .outcome,
            AuthorizationOutcome::Granted
        );
        assert_eq!(
            values(&h.cov_notification().await, oid),
            expected_values(AccessEvent::GRANTED, 42, 0, 1)
        );
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn local_authorization_competing_mode_waiter_cannot_enter_between_evaluation_commit_capture()
{
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let entered = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Arc::new(std::sync::Mutex::new(release_rx));
    let armed = Arc::new(AtomicBool::new(true));
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        counted(db, Arc::new(AtomicUsize::new(0)));
        let mut wrapper = Boundary::wrap(db, identifier(ObjectType::ACCESS_POINT));
        wrapper.on_mode = Some(Arc::new(move || {
            if armed.swap(false, Ordering::SeqCst) {
                entered.lock().unwrap().take().unwrap().send(()).unwrap();
                release.lock().unwrap().recv().unwrap();
            }
        }));
        db.add(Box::new(wrapper)).unwrap();
    })
    .await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    subscribed(&mut h, oid).await;
    h.subscribe_with_delay(false, vec![(oid, vec![(P::ACCESS_EVENT_TIME, true)])], 0)
        .await;
    assert_eq!(response(&h).await, Ok(()));
    h.notification().await;
    h.set_clock(20);
    let server = Arc::new(h.server);
    let owner = Arc::clone(&server);
    let runtime = tokio::runtime::Handle::current();
    let operation = tokio::task::spawn_blocking(move || {
        runtime.block_on(owner.authorize_access_local(request(false), 42))
    });
    entered_rx.await.unwrap();
    // The reader is synchronously paused inside evaluation. Queue the competing
    // writer now; it must stay pending while both evaluation guards are held.
    let mut writer = Box::pin(Arc::clone(&server.db).write_owned());
    pending(writer.as_mut());
    assert!(server.cov_table.try_write().is_err());
    release_tx.send(()).unwrap();
    let mut db = writer.await;
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(P::ACCESS_EVENT_TAG, None)
            .unwrap(),
        PropertyValue::Unsigned(42)
    );
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(P::ACCESS_EVENT, None)
            .unwrap(),
        PropertyValue::Enumerated(AccessEvent::GRANTED.to_raw())
    );
    db.get_mut(&oid)
        .unwrap()
        .write_property(
            P::AUTHORIZATION_MODE,
            None,
            PropertyValue::Enumerated(AuthMode::DENY_ALL.to_raw()),
            None,
        )
        .unwrap();
    // Changing the live event here also distinguishes prior timed capture from
    // a delayed live read after the competing writer obtains the database.
    db.get_mut(&oid)
        .unwrap()
        .report_access_input_internal(AccessControlInput::AccessEvent(access_event(
            AccessEvent::DENIED_OTHER,
            99,
            30,
        )))
        .unwrap();
    drop(db);
    let result = operation.await.unwrap().unwrap();
    assert_eq!(result.evaluation.outcome, AuthorizationOutcome::Granted);
    h.server = Arc::try_unwrap(server).ok().unwrap();
    assert_eq!(
        values(&h.cov_notification().await, oid),
        expected_values(AccessEvent::GRANTED, 42, 0, 1)
    );
    let timed = h.notification().await;
    assert!(
        timed
            .list_of_cov_notifications
            .iter()
            .flat_map(|i| &i.list_of_values)
            .any(|v| v.property_identifier == P::ACCESS_EVENT_TIME
                && v.value == stamp_bytes(20)
                && v.time_of_change == Some(time(20)))
    );
    // Admitted first, the same competing mode change must affect the next pass.
    let next = h
        .server
        .authorize_access_local(request(false), 43)
        .await
        .unwrap();
    assert!(matches!(
        next.evaluation.outcome,
        AuthorizationOutcome::Denied {
            access_event: AccessEvent::DENIED_DENY_ALL,
            ..
        }
    ));
    h.server.stop().await.unwrap();
}
