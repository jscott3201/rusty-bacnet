//! A graceful endpoint stop waits for the objects' saves while the runtime
//! progresses: the server's stop, first, settles them (#2617).
use super::*;
use bacnet_encoding::constructed::decode_destination_list;

async fn served_snapshot(db: &RwLock<ObjectDatabase>) -> NotificationClassSnapshot {
    let guard = db.read().await;
    let value = guard
        .get(&oid(ObjectType::NOTIFICATION_CLASS, 1))
        .unwrap()
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let PropertyValue::ApplicationData(encoded) = value else {
        panic!("Recipient_List must contain encoded destinations")
    };
    snapshot(&decode_destination_list(&encoded).unwrap())
}

#[tokio::test(flavor = "current_thread")]
async fn graceful_stop_waits_for_save_and_correction_without_blocking_runtime() {
    for role in [SessionRole::ServerOnly, SessionRole::Both] {
        let storage = Storage::holding();
        let (class, _dropped_on) = reporting_class(&storage);
        let mut endpoint = started_with(role, class).await;
        let db = database(&endpoint);
        let go = stage_held_save(&db, &storage).await;
        let expected = served_snapshot(&db).await;
        let (progress, watchdog) = watchdog(go);
        let stop = endpoint.session.stop();
        tokio::pin!(stop);
        let pending = timeout(Duration::from_millis(20), &mut stop).await.is_err();
        tokio::spawn(async move {
            let _ = progress.send(());
        })
        .await
        .unwrap();
        let stalled = tokio::task::spawn_blocking(move || watchdog.join().unwrap())
            .await
            .unwrap();
        assert!(!stalled, "{role:?}: stop blocked the runtime worker");
        assert!(
            pending,
            "{role:?}: stop returned while a durable save was held"
        );
        bounded(stop).await.unwrap();
        assert_eq!(*storage.saved.lock().unwrap(), Some(expected), "{role:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn graceful_stop_finishes_after_failed_correction_without_claiming_storage_success() {
    let storage = Storage::holding();
    storage.fail_on.store(2, Ordering::SeqCst);
    let (class, _dropped_on) = reporting_class(&storage);
    let mut endpoint = started_with(SessionRole::Both, class).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    let expected = served_snapshot(&db).await;
    let (progress, watchdog) = watchdog(go);
    assert!(
        timeout(Duration::from_millis(20), endpoint.session.stop())
            .await
            .is_err()
    );
    progress.send(()).unwrap();
    assert!(
        !tokio::task::spawn_blocking(move || watchdog.join().unwrap())
            .await
            .unwrap()
    );
    assert!(matches!(
        bounded(endpoint.session.stop()).await,
        Ok(SessionExit::Cancelled)
    ));
    assert_eq!(storage.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(served_snapshot(&db).await, expected);
    assert_eq!(
        *storage.saved.lock().unwrap(),
        Some(snapshot(&[destination(11)])),
        "failed correction leaves storage different from the served state"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn retained_dispatch_and_ingress_errors_still_wait_for_settlement() {
    for ingress_error in [false, true] {
        let storage = Storage::holding();
        let (class, _dropped_on) = reporting_class(&storage);
        let mut endpoint = Endpoint::with_stop_failure(SessionRole::Both, ingress_error);
        endpoint.session = endpoint.session.with_database(
            crate::DeviceIdentity::new(123, 42)
                .unwrap()
                .build_database()
                .unwrap(),
        );
        add_class(&mut endpoint, class);
        endpoint.start(false).await;
        if !ingress_error {
            endpoint.session.dispatch_task.as_ref().unwrap().abort();
        }
        let expected_error = if ingress_error {
            "injected capture stop failure"
        } else {
            "dispatch failed"
        };
        let db = database(&endpoint);
        let go = stage_held_save(&db, &storage).await;
        let expected = served_snapshot(&db).await;
        let (progress, watchdog) = watchdog(go);
        // The server's stop, first, settles the held save.
        assert!(
            timeout(Duration::from_millis(20), endpoint.session.stop())
                .await
                .is_err()
        );
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopping as u8
        );
        // A stop under way says so (#1833).
        assert!(matches!(
            endpoint.session.client(),
            Err(Error::State(bacnet_types::error::Lifecycle::Stopping, _))
        ));
        tokio::spawn(async move {
            let _ = progress.send(());
        })
        .await
        .unwrap();
        let stalled = tokio::task::spawn_blocking(move || watchdog.join().unwrap())
            .await
            .unwrap();
        assert!(!stalled);
        let error = bounded(endpoint.session.stop()).await.unwrap_err();
        assert!(error.to_string().contains(expected_error), "{error}");
        // An aborted dispatch task leaves the session shut for good.
        assert!(
            ingress_error
                || matches!(
                    error,
                    Error::State(bacnet_types::error::Lifecycle::Closed, _)
                ),
            "{error:?}"
        );
        assert_eq!(*storage.saved.lock().unwrap(), Some(expected));
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopped as u8
        );
        assert!(endpoint.session.stop_exit.is_none());
    }
}

/// A server-role session with no Audit Reporter whose database the
/// application holds at stop: its server hands the settling to a task, so
/// the stop returns with the guard still held and the save storage holds
/// unfinished, and once the guard goes, storage comes back to the state the
/// class serves (#2617).
#[tokio::test(flavor = "current_thread")]
async fn a_guard_held_across_stop_defers_the_settling_without_holding_the_stop() {
    let storage = Storage::holding();
    let (class, _dropped_on) = reporting_class(&storage);
    let mut endpoint = started_with(SessionRole::Both, class).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    let expected = served_snapshot(&db).await;
    let guard = Arc::clone(&db).read_owned().await;
    bounded(endpoint.session.stop()).await.unwrap();
    assert_eq!(
        endpoint.session.lifecycle.load(Ordering::Acquire),
        Lifecycle::Stopped as u8
    );
    drop(guard);
    let (progress, watchdog) = watchdog(go);
    tokio::spawn(async move {
        let _ = progress.send(());
    })
    .await
    .unwrap();
    let stalled = tokio::task::spawn_blocking(move || watchdog.join().unwrap())
        .await
        .unwrap();
    assert!(!stalled, "the deferred settling blocked the runtime");
    // The held save lands the staged list, then the correction the
    // deferred settling queued puts back the one the class serves.
    crate::session::eventually::eventually(WAIT, "storage holds the served list", async || {
        storage.attempts.load(Ordering::SeqCst) == 2
            && *storage.saved.lock().unwrap() == Some(expected.clone())
    })
    .await;
}
