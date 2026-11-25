//! A server seat beside a client seat on endpoint ingress (#2617): each
//! rule of who receives what, that a server which stops reading holds up
//! neither the client nor the link, and how the seats end.
use super::*;
use crate::coordinator::{
    CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator, TerminalPolicy,
};
use crate::seat::{ClientSeat, SeatRoles, ServerSeat};
use bacnet_types::enums::ConfirmedServiceChoice;

const PEER: [u8; 1] = [0x11];

const WHO_IS: &[u8] = &[0x10, 0x08];
const READ_PROPERTY: &[u8] = &[0x00, 0x05, 0x21, 0x0c];
const COV_NOTIFICATION: &[u8] = &[0x00, 0x05, 0x22, 0x01];
const COV_NOTIFICATION_MULTIPLE: &[u8] = &[0x00, 0x05, 0x25, 0x1f];
/// A first segment of a ConfirmedCOVNotificationMultiple: the service choice
/// sits after the sequence number and window size.
const SEGMENTED_COV_NOTIFICATION_MULTIPLE: &[u8] = &[0x0c, 0x05, 0x26, 0x00, 0x01, 0x1f, 0xaa];
const EVENT_NOTIFICATION: &[u8] = &[0x00, 0x05, 0x23, 0x02];
/// A first segment of a WriteProperty: segments of a request are the
/// server's to reassemble, or to abort.
const SEGMENTED_WRITE: &[u8] = &[0x0c, 0x05, 0x24, 0x00, 0x01, 0x0f, 0xaa];

struct Node {
    endpoint: EndpointIngress<TestTransport>,
    ingress: IngressReceivers,
    link: Link,
    coordinator: Arc<OutboundTransactionCoordinator>,
    client: ClientSeat<TestTransport>,
    server: ServerSeat<TestTransport>,
}

/// A started endpoint whose queues hold `capacity`, with a client seat
/// and a server seat, which takes the request queue's place and depth.
async fn node(capacity: usize) -> Node {
    let (transport, link) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, capacity);
    let ingress = endpoint.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let client = endpoint
        .client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8)
        .unwrap();
    let server = endpoint.server_seat(Arc::clone(&coordinator)).unwrap();
    Node {
        endpoint,
        ingress,
        link,
        coordinator,
        client,
        server,
    }
}

async fn inject_expecting_reply(link: &Link, apdu: &[u8]) -> oneshot::Receiver<Bytes> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut received = received_npdu(apdu);
    received.reply_tx = Some(reply_tx);
    link.sender.send(received).await.unwrap();
    reply_rx
}

/// A marker the policy queue takes: once it is there, everything injected
/// before it has been classified.
async fn settle(node: &mut Node) {
    inject(&node.link, &[0x80]).await;
    receive(&mut node.ingress.policy_outcomes).await;
}

/// Nothing is queued for either seat or for any of the endpoint's queues.
fn assert_quiet(node: &mut Node) {
    assert!(node.client.apdus.try_recv().is_err(), "client");
    assert!(node.server.apdus.try_recv().is_err(), "server");
    assert!(node.ingress.inbound_requests.try_recv().is_err(), "inbound");
    assert!(
        node.ingress.terminal_or_segment.try_recv().is_err(),
        "answers"
    );
    assert!(node.ingress.policy_outcomes.try_recv().is_err(), "policy");
}

#[tokio::test]
async fn every_request_is_the_servers_but_the_clients_cov_notifications() {
    let mut node = node(8).await;

    // An Unconfirmed-Request: the server's, and a copy for the client. The
    // endpoint's own request queue, whose place the server took, gets none.
    let _reply = inject_expecting_reply(&node.link, WHO_IS).await;
    let served = receive(&mut node.server.apdus).await;
    let copy = receive(&mut node.client.apdus).await;
    assert_eq!((served.apdu.as_ref(), copy.apdu.as_ref()), (WHO_IS, WHO_IS));
    assert!(served.reply_tx.is_some());
    assert!(copy.reply_tx.is_none() && copy.direct_response.is_none());
    assert_quiet(&mut node);

    // A Confirmed-Request, segmented or not: the server's alone, with its
    // reply sender.
    for apdu in [READ_PROPERTY, SEGMENTED_WRITE] {
        let _reply = inject_expecting_reply(&node.link, apdu).await;
        let served = receive(&mut node.server.apdus).await;
        assert_eq!(served.apdu.as_ref(), apdu);
        assert!(served.reply_tx.is_some());
        assert_quiet(&mut node);
    }

    // A COV notification of either form, a segmented one too (the client
    // aborts it), is the client's alone (#2654).
    for apdu in [
        COV_NOTIFICATION,
        COV_NOTIFICATION_MULTIPLE,
        SEGMENTED_COV_NOTIFICATION_MULTIPLE,
    ] {
        let _reply = inject_expecting_reply(&node.link, apdu).await;
        let notified = receive(&mut node.client.apdus).await;
        assert_eq!(notified.apdu.as_ref(), apdu);
        assert!(notified.reply_tx.is_some());
        assert_quiet(&mut node);
    }

    // A ConfirmedEventNotification is the server's to answer; the client
    // hears a copy.
    let _reply = inject_expecting_reply(&node.link, EVENT_NOTIFICATION).await;
    let served = receive(&mut node.server.apdus).await;
    let copy = receive(&mut node.client.apdus).await;
    assert!(served.reply_tx.is_some() && copy.reply_tx.is_none());
    assert_quiet(&mut node);

    let (client, server) = (
        node.endpoint.client_seat_counters(),
        node.endpoint.server_seat_counters(),
    );
    assert_eq!((client.apdus, client.full_drops), (5, 0));
    assert_eq!((server.apdus, server.full_drops), (4, 0));
}

#[tokio::test]
async fn an_answer_goes_to_the_role_whose_lease_holds_it() {
    let mut node = node(8).await;
    let peer = CanonicalPeer::direct(&PEER);
    let client = node
        .coordinator
        .reserve(LeaseMetadata::requester(
            peer.clone(),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::EitherAck,
        ))
        .unwrap();
    let server = node
        .coordinator
        .reserve(LeaseMetadata::notification(
            peer,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        ))
        .unwrap();

    inject(&node.link, &[0x20, client.invoke_id(), 0x0c]).await;
    receive(&mut node.client.apdus).await;
    assert_quiet(&mut node);
    // Every answer form, to the server's own request.
    let id = server.invoke_id();
    for apdu in [
        &[0x20, id, 0x01][..],
        &[0x50, id, 0x01, 0x91, 0x00, 0x91, 0x00],
        &[0x60, id, 0x00],
        &[0x71, id, 0x00],
    ] {
        inject(&node.link, apdu).await;
        assert_eq!(receive(&mut node.server.apdus).await.apdu.as_ref(), apdu);
        assert_quiet(&mut node);
    }
    // A requester's Abort or Segment-ACK is about a request the server
    // serves, whatever lease its invoke ID matches. The client sees a copy
    // of the Abort, which may end a notification it reassembles (#2646).
    for apdu in [
        &[0x70, client.invoke_id(), 0x00][..],
        &[0x40, 9, 0x00, 0x01],
    ] {
        inject(&node.link, apdu).await;
        assert_eq!(receive(&mut node.server.apdus).await.apdu.as_ref(), apdu);
        if apdu[0] == 0x70 {
            let copy = receive(&mut node.client.apdus).await;
            assert_eq!(copy.apdu.as_ref(), apdu);
            assert!(copy.direct_response.is_none() && copy.router_response.is_none());
        }
        assert_quiet(&mut node);
    }
    // An answer no lease holds stays with the endpoint, which counts it.
    inject(&node.link, &[0x20, 200, 0x0c]).await;
    receive(&mut node.ingress.terminal_or_segment).await;
    assert_quiet(&mut node);
}

/// The server never reads: its seat fills, and is skipped. Neither the
/// client's answers and notifications nor the link wait for it, and what
/// was the server's is never handed to another role in its place.
#[tokio::test]
async fn a_server_that_stops_reading_holds_up_neither_the_client_nor_the_link() {
    let mut node = node(4).await;
    let lease = node
        .coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&PEER),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::EitherAck,
        ))
        .unwrap();
    let answer = [0x20, lease.invoke_id(), 0x0c];
    let mut replies = Vec::new();
    for _ in 0..6 {
        inject(&node.link, WHO_IS).await;
        replies.push(inject_expecting_reply(&node.link, READ_PROPERTY).await);
        inject(&node.link, &answer).await;
        inject(&node.link, COV_NOTIFICATION).await;
        // The client keeps up, and gets each round's copy, answer and
        // notification at once, the server's full seat or not.
        for expected in [WHO_IS, &answer[..], COV_NOTIFICATION] {
            assert_eq!(
                receive(&mut node.client.apdus).await.apdu.as_ref(),
                expected
            );
        }
    }
    settle(&mut node).await;

    // The server's seat of 4 took a Who-Is while it had more than half
    // free, and two reads while it had more than its last quarter; the rest
    // was dropped and counted, and each dropped read's reply sender let go
    // of, so MS/TP postpones at once.
    let server = node.endpoint.server_seat_counters();
    assert_eq!((server.apdus, server.full_drops), (3, 9));
    let released = replies
        .iter_mut()
        .map(|reply| reply.try_recv())
        .filter(|reply| *reply == Err(oneshot::error::TryRecvError::Closed))
        .count();
    assert_eq!(released, 4);
    assert_eq!(node.endpoint.client_seat_counters().full_drops, 0);
    // None of it reached the endpoint's own request queue.
    assert!(node.ingress.inbound_requests.try_recv().is_err());
    assert_eq!(
        node.endpoint.full_drop_counters(),
        IngressDropCounters::default()
    );
}

#[tokio::test]
async fn a_gone_server_gives_the_endpoint_its_requests_back() {
    let Node {
        endpoint,
        mut ingress,
        link,
        coordinator,
        client: _client,
        server,
    } = node(8).await;
    // One server at a time, on the node's one pool of invoke IDs.
    let second = endpoint.server_seat(Arc::clone(&coordinator));
    assert!(matches!(
        second.unwrap_err(),
        Error::LimitReached(text)
            if text.to_string() == "the endpoint already has a full server: drop it before making another"
    ));

    // A server that is dropped frees its seat at once, before its tasks
    // have let go of the queue.
    let alive: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
    server.holds.alive_while(Arc::downgrade(&alive));
    drop(alive);
    let other_pool = endpoint.server_seat(Arc::new(OutboundTransactionCoordinator::new()));
    assert!(matches!(
        other_pool.unwrap_err(),
        Error::InvalidConfig(text) if text.to_string().contains("pool of invoke IDs")
    ));
    let again = endpoint.server_seat(Arc::clone(&coordinator)).unwrap();

    // With its queue gone too, a request goes where it went with no seat.
    drop((server, again));
    inject(&link, READ_PROPERTY).await;
    assert_eq!(
        receive(&mut ingress.inbound_requests).await.apdu.as_ref(),
        READ_PROPERTY
    );
}

#[tokio::test]
async fn one_stop_ends_both_seats_and_never_waits_for_the_server() {
    let Node {
        mut endpoint,
        link,
        mut server,
        client,
        ..
    } = node(8).await;
    // The server keeps the layer and its queue and names no task: the stop
    // closes its queue and stops the link, waiting for nothing of it.
    let exit = timeout(WAIT, endpoint.stop())
        .await
        .expect("the stop doesn't wait on the server");
    assert!(matches!(exit.unwrap(), ClassifierExit::Cancelled));
    assert_eq!(link.stops(), 1);
    assert!(server.apdus.recv().await.is_none());
    assert!(server.holds.hold().is_none());
    assert!(matches!(
        endpoint.server_seat(Arc::new(OutboundTransactionCoordinator::new())),
        Err(Error::State(error::Lifecycle::Closed, _))
    ));
    drop((server, client));
    assert_eq!(link.stops(), 1);
}

/// The server seat is as deep as the request queue it replaces, whatever
/// capacity the endpoint was made with, and its tiers scale with it. At 2 a
/// confirmed request still gets in behind an unconfirmed one (#2635), and
/// at 1 there is no reserve.
#[tokio::test]
async fn the_server_seat_takes_the_request_queues_depth_and_tiers() {
    for (capacity, tiers) in [
        (1, [1, 0, 0]),
        (2, [1, 1, 0]),
        (3, [1, 1, 1]),
        (16, [8, 4, 4]),
        (1024, [512, 256, 256]),
    ] {
        let mut node = node(capacity).await;
        let lease = node
            .coordinator
            .reserve(LeaseMetadata::notification(
                CanonicalPeer::direct(&PEER),
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            ))
            .unwrap();
        let answer = [0x20, lease.invoke_id(), 0x01];
        for apdu in [WHO_IS, READ_PROPERTY, &answer[..]] {
            for _ in 0..=capacity {
                inject(&node.link, apdu).await;
            }
        }
        settle(&mut node).await;

        let mut taken = [0; 3];
        while let Ok(received) = node.server.apdus.try_recv() {
            let tier = match received.apdu[0] >> 4 {
                1 => 0,
                0 => 1,
                _ => 2,
            };
            taken[tier] += 1;
        }
        assert_eq!(taken, tiers, "capacity {capacity}");
        let counters = node.endpoint.server_seat_counters();
        assert_eq!(counters.apdus, capacity as u64, "capacity {capacity}");
        assert_eq!(counters.full_drops, 2 * capacity as u64 + 3);
    }
}

/// A server's seat asked for at start is attached before the classifier
/// takes its first APDU (#2617). Requests already on the link, which the
/// classifier takes at once on another worker while the start is held
/// just after the classifier spawns, all reach the server's seat and none
/// the endpoint's own request queue. Real time on a multi-thread runtime,
/// where the classifier runs beside the start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_on_the_link_at_start_are_the_seated_servers() {
    let (transport, link) = test_transport();
    for _ in 0..6 {
        inject(&link, READ_PROPERTY).await;
    }
    let mut endpoint = EndpointIngress::new(transport, 16);
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let started = crate::endpoint_ingress::AFTER_CLASSIFIER_SPAWN.scope(
        Duration::from_millis(50),
        endpoint.start_with_server_seat(coordinator),
    );
    let (mut ingress, mut server) = timeout(Duration::from_secs(5), started)
        .await
        .expect("the start returned")
        .unwrap();
    for _ in 0..6 {
        receive(&mut server.apdus).await;
    }
    assert!(
        ingress.inbound_requests.try_recv().is_err(),
        "a request reached the endpoint's own queue"
    );
    assert_eq!(endpoint.server_seat_counters().apdus, 6);
    drop(server);
    timeout(Duration::from_secs(5), endpoint.stop())
        .await
        .expect("the stop returned")
        .unwrap();
}
