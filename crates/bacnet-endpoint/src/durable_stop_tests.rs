//! Graceful endpoint stop waits for object saves while the runtime progresses.
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
    for role in [
        SessionRole::ClientOnly,
        SessionRole::ServerOnly,
        SessionRole::Both,
    ] {
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
    assert!(timeout(Duration::from_millis(20), endpoint.session.stop())
        .await
        .is_err());
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
async fn canceled_stop_waits_for_database_and_rejoins_one_settlement_task() {
    for source in [false, true] {
        let storage = Storage::holding();
        let (class, _dropped_on) = reporting_class(&storage);
        let mut endpoint = if source {
            let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
            let mut endpoint = unstarted_source(SessionRole::Both, recipient, None);
            add_class(&mut endpoint, class);
            endpoint.start(false).await;
            endpoint
        } else {
            started_with(SessionRole::Both, class).await
        };
        let db = database(&endpoint);
        let go = stage_held_save(&db, &storage).await;
        let expected = served_snapshot(&db).await;
        let (progress, watchdog) = watchdog(go);
        let (locked, when_locked) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let holder_db = db.clone();
        let holder = tokio::spawn(async move {
            let _guard = holder_db.read().await;
            locked.send(()).unwrap();
            let _ = released.await;
        });
        when_locked.await.unwrap();
        assert!(timeout(Duration::from_millis(20), endpoint.session.stop())
            .await
            .is_err());
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopping as u8
        );
        assert!(endpoint.session.stop_exit.is_some());
        assert_eq!(endpoint.session.source_recipient.is_some(), source);
        assert!(!endpoint.session.is_running());
        release.send(()).unwrap();
        holder.await.unwrap();

        assert!(timeout(Duration::from_millis(20), endpoint.session.stop())
            .await
            .is_err());
        let owner = endpoint.session.durable_settlement.as_ref().unwrap().id();
        assert!(endpoint.session.source_recipient.is_none());
        assert!(
            db.try_write().is_ok(),
            "save wait must release the database guard"
        );
        assert!(Arc::ptr_eq(
            &db,
            endpoint.session.database.as_ref().unwrap()
        ));
        for _ in 0..3 {
            assert!(timeout(Duration::from_millis(10), endpoint.session.stop())
                .await
                .is_err());
            assert_eq!(
                endpoint.session.durable_settlement.as_ref().unwrap().id(),
                owner
            );
            assert!(matches!(
                endpoint.session.stop_exit,
                Some(Ok(SessionExit::Cancelled))
            ));
        }
        tokio::spawn(async move {
            let _ = progress.send(());
        })
        .await
        .unwrap();
        let stalled = tokio::task::spawn_blocking(move || watchdog.join().unwrap())
            .await
            .unwrap();
        assert!(!stalled, "database/save wait blocked the runtime");
        assert!(matches!(
            bounded(endpoint.session.stop()).await,
            Ok(SessionExit::Cancelled)
        ));
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopped as u8
        );
        assert!(endpoint.session.durable_settlement.is_none());
        assert!(endpoint.session.stop_exit.is_none());
        assert_eq!(*storage.saved.lock().unwrap(), Some(expected));
        assert!(endpoint.session.stop().await.is_err());
    }
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
        assert!(timeout(Duration::from_millis(20), endpoint.session.stop())
            .await
            .is_err());
        let saved = endpoint
            .session
            .stop_exit
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .to_string();
        assert!(saved.contains(expected_error), "{saved}");
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopping as u8
        );
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
        assert_eq!(error.to_string(), saved);
        assert_eq!(*storage.saved.lock().unwrap(), Some(expected));
        assert_eq!(
            endpoint.session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Stopped as u8
        );
        assert!(endpoint.session.stop_exit.is_none());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_a_canceled_stop_settlement_keeps_database_drop_off_runtime() {
    let storage = Storage::holding();
    let (class, dropped_on) = reporting_class(&storage);
    let mut endpoint = started_with(SessionRole::ClientOnly, class).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    let weak = Arc::downgrade(&db);
    drop(db);
    let (progress, watchdog) = watchdog(go);
    assert!(timeout(Duration::from_millis(20), endpoint.session.stop())
        .await
        .is_err());
    assert!(endpoint.session.durable_settlement.is_some());
    drop(endpoint);
    assert!(thread_was_free(weak, progress, watchdog).await);
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test(flavor = "current_thread")]
async fn unpolled_settlement_owner_drops_database_off_runtime() {
    let storage = Storage::holding();
    let (class, dropped_on) = reporting_class(&storage);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    let db = Arc::new(RwLock::new(db));
    let go = stage_held_save(&db, &storage).await;
    let weak = Arc::downgrade(&db);
    let settlement = settle_durable_writes(db);
    let (progress, watchdog) = watchdog(go);
    drop(settlement);
    assert!(thread_was_free(weak, progress, watchdog).await);
    dropped_off_the_runtime(dropped_on, &storage).await;
}
