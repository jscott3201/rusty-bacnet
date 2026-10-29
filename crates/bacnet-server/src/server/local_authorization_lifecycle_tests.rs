//! Caller cancellation and the reused post-commit task lifecycle.
use super::boundary::*;
use super::fixture::*;
use super::*;
use futures_util::FutureExt;
use std::future::Future;
use std::task::{Context, Poll, Waker};

#[tokio::test(start_paused = true)]
async fn local_authorization_cancel_after_commit_keeps_every_retained_payload() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        configured(db, AuthMode::VERIFICATION_REQUIRED, true, false)
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
    let mut req = request(false);
    req.now = Duration::from_secs(105);
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
    let (entered, release) = h.hold_notification_barrier(true);
    let mut route = Box::pin(h.server.authorize_access_local(req, 42));
    assert!(
        route
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    entered.acquire().await.unwrap().forget();
    assert_eq!(snapshot(&h).await[1], PropertyValue::Unsigned(42));
    drop(route);
    {
        let mut db = h.server.database().write().await;
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
    for (i, event) in [
        AccessEvent::PASSBACK_DETECTED,
        AccessEvent::ACCOMPANIMENT_BY,
        AccessEvent::VERIFICATION_REQUIRED,
        AccessEvent::GRANTED,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            values(&h.cov_notification().await, oid),
            expected_values(event, 42, i as u8, if i == 1 { 2 } else { 1 })
        );
    }
    assert_eq!(snapshot(&h).await[1], PropertyValue::Unsigned(99));
    // A later operation on the same object still proceeds after cancellation.
    assert_eq!(
        h.server
            .authorize_access_local(req, 43)
            .await
            .unwrap()
            .committed_events
            .len(),
        4
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_postcommit_panic_propagates_and_next_attempt_progresses() {
    let panic = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&panic);
    let mut h = Harness::start_with(ServerConfig::default(), move |db| {
        native_objects(db);
        let mut wrapper = Boundary::wrap(db, identifier(ObjectType::ACCESS_POINT));
        wrapper.panic_event = Some(flag);
        db.add(Box::new(wrapper)).unwrap();
    })
    .await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    subscribed(&mut h, oid).await;
    h.set_clock(20);
    panic.store(true, Ordering::SeqCst);
    let outcome = std::panic::AssertUnwindSafe(h.server.authorize_access_local(request(false), 42))
        .catch_unwind()
        .await;
    assert!(outcome.is_err());
    assert_eq!(snapshot(&h).await[1], PropertyValue::Unsigned(42));
    assert!(h.server.database().try_write().is_ok());
    assert!(h.server.cov_table.try_write().is_ok());
    assert_eq!(
        h.server
            .authorize_access_local(request(false), 43)
            .await
            .unwrap()
            .evaluation
            .outcome,
        AuthorizationOutcome::Granted
    );
    assert_eq!(
        values(&h.cov_notification().await, oid),
        expected_values(AccessEvent::GRANTED, 43, 1, 1)
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_closed_task_set_keeps_commit_and_releases_database() {
    let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    subscribed(&mut h, oid).await;
    h.set_clock(20);
    h.server.request_tasks.close();
    let result = h
        .server
        .authorize_access_local(request(false), 42)
        .await
        .unwrap();
    assert_eq!(result.committed_events.len(), 1);
    assert_eq!(snapshot(&h).await[1], PropertyValue::Unsigned(42));
    h.no_notification().await;
    assert!(h.server.database().try_write().is_ok());
    assert!(h.server.cov_table.try_write().is_ok());
    // The closed task set preserves commit semantics, not delivery; another
    // local call can still commit until stop marks the network inactive.
    assert_eq!(
        h.server
            .authorize_access_local(request(false), 43)
            .await
            .unwrap()
            .committed_events
            .len(),
        1
    );
    h.server.stop().await.unwrap();
    assert!(
        h.server
            .authorize_access_local(request(false), 44)
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn local_authorization_shutdown_aborts_blocked_delivery_without_undoing_commit() {
    let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
    let oid = identifier(ObjectType::ACCESS_POINT);
    subscribed(&mut h, oid).await;
    h.set_clock(20);
    let (entered, _release) = h.hold_notification_barrier(true);
    let mut route = Box::pin(h.server.authorize_access_local(request(false), 42));
    assert!(
        route
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    entered.acquire().await.unwrap().forget();
    drop(route);
    h.server.stop().await.unwrap();
    assert_eq!(snapshot(&h).await[1], PropertyValue::Unsigned(42));
    assert!(h.server.database().try_write().is_ok());
    assert!(h.server.cov_table.try_write().is_ok());
    assert!(
        h.server
            .authorize_access_local(request(false), 43)
            .await
            .is_err()
    );
}

#[test]
fn local_authorization_without_runtime_refuses_before_mutation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut h = runtime.block_on(Harness::start_with(ServerConfig::default(), native_objects));
    let before = runtime.block_on(snapshot(&h));
    let mut route = Box::pin(h.server.authorize_access_local(request(false), 42));
    assert!(matches!(
        route.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(Error::Encoding(_)))
    ));
    drop(route);
    assert_eq!(runtime.block_on(snapshot(&h)), before);
    runtime.block_on(h.server.stop()).unwrap();
}
