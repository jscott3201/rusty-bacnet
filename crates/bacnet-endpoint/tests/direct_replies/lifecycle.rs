use super::*;
use bacnet_transport::{port::DirectResponseScope, sc::WebSocketPort};
use std::time::Duration;

/// The session's server answers a peer's request on its direct connection.
/// That answer can't pass ordinary sends held behind the client's (it goes
/// by its saved route), so here it waits on the database the test holds. A
/// stop or a drop of the session cancels it (#2617): the server's request
/// work ends with it, the peer never gets the answer, and the saved route
/// dies with the listener.
async fn session_pending_response(stop: bool) {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(node())
        .with_database(database("owned"));
    session.start().await.unwrap();
    let mut a = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut a, &read_name(10), None).await;
    let saved = admitted.direct_response.clone().unwrap();
    let database = std::sync::Arc::clone(session.server().unwrap().database());
    let guard = database.write().await;
    f.feed(admitted).await;
    let server = session.server().unwrap();
    bounded(async {
        while server.request_admission_counters().confirmed_active == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let read = a.ws.recv();
    tokio::pin!(read);
    if stop {
        let stopping = session.stop();
        tokio::pin!(stopping);
        let _ = futures_util::poll!(&mut stopping);
        drop(guard);
        bounded(stopping).await.unwrap();
    } else {
        drop(session);
        drop(guard);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        futures_util::poll!(&mut read).is_pending(),
        "a cancelled answer reached the peer"
    );
    assert!(f.generic.try_recv().is_err());
    f.listener.stop().await;
    assert_eq!(f.listener.active_connections(), 0);
    assert!(
        saved
            .send(&[1, 0, 0x60, 10, 9], &DirectResponseScope::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn live_session_stop_cancels_the_servers_queued_direct_answer() {
    session_pending_response(true).await;
}
#[tokio::test]
async fn live_session_drop_cancels_the_servers_queued_direct_answer() {
    session_pending_response(false).await;
}
