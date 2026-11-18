use super::*;
use bacnet_endpoint_core::endpoint_ingress::{
    EndpointApduDestination, EndpointEgressAdmissionError, EndpointIngress,
};
use bacnet_network::response_route::ResponseRoute;
use bacnet_npdu::NetworkPriority;
use bacnet_transport::{port::DirectResponseScope, sc::WebSocketPort};
use bacnet_types::MacAddr;
use std::time::Duration;

#[tokio::test]
async fn checked_response_queue_is_bounded_caller_owned_and_does_not_retarget() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let gate = f.hold_sends();
    let mut ingress = EndpointIngress::new(port, 1);
    let queues = ingress.start().await.unwrap();
    let mut a = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut a, &unsupported(1), None).await;
    let route = ResponseRoute::new(admitted.provenance, admitted.direct_response.clone());
    let first = queues
        .egress
        .admit_owned_apdu(
            vec![0x10, 8],
            EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&[1]),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
        .unwrap();
    bounded(f.generic.recv()).await.unwrap(); // The sole network worker is now held.
    let canceled = queues
        .egress
        .admit_response_apdu(
            vec![0x60, 1, 9],
            MacAddr::from_slice(&PEER_MAC),
            None,
            route.clone(),
        )
        .unwrap();
    assert!(matches!(
        queues.egress.admit_response_apdu(
            vec![0x60, 2, 9],
            MacAddr::from_slice(&PEER_MAC),
            None,
            route.clone()
        ),
        Err(EndpointEgressAdmissionError::QueueFull)
    ));
    let mut b = f.peer(ca.tls("B"), None).await;
    let fresh = f.capture(&mut b, &unsupported(3), None).await;
    drop(canceled);
    gate.add_permits(1);
    assert!(bounded(first.complete()).await.result.is_ok());
    bounded(queues.egress.wait_for_capacity()).await.unwrap();
    let current = queues
        .egress
        .admit_response_apdu(
            vec![0x60, 3, 9],
            MacAddr::from_slice(&PEER_MAC),
            None,
            ResponseRoute::new(fresh.provenance, fresh.direct_response),
        )
        .unwrap();
    let result = bounded(current.complete()).await;
    assert!(result.attempted && result.result.is_ok());
    assert!(matches!(f.response(&b).await.1, Apdu::Reject(r) if r.invoke_id == 3));
    ingress.stop().await.unwrap();
    assert!(matches!(
        queues.egress.admit_response_apdu(
            vec![0x60, 4, 9],
            MacAddr::from_slice(&PEER_MAC),
            None,
            route
        ),
        Err(EndpointEgressAdmissionError::Closed)
    ));
    f.listener.stop().await;
    assert_eq!(f.listener.active_connections(), 0);
    // Holding the original capability did not retain A's socket or membership.
    assert!(
        admitted
            .direct_response
            .unwrap()
            .send(&[1, 0, 0x60, 4, 9], &DirectResponseScope::default())
            .await
            .is_err()
    );
}

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
