//! The line from a node's server to its client (#2743): a
//! ConfirmedEventNotification that came in segments reaches the client seat
//! only once the server has it whole, as one APDU with no reply authority,
//! by the rule and in the tier of a copy of one that came whole, within a
//! bound on the octets such copies hold. Its segments never reach the client
//! seat.
use super::*;
use crate::coordinator::OutboundTransactionCoordinator;
use crate::seat::{
    CLIENT_SEAT_QUEUE, ClientSeat, MAX_REASSEMBLED_COPY_OCTETS, SeatRoles, ServerSeat,
};
use bacnet_encoding::apdu::{ConfirmedRequest, encode_apdu};
use bacnet_types::enums::ConfirmedServiceChoice;

const PEER: [u8; 1] = [0x11];

/// A first segment of a ConfirmedEventNotification: the service choice
/// sits after the sequence number and window size.
const SEGMENTED_EVENT_NOTIFICATION: &[u8] = &[0x0c, 0x05, 0x27, 0x00, 0x01, 0x02, 0xaa];
const EVENT_NOTIFICATION: &[u8] = &[0x00, 0x05, 0x23, 0x02, 0x09, 0x01];

struct Node {
    endpoint: EndpointIngress<TestTransport>,
    ingress: IngressReceivers,
    link: Link,
    client: ClientSeat<TestTransport>,
    server: ServerSeat<TestTransport>,
}

/// A started endpoint with a server seat and a client seat of `capacity`,
/// beside a server role or not.
async fn node(server_role: bool, capacity: usize) -> Node {
    let (transport, link) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let ingress = endpoint.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let client = endpoint
        .client_seat(
            Arc::clone(&coordinator),
            SeatRoles::new(server_role),
            capacity,
        )
        .unwrap();
    let server = endpoint.server_seat(coordinator).unwrap();
    Node {
        endpoint,
        ingress,
        link,
        client,
        server,
    }
}

/// `service` with `parameters`, whole, as the receiver hands it back.
fn whole(service: ConfirmedServiceChoice, parameters: &'static [u8]) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(64),
        max_apdu_length: 480,
        invoke_id: 5,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: Bytes::from_static(parameters),
    }
}

/// The last segment's envelope, as the server took it from its seat: from
/// [`PEER`], with its reply authority.
async fn last_segment(node: &mut Node) -> ReceivedApdu {
    let (reply_tx, _reply_rx) = oneshot::channel();
    let mut received = received_npdu(SEGMENTED_EVENT_NOTIFICATION);
    received.reply_tx = Some(reply_tx);
    node.link.sender.send(received).await.unwrap();
    receive(&mut node.server.apdus).await
}

fn encoded(request: &ConfirmedRequest) -> Vec<u8> {
    let mut apdu = BytesMut::new();
    encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(request.clone())).unwrap();
    apdu.to_vec()
}

#[tokio::test]
async fn the_client_hears_a_reassembled_event_notification_once_and_whole() {
    let mut node = node(true, 8).await;
    // Ingress gave the segment to the server alone.
    let last = last_segment(&mut node).await;
    assert!(last.reply_tx.is_some());
    assert!(
        node.client.apdus.try_recv().is_err(),
        "no copy of a segment"
    );
    assert_eq!(node.endpoint.client_seat_counters().apdus, 0);

    let event = whole(
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        &[0x09, 0x01],
    );
    node.server.client_copies.reassembled(&last, &event);
    let copy = receive(&mut node.client.apdus).await;
    assert_eq!(copy.apdu.as_ref(), encoded(&event));
    // The unsegmented form, with the service choice where a whole one has it.
    assert_eq!(copy.apdu[0], 0x02);
    assert_eq!(copy.apdu[3], 0x02);
    assert_eq!(copy.source_mac.as_slice(), PEER);
    assert!(copy.reply_tx.is_none());
    assert!(copy.direct_response.is_none() && copy.router_response.is_none());
    assert!(node.client.apdus.try_recv().is_err());
    assert!(node.server.apdus.try_recv().is_err());
    assert!(node.ingress.inbound_requests.try_recv().is_err());
    assert_eq!(node.endpoint.client_seat_counters().apdus, 1);
}

#[tokio::test]
async fn only_what_the_client_hears_a_copy_of_whole_is_handed_on() {
    // Beside the server role, a reassembled request of any other service is
    // the server's alone; a COV notification never reaches the server in
    // segments, as the client takes them.
    let mut node = node(true, 8).await;
    let last = last_segment(&mut node).await;
    for service in [
        ConfirmedServiceChoice::WRITE_PROPERTY,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
    ] {
        node.server
            .client_copies
            .reassembled(&last, &whole(service, &[0x09, 0x01]));
    }
    assert!(node.client.apdus.try_recv().is_err());

    // A client seat with no server role beside it takes every segment
    // itself, so the server never has one to hand on.
    let mut alone = self::node(false, 8).await;
    inject(&alone.link, SEGMENTED_EVENT_NOTIFICATION).await;
    let segment = receive(&mut alone.client.apdus).await;
    assert_eq!(segment.apdu.as_ref(), SEGMENTED_EVENT_NOTIFICATION);
    let event = whole(
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        &[0x09, 0x01],
    );
    alone.server.client_copies.reassembled(&segment, &event);
    assert!(alone.client.apdus.try_recv().is_err());
    assert!(alone.server.apdus.try_recv().is_err());
}

/// A client seat that falls behind: the copy takes a confirmed request's
/// tier, as a whole notification's copy does, and once only the seat's
/// reserve is left it is dropped and counted, without waiting.
#[tokio::test]
async fn a_full_client_seat_drops_and_counts_the_copy() {
    let mut node = node(true, 4).await;
    let last = last_segment(&mut node).await;
    let event = whole(
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        &[0x09, 0x01],
    );
    // A seat of 4 keeps 1 slot from confirmed requests: 3 copies fit, of
    // either kind.
    inject(&node.link, EVENT_NOTIFICATION).await;
    receive(&mut node.server.apdus).await;
    node.server.client_copies.reassembled(&last, &event);
    node.server.client_copies.reassembled(&last, &event);
    node.server.client_copies.reassembled(&last, &event);
    let counters = node.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (3, 1));
    for expected in [
        EVENT_NOTIFICATION.to_vec(),
        encoded(&event),
        encoded(&event),
    ] {
        assert_eq!(
            node.client.apdus.try_recv().unwrap().apdu.as_ref(),
            expected
        );
    }
    assert!(node.client.apdus.try_recv().is_err());
}

/// Once the client's queue is gone, or the endpoint has stopped, nothing is
/// handed on and nothing fails.
#[tokio::test]
async fn with_no_client_nothing_is_handed_on() {
    let Node {
        mut endpoint,
        link,
        client,
        mut server,
        ..
    } = node(true, 8).await;
    let (reply_tx, _reply_rx) = oneshot::channel();
    let mut received = received_npdu(SEGMENTED_EVENT_NOTIFICATION);
    received.reply_tx = Some(reply_tx);
    link.sender.send(received).await.unwrap();
    let last = receive(&mut server.apdus).await;
    let event = whole(
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        &[0x09, 0x01],
    );
    drop(client);
    server.client_copies.reassembled(&last, &event);
    assert_eq!(endpoint.client_seat_counters().apdus, 0);

    let copies = server.client_copies.clone();
    timeout(WAIT, endpoint.stop()).await.unwrap().unwrap();
    drop((endpoint, server));
    copies.reassembled(&last, &event);
}

/// Copies of reassembled requests hold at most
/// [`MAX_REASSEMBLED_COPY_OCTETS`] on the client seat: one past it is
/// dropped and counted before it is made, and each copy's octets come back
/// once the client lets go of it. A copy of a request that came whole is
/// not counted there.
#[tokio::test]
async fn reassembled_copies_hold_a_bounded_number_of_octets() {
    let mut node = node(true, CLIENT_SEAT_QUEUE).await;
    let last = last_segment(&mut node).await;
    let large = MAX_REASSEMBLED_COPY_OCTETS / 4;
    let event = ConfirmedRequest {
        service_request: Bytes::from(vec![0x09; large]),
        ..whole(ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION, &[])
    };
    let octets = large + 4;
    let seats = Arc::clone(node.endpoint.seats.as_ref().unwrap());
    for _ in 0..4 {
        node.server.client_copies.reassembled(&last, &event);
    }
    // Three fit; a fourth would pass the bound.
    let counters = node.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (3, 1));
    assert_eq!(seats.reassembled_octets(), 3 * octets);
    inject(&node.link, EVENT_NOTIFICATION).await;
    receive(&mut node.server.apdus).await;
    assert_eq!(node.endpoint.client_seat_counters().apdus, 4);
    assert_eq!(seats.reassembled_octets(), 3 * octets, "a whole one's copy");

    // Held while the client holds the copy, given back once it lets go.
    let taken = node.client.apdus.try_recv().unwrap();
    assert_eq!(taken.apdu.len(), octets);
    assert_eq!(seats.reassembled_octets(), 3 * octets);
    drop(taken);
    assert_eq!(seats.reassembled_octets(), 2 * octets);
    node.server.client_copies.reassembled(&last, &event);
    assert_eq!(node.endpoint.client_seat_counters().apdus, 5);
    while node.client.apdus.try_recv().is_ok() {}
    assert_eq!(seats.reassembled_octets(), 0);
}

/// Room found on one client seat goes with the copy to that seat only: a
/// client made on a new seat in between gets nothing its own checks didn't
/// admit, and the copy is dropped and counted.
#[tokio::test]
async fn a_copy_goes_only_to_the_seat_its_room_was_found_on() {
    let Node {
        endpoint,
        link,
        client,
        mut server,
        ..
    } = node(true, 8).await;
    let mut received = received_npdu(SEGMENTED_EVENT_NOTIFICATION);
    received.reply_tx = Some(oneshot::channel().0);
    link.sender.send(received).await.unwrap();
    let last = receive(&mut server.apdus).await;
    let seats = Arc::clone(endpoint.seats.as_ref().unwrap());
    let reserved = seats
        .with(|client, _| seats.reserve_reassembled(client?, 6))
        .flatten()
        .expect("room on the first seat");

    drop(client);
    let mut again = endpoint
        .client_seat(Arc::clone(&server.coordinator), SeatRoles::new(true), 8)
        .unwrap();
    let crate::seat::ReservedCopy { seat, budget } = reserved;
    drop(budget);
    seats.offer_reassembled(&seat, last);
    assert!(again.apdus.try_recv().is_err(), "nothing on the new seat");
    let counters = endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (0, 1));
}
