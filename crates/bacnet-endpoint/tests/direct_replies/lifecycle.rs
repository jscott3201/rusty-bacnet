use super::*;
use bacnet_endpoint_core::endpoint_ingress::{
    EndpointApduDestination, EndpointEgressAdmissionError, EndpointIngress,
};
use bacnet_network::response_route::ResponseRoute;
use bacnet_npdu::NetworkPriority;
use bacnet_transport::{port::DirectResponseScope, sc::WebSocketPort};
use bacnet_types::{MacAddr, enums::PropertyIdentifier};

#[tokio::test]
async fn checked_response_queue_is_bounded_caller_owned_and_does_not_retarget() {
    let ca = TestCa::new();
    let (mut f, mut port) = Fixture::new(&ca).await;
    let gate = port.hold_sends();
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

async fn session_pending_response(stop: bool) {
    let ca = TestCa::new();
    let (mut f, mut port) = Fixture::new(&ca).await;
    let _gate = port.hold_sends();
    let mut session = EndpointSession::new(
        port,
        SessionRole::Both,
        SessionConfig {
            queue_capacity: 1,
            ..Default::default()
        },
    )
    .unwrap()
    .with_database(database("owned"));
    session.start().await.unwrap();
    let handle = session.cloned_server_handle().unwrap();
    let client = session.cloned_client_handle().unwrap();
    let request = tokio::spawn(async move {
        client
            .read_property(&[1], csv_oid(), PropertyIdentifier::OBJECT_NAME, None)
            .await
    });
    bounded(f.generic.recv()).await.unwrap();
    let mut a = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut a, &unsupported(10), None).await;
    let saved = admitted.direct_response.clone().unwrap();
    let response = handle.handle_inbound(received_apdu(admitted));
    tokio::pin!(response);
    // Unsupported request has no async service work. This first poll synchronously
    // admits its response behind the held network operation, then waits completion.
    assert!(futures_util::poll!(&mut response).is_pending());
    if stop {
        bounded(session.stop()).await.unwrap();
    } else {
        drop(session);
    }
    assert!(bounded(response).await.is_err());
    assert!(bounded(request).await.unwrap().is_err());
    assert!(!handle.is_session_alive());
    let read = a.ws.recv();
    tokio::pin!(read);
    assert!(futures_util::poll!(&mut read).is_pending());
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
async fn live_session_stop_cancels_queued_original_response_and_retained_handle() {
    session_pending_response(true).await;
}
#[tokio::test]
async fn live_session_drop_cancels_queued_original_response_and_retained_handle() {
    session_pending_response(false).await;
}

#[tokio::test]
async fn canceling_live_responder_waiter_retracts_queued_reply_before_replacement() {
    let ca = TestCa::new();
    let (mut f, mut port) = Fixture::new(&ca).await;
    let gate = port.hold_sends();
    let mut session =
        EndpointSession::new(port, SessionRole::Both, SessionConfig::default()).unwrap();
    session.start().await.unwrap();
    let handle = session.cloned_server_handle().unwrap();
    let client = session.cloned_client_handle().unwrap();
    let request = tokio::spawn(async move {
        client
            .read_property(&[1], csv_oid(), PropertyIdentifier::OBJECT_NAME, None)
            .await
    });
    bounded(f.generic.recv()).await.unwrap();
    let mut a = f.peer(ca.tls("A"), None).await;
    let admitted = f.capture(&mut a, &unsupported(20), None).await;
    {
        let response = handle.handle_inbound(received_apdu(admitted));
        tokio::pin!(response);
        assert!(futures_util::poll!(&mut response).is_pending());
    }
    gate.add_permits(1);
    request.abort();
    assert!(bounded(request).await.unwrap_err().is_cancelled());
    // Keep A current until the barrier: retirement must not mask failed cancellation.
    f.barrier(&mut a, 21).await;
    let mut b = f.peer(ca.tls("B"), None).await;
    f.barrier(&mut b, 22).await;
    session.stop().await.unwrap();
    f.listener.stop().await;
}
