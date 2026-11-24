//! A server on a node's seat (#2617): it answers what the node hands it,
//! leases from the node's pool, and its stop leaves the node its link.
use super::*;
use crate::server::test_transport::{SendLog, TestTransport, TestTransportHandle};
use bacnet_endpoint_core::coordinator::{
    NOTIFICATION_RESERVE, OutboundTransactionCoordinator, REQUESTER_RESERVE,
};
use bacnet_endpoint_core::endpoint_ingress::{EndpointIngress, IngressReceivers};
use bacnet_network::response_route::ResponseRoute;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};

/// The requester every injected APDU comes from.
pub(super) const PEER: [u8; 1] = [0x11];

/// A node's link and ingress, with a seat for a server.
pub(super) struct Node {
    pub(super) ingress: EndpointIngress<TestTransport>,
    pub(super) _receivers: IngressReceivers,
    pub(super) inbound: mpsc::Sender<ReceivedNpdu>,
    pub(super) sent: SendLog,
    pub(super) link: TestTransportHandle,
    pub(super) coordinator: Arc<OutboundTransactionCoordinator>,
}

pub(super) async fn node() -> (Node, ServerSeat<TestTransport>) {
    let (transport, inbound) = TestTransport::inbound(8);
    let (sent, link) = (transport.sent(), transport.handle());
    let mut ingress = EndpointIngress::new(transport, 8);
    let receivers = ingress.start().await.unwrap();
    // A node with both roles keeps each its reserve (#2617).
    let coordinator = Arc::new(OutboundTransactionCoordinator::with_reserves(
        REQUESTER_RESERVE,
        NOTIFICATION_RESERVE,
    ));
    let seat = ingress.server_seat(Arc::clone(&coordinator)).unwrap();
    let node = Node {
        ingress,
        _receivers: receivers,
        inbound,
        sent,
        link,
        coordinator,
    };
    (node, seat)
}

/// A ReadProperty of the local Device's Object_Name under `invoke_id`, as
/// the link hands it up from [`PEER`].
fn read_device_name(invoke_id: u8) -> ReceivedNpdu {
    let mut parameters = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(
            ObjectType::DEVICE,
            ObjectIdentifier::MAX_INSTANCE,
        )
        .unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut parameters);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: parameters.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = vec![0x01, 0x04];
    npdu.extend_from_slice(&apdu);
    ReceivedNpdu {
        direct_response: None,
        npdu: Bytes::from(npdu),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// The APDUs sent to [`PEER`] so far.
pub(super) fn answers(sent: &SendLog) -> Vec<Apdu> {
    sent.frames()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == PEER)
        .filter_map(|frame| {
            let npdu = bacnet_encoding::npdu::decode_npdu(frame.npdu).ok()?;
            apdu::decode_apdu(npdu.payload).ok()
        })
        .collect()
}

/// Wait on paused time, a millisecond at a time, for `count` answers.
pub(super) async fn answered(sent: &SendLog, count: usize) -> Vec<Apdu> {
    for _ in 0..5_000 {
        let answers = answers(sent);
        if answers.len() >= count {
            return answers;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("{count} answers never came: {:?}", answers(sent));
}

#[tokio::test(start_paused = true)]
async fn a_seated_server_answers_and_its_stop_leaves_the_node_its_link() {
    let (mut node, seat) = node().await;
    let network = Arc::clone(&seat.network);
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .build()
        .await
        .unwrap();
    assert!(!server.owns_link());

    // A request the node's ingress hands the server is answered on the
    // node's link.
    node.inbound.send(read_device_name(7)).await.unwrap();
    let replies = answered(&node.sent, 1).await;
    assert!(
        matches!(&replies[..], [Apdu::ComplexAck(ack)] if ack.invoke_id == 7),
        "{replies:?}"
    );
    assert_eq!(node.ingress.server_seat_counters().apdus, 1);

    // The server's own requests lease from the node's pool.
    let reserved = server
        .notification_transactions
        .reserve(
            canonical_direct_peer(&PEER),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        )
        .unwrap();
    assert_eq!(node.coordinator.active_count().unwrap(), 1);
    drop(reserved);

    // Its stop joins its own work and leaves the link to the node: not
    // stopped, and its replies not sealed for the node's other roles.
    server.stop().await.unwrap();
    assert_eq!(node.link.stops(), 0);
    let reply = network.encode_response_for_handoff(
        bacnet_network::layer::IssuedApdu {
            apdu: &[0x20, 1, 0x01],
            next_hop: &PEER,
            destination: None,
            expecting_reply: false,
            priority: NetworkPriority::Normal,
        },
        &ResponseRoute::unverified(),
    );
    assert!(reply.is_ok(), "the layer's replies were sealed");
    // The node's stop stops the link, once.
    node.ingress.stop().await.unwrap();
    assert_eq!(node.link.stops(), 1);
    drop(server);
    assert_eq!(node.link.stops(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_seated_server_outlives_its_node_and_stops_cleanly() {
    let (mut node, seat) = node().await;
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .build()
        .await
        .unwrap();
    // The node stops first: the server's queue closes, and the link stops
    // under it, once.
    node.ingress.stop().await.unwrap();
    assert_eq!(node.link.stops(), 1);
    // What the server sends from now on fails as a stopped link's send.
    let error = server.broadcast_i_am().await.unwrap_err();
    assert!(error.to_string().contains("stopped"), "{error}");
    server.stop().await.unwrap();
    assert_eq!(node.link.stops(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_seat_refuses_what_only_a_link_of_its_own_can_take() {
    let port = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
    let (_node, seat) = node().await;
    let refused = BACnetServer::generic_builder()
        .registered_network_port(port)
        .seat(seat)
        .build()
        .await;
    let error = refused.err().expect("a registered port is refused");
    assert!(error.to_string().contains("Network Port"), "{error}");

    // Audit Reporters a seat can take (#2617, PR 2), but one the database
    // lacks fails the build, which leaves the node's link as it is.
    let (audited, seat) = node().await;
    let refused = BACnetServer::generic_builder()
        .audit_reporters(AuditReportersConfig {
            reporters: vec![ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap()],
        })
        .seat(seat)
        .build()
        .await;
    assert!(refused.is_err(), "a missing Audit Reporter is refused");
    assert_eq!(audited.link.stops(), 0);

    // One link or the other: a transport set first is refused, not
    // dropped unstarted.
    let (_node, seat) = node().await;
    let unused = TestTransport::new();
    let unused_link = unused.handle();
    let refused = BACnetServer::generic_builder()
        .transport(unused)
        .seat(seat)
        .build()
        .await;
    let error = refused.err().expect("a transport and a seat are refused");
    assert!(error.to_string().contains("transport"), "{error}");
    assert_eq!(unused_link.starts(), 0);
}

/// What the node's ingress hands a server held by
/// [`SeatedServerBuilder::hold_requests_until`] waits in the seat's queue,
/// unanswered, until the node lets go: then it is answered, once.
#[tokio::test(start_paused = true)]
async fn a_held_server_takes_nothing_until_the_node_lets_go() {
    for send in [true, false] {
        let (mut node, seat) = node().await;
        let (gate, held) = oneshot::channel();
        let mut server = BACnetServer::generic_builder()
            .seat(seat)
            .hold_requests_until(held)
            .build()
            .await
            .unwrap();
        node.inbound.send(read_device_name(7)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(answers(&node.sent).is_empty(), "answered while held");
        assert_eq!(
            node.ingress.server_seat_counters().apdus,
            1,
            "the request waits in the seat's queue"
        );
        // Sent, or dropped with a start that went no further.
        if send {
            gate.send(()).unwrap();
        } else {
            drop(gate);
        }
        let replies = answered(&node.sent, 1).await;
        assert!(
            matches!(&replies[..], [Apdu::ComplexAck(ack)] if ack.invoke_id == 7),
            "{replies:?}"
        );
        server.stop().await.unwrap();
        node.ingress.stop().await.unwrap();
    }
}

/// A server stopped while still held stops: its stop doesn't wait for the
/// node to let go, and it answers nothing it was holding.
#[tokio::test(start_paused = true)]
async fn a_held_server_stops_without_the_node_letting_go() {
    let (mut node, seat) = node().await;
    let (gate, held) = oneshot::channel::<()>();
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .hold_requests_until(held)
        .build()
        .await
        .unwrap();
    node.inbound.send(read_device_name(9)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), server.stop())
        .await
        .expect("the held server's stop returned")
        .unwrap();
    drop(gate);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(answers(&node.sent).is_empty(), "{:?}", answers(&node.sent));
    node.ingress.stop().await.unwrap();
}

/// The seated build can be spawned: its future is `Send`, as the
/// standalone build's is.
#[allow(dead_code)]
fn the_seated_build_is_send(builder: SeatedServerBuilder<TestTransport>) -> impl Send {
    builder.build()
}
