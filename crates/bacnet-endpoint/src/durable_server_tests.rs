//! A session's full server (#2617) and its objects' saves: the session's
//! stop stops the server first, whose own stop waits for the saves while
//! the link is still up, and a session dropped with its server, stopped or
//! not, lets the database go off the runtime as a client-only one does.
use super::*;
use bacnet_server::server::BACnetServer;

/// A started session of both roles on the capture link, with Device 123,
/// `class`, and a full server it built.
async fn started_with_server(class: NotificationClass) -> Endpoint {
    let mut endpoint = Endpoint::new(SessionRole::Both);
    let identity = crate::DeviceIdentity::new(123, 42).unwrap();
    let db = identity.build_database().unwrap();
    endpoint.session = endpoint
        .session
        .with_database(db)
        .device(identity)
        .with_server(BACnetServer::generic_builder());
    add_class(&mut endpoint, class);
    endpoint.start(false).await;
    assert!(endpoint.session.server().is_some());
    endpoint
}

#[tokio::test(flavor = "current_thread")]
async fn stop_stops_the_server_first_while_the_link_is_up() {
    let storage = Storage::holding();
    let (class, _dropped_on) = reporting_class(&storage);
    let mut endpoint = started_with_server(class).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    let (progress, watchdog) = watchdog(go);
    // The server's stop waits for the held save: the session's stop is
    // still in it, and the link not stopped yet.
    assert!(
        timeout(Duration::from_millis(20), endpoint.session.stop())
            .await
            .is_err(),
        "stop returned while a save was held"
    );
    assert!(!endpoint.session.ingress_stopped, "the link stopped first");
    assert!(
        endpoint.session.server.is_some(),
        "kept through a cancelled stop"
    );
    progress.send(()).unwrap();
    assert!(
        !tokio::task::spawn_blocking(move || watchdog.join().unwrap())
            .await
            .unwrap(),
        "the stop held the runtime's thread"
    );
    bounded(endpoint.session.stop()).await.unwrap();
    assert!(endpoint.session.server().is_none());
    assert!(endpoint.session.ingress_stopped);
    // The abandoned staged write was put back before the link went.
    assert_eq!(
        *storage.saved.lock().unwrap(),
        Some(snapshot(&[destination(1)]))
    );
}

#[tokio::test]
async fn a_session_dropped_with_its_server_lets_the_database_go_off_the_runtime() {
    for stop_first in [true, false] {
        let storage = Storage::holding();
        let (class, dropped_on) = reporting_class(&storage);
        let mut endpoint = started_with_server(class).await;
        if stop_first {
            bounded(endpoint.session.stop()).await.unwrap();
        }
        let db = database(&endpoint);
        let go = stage_held_save(&db, &storage).await;
        let weak = Arc::downgrade(&db);
        drop(db);
        // Unstopped, the server's aborted tasks, the server and the session
        // each hold the database; whichever goes last lets it go off the
        // runtime.
        let (progress, watchdog) = watchdog(go);
        drop(endpoint);
        assert!(
            thread_was_free(weak, progress, watchdog).await,
            "dropping the session held the runtime's thread (stopped first: {stop_first})"
        );
        dropped_off_the_runtime(dropped_on, &storage).await;
    }
}
