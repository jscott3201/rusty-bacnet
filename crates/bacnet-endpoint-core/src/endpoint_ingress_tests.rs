use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_npdu::NetworkPriority;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_transport::test_transport::{SendMode, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Duration, timeout};

use super::*;

const WAIT: Duration = Duration::from_secs(1);

/// The test's side of a [`test_transport`] or [`blocking_transport`] link.
struct Link {
    sender: mpsc::Sender<ReceivedNpdu>,
    sent: mpsc::Receiver<(Vec<u8>, Vec<u8>)>,
    counters: TestTransportHandle,
}

impl Link {
    fn starts(&self) -> usize {
        self.counters.starts()
    }

    fn stops(&self) -> usize {
        self.counters.stops()
    }
}

/// A link at MAC 0xaa whose unicasts land in the link's `sent` (a full or
/// closed queue fails the send), whose broadcasts vanish, and for which
/// MAC 0xFF reaches a group of nodes.
fn test_transport() -> (TestTransport, Link) {
    let (sender, receiver) = mpsc::channel(32);
    let (sent_tx, sent_rx) = mpsc::channel(32);
    let transport = TestTransport::builder()
        .local_mac(&[0xaa])
        .inbound(receiver)
        .broadcast(SendMode::Ignore)
        .on_is_group_destination(|mac| mac == [0xFF])
        .on_send(move |frame| {
            let sent = sent_tx.clone();
            async move {
                sent.send((frame.npdu.to_vec(), frame.mac.to_vec()))
                    .await
                    .map_err(|_| Error::Encoding("test sent-frame receiver closed".into()))
            }
        })
        .build();
    let counters = transport.handle();
    (
        transport,
        Link {
            sender,
            sent: sent_rx,
            counters,
        },
    )
}

/// The test's side of a [`blocking_transport`] link.
struct BlockingLink {
    sender: mpsc::Sender<ReceivedNpdu>,
    entered_send: Arc<Notify>,
    counters: TestTransportHandle,
}

impl BlockingLink {
    fn stops(&self) -> usize {
        self.counters.stops()
    }
}

/// A link whose every unicast tells `entered_send` and then never finishes;
/// broadcasts vanish.
fn blocking_transport() -> (TestTransport, BlockingLink) {
    let (sender, receiver) = mpsc::channel(1);
    let entered_send = Arc::new(Notify::new());
    let entered = Arc::clone(&entered_send);
    let transport = TestTransport::builder()
        .local_mac(&[0xaa])
        .inbound(receiver)
        .broadcast(SendMode::Ignore)
        .on_send(move |_| {
            entered.notify_one();
            std::future::pending()
        })
        .build();
    let counters = transport.handle();
    (
        transport,
        BlockingLink {
            sender,
            entered_send,
            counters,
        },
    )
}

fn npdu_bytes(apdu: &[u8]) -> Bytes {
    let npdu = Npdu {
        expecting_reply: false,
        priority: NetworkPriority::Normal,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    buffer.freeze()
}

fn received_npdu(apdu: &[u8]) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu_bytes(apdu),
        source_mac: MacAddr::from_slice(&[0x11]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

async fn inject(handle: &Link, apdu: &[u8]) {
    handle.sender.send(received_npdu(apdu)).await.unwrap();
}

async fn receive<T>(receiver: &mut mpsc::Receiver<T>) -> T {
    timeout(WAIT, receiver.recv())
        .await
        .expect("ingress receive timed out")
        .expect("ingress route closed")
}

#[tokio::test]
async fn loopback_routes_all_apdu_classes_once_and_in_order() {
    let (transport, peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(transport, 8);
    let mut ingress = endpoint.start().await.unwrap();
    let apdus: [&[u8]; 8] = [
        &[0x00, 0x05, 0x11, 0x0c],
        &[0x20, 0x12, 0x0c],
        &[0x10, 0x08],
        &[0x30, 0x13, 0x0c],
        &[0x50, 0x14, 0x0c, 0x91, 0x00, 0x91, 0x00],
        &[0x60, 0x15, 0x00],
        &[0x70, 0x16, 0x00],
        &[0x40, 0x17, 0x00, 0x01],
    ];

    for apdu in apdus {
        peer.send_unicast(&npdu_bytes(apdu), &[0x01]).await.unwrap();
    }

    let mut inbound_types = Vec::new();
    for _ in 0..2 {
        inbound_types.push(receive(&mut ingress.inbound_requests).await.apdu[0] >> 4);
    }
    let mut terminal_types = Vec::new();
    for _ in 0..6 {
        terminal_types.push(receive(&mut ingress.terminal_or_segment).await.apdu[0] >> 4);
    }

    assert_eq!(inbound_types, [0, 1]);
    assert_eq!(terminal_types, [2, 3, 5, 6, 7, 4]);
    assert!(ingress.inbound_requests.try_recv().is_err());
    assert!(ingress.terminal_or_segment.try_recv().is_err());
    assert!(ingress.policy_outcomes.try_recv().is_err());
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
}

#[tokio::test]
async fn direct_egress_preserves_npdu_flags_priority_and_destination() {
    let (transport, mut handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress.clone();

    egress
        .send_direct(
            vec![0x20, 0x2a, 0x0c],
            MacAddr::from_slice(&[0x44, 0x55]),
            true,
            NetworkPriority::Urgent,
        )
        .await
        .unwrap();

    let (npdu, destination) = timeout(WAIT, handle.sent.recv()).await.unwrap().unwrap();
    let decoded = decode_npdu(Bytes::from(npdu)).unwrap();
    assert_eq!(destination, [0x44, 0x55]);
    assert!(decoded.expecting_reply);
    assert_eq!(decoded.priority, NetworkPriority::Urgent);
    assert_eq!(decoded.payload.as_ref(), &[0x20, 0x2a, 0x0c]);

    endpoint.stop().await.unwrap();
    assert!(matches!(
        egress
            .send_direct(
                vec![0x20, 0x2b, 0x0c],
                MacAddr::from_slice(&[0x44]),
                false,
                NetworkPriority::Normal,
            )
            .await,
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));
}

#[tokio::test]
async fn saturated_egress_is_bounded_and_stop_cancels_accepted_commands() {
    let (transport, handle) = blocking_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress.clone();

    let first_egress = egress.clone();
    let first = tokio::spawn(async move {
        first_egress
            .send_direct(
                vec![0x20, 0x01, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::Normal,
            )
            .await
    });
    timeout(WAIT, handle.entered_send.notified()).await.unwrap();

    let second_egress = egress.clone();
    let second = tokio::spawn(async move {
        second_egress
            .send_direct(
                vec![0x20, 0x02, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::Normal,
            )
            .await
    });
    tokio::task::yield_now().await;

    assert!(matches!(
        egress
            .send_direct(
                vec![0x20, 0x03, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::Normal,
            )
            .await,
        Err(Error::Encoding(message)) if message == "endpoint egress queue is full"
    ));

    handle
        .sender
        .send(received_npdu(&[0x20, 0x44, 0x0c]))
        .await
        .unwrap();
    let terminal = timeout(WAIT, ingress.terminal_or_segment.recv())
        .await
        .expect("blocked egress starved terminal classification")
        .expect("terminal route closed while egress was blocked");
    assert_eq!(terminal.apdu.as_ref(), &[0x20, 0x44, 0x0c]);

    assert!(matches!(
        timeout(WAIT, endpoint.stop()).await.unwrap().unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(handle.stops(), 1);
    for result in [first.await.unwrap(), second.await.unwrap()] {
        assert!(matches!(
            result,
            Err(Error::Encoding(message)) if message == "endpoint shutdown"
        ));
    }
}

#[tokio::test]
async fn malformed_and_unsupported_apdus_have_policy_outcomes() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 4);
    let mut ingress = endpoint.start().await.unwrap();

    inject(&handle, &[]).await;
    inject(&handle, &[0x80]).await;

    let malformed = receive(&mut ingress.policy_outcomes).await;
    assert_eq!(malformed.reason, PolicyReason::MalformedApdu);
    assert!(malformed.received.apdu.is_empty());
    let unsupported = receive(&mut ingress.policy_outcomes).await;
    assert_eq!(unsupported.reason, PolicyReason::UnsupportedPduType(8));
    assert_eq!(unsupported.received.apdu.as_ref(), &[0x80]);
    assert!(ingress.inbound_requests.try_recv().is_err());
    assert!(ingress.terminal_or_segment.try_recv().is_err());

    endpoint.stop().await.unwrap();
}

/// A confirmed request from a group address goes to policy, not to the
/// server role, whose answer would reach every node in the group (#1504).
#[tokio::test]
async fn a_confirmed_request_from_a_group_source_goes_to_policy() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 4);
    let mut ingress = endpoint.start().await.unwrap();
    // ReadProperty, and Who-Is.
    let confirmed = [
        0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
    ];
    let unconfirmed = [0x10, 0x08];
    for apdu in [&confirmed[..], &unconfirmed] {
        let mut from_group = received_npdu(apdu);
        from_group.source_mac = MacAddr::from_slice(&[0xFF]);
        handle.sender.send(from_group).await.unwrap();
    }
    inject(&handle, &confirmed).await;

    let refused = receive(&mut ingress.policy_outcomes).await;
    assert_eq!(refused.reason, PolicyReason::GroupSource);
    assert_eq!(refused.received.source_mac[..], [0xFF]);
    // The group's unconfirmed request, and a station's confirmed one, still
    // reach the role.
    let who_is = receive(&mut ingress.inbound_requests).await;
    assert_eq!(who_is.apdu.as_ref(), unconfirmed);
    let request = receive(&mut ingress.inbound_requests).await;
    assert_eq!(request.source_mac[..], [0x11]);
    assert!(ingress.policy_outcomes.try_recv().is_err());

    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn request_route_preserves_the_complete_envelope_and_reply_sender() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let mut ingress = endpoint.start().await.unwrap();
    // A global broadcast carries only an Unconfirmed-Request (#1491).
    let apdu = [0x10, 0x08];
    let source_network = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0x44, 0x55]),
    };
    let destination = NpduAddress {
        network: 0xffff,
        mac_address: MacAddr::new(),
    };
    let npdu = Npdu {
        expecting_reply: true,
        priority: NetworkPriority::Urgent,
        destination: Some(destination),
        source: Some(source_network.clone()),
        hop_count: 255,
        payload: Bytes::copy_from_slice(&apdu),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    let attributes = vec![DataAttribute {
        option_type: 9,
        must_understand: true,
        data: vec![1, 2, 3],
    }];
    let (reply_tx, reply_rx) = oneshot::channel();
    handle
        .sender
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: buffer.freeze(),
            source_mac: MacAddr::from_slice(&[0xde, 0xad]),
            link_layer_group: true,
            data_attributes: attributes.clone(),
            provenance: TransportProvenance::unverified(),
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap();

    let mut received = receive(&mut ingress.inbound_requests).await;
    assert_eq!(received.apdu.as_ref(), apdu);
    assert_eq!(received.source_mac.as_slice(), &[0xde, 0xad]);
    assert_eq!(received.source_network, Some(source_network));
    assert!(received.link_layer_group);
    assert!(received.is_group);
    assert_eq!(received.data_attributes, attributes);
    received
        .reply_tx
        .take()
        .expect("reply sender was lost")
        .send(Bytes::from_static(b"reply"))
        .unwrap();
    assert_eq!(reply_rx.await.unwrap().as_ref(), b"reply");
    assert!(ingress.terminal_or_segment.try_recv().is_err());
    assert!(ingress.policy_outcomes.try_recv().is_err());

    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn duplicate_start_is_rejected_before_transport_and_stop_is_single_owner() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    assert_eq!(handle.starts(), 1);

    assert!(endpoint.start().await.is_err());
    assert_eq!(handle.starts(), 1);
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(handle.stops(), 1);
    assert!(endpoint.stop().await.is_err());
    assert_eq!(handle.stops(), 1);
    assert!(ingress.inbound_requests.recv().await.is_none());
    assert!(ingress.terminal_or_segment.recv().await.is_none());
    assert!(ingress.policy_outcomes.recv().await.is_none());
}

#[tokio::test]
async fn zero_capacity_is_rejected_before_transport_start() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 0);

    assert!(endpoint.start().await.is_err());
    assert_eq!(handle.starts(), 0);
    assert_eq!(handle.stops(), 0);
}

#[tokio::test]
async fn full_and_closed_role_routes_are_explicit_and_stop_does_not_block() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let ingress = endpoint.start().await.unwrap();
    let mut inbound = ingress.inbound_requests;
    let terminal = ingress.terminal_or_segment;
    let mut policies = ingress.policy_outcomes;

    inject(&handle, &[0x00, 0x05, 0x01, 0x0c]).await;
    inject(&handle, &[0x00, 0x05, 0x02, 0x0c]).await;

    drop(terminal);
    inject(&handle, &[0x20, 0x03, 0x0c]).await;
    let closed = receive(&mut policies).await;
    assert_eq!(
        closed.reason,
        PolicyReason::RouteClosed(IngressRoute::TerminalOrSegment)
    );
    assert_eq!(closed.received.apdu[1], 3);
    // The request the full queue had no room for was dropped and counted,
    // and never handed to policy.
    assert_eq!(endpoint.full_drop_counters().confirmed_full_drops, 1);
    assert!(policies.try_recv().is_err());

    timeout(WAIT, endpoint.stop())
        .await
        .expect("stop blocked on a saturated role route")
        .unwrap();
    assert_eq!(handle.stops(), 1);
    assert_eq!(inbound.recv().await.unwrap().apdu[2], 1);
    assert!(inbound.recv().await.is_none());
    assert!(policies.recv().await.is_none());
}

/// Waits on paused time, 1 ms at a time, until `ready` holds; panics with
/// `what` after [`WAIT`].
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !ready() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "not within {WAIT:?}: {what}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// `apdu` with a one-use reply sender, as MS/TP hands up a request whose
/// sender waits for the reply.
async fn inject_expecting_reply(handle: &Link, apdu: &[u8]) -> oneshot::Receiver<Bytes> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut received = received_npdu(apdu);
    received.reply_tx = Some(reply_tx);
    handle.sender.send(received).await.unwrap();
    reply_rx
}

/// A flood of drops logs one line per [`DROP_LOG_INTERVAL`], carrying the
/// count since the line before, so the report can't become a flood itself.
/// Unconfirmed drops alone are routine and go out at debug level; any other
/// class makes the line a warning. The tail is reported when ingress ends.
#[tokio::test(start_paused = true)]
async fn full_queue_drops_log_at_most_once_per_interval_with_a_count() {
    use super::routing::{DropReport, Dropped, IngressDrops};
    let report = |dropped, warned| Some(DropReport { dropped, warned });
    let drops = IngressDrops::default();
    assert_eq!(drops.flush(), None, "nothing to report");
    assert_eq!(drops.add(Dropped::Unconfirmed), report(1, false));
    for _ in 0..1_000 {
        assert_eq!(drops.add(Dropped::Unconfirmed), None);
    }
    tokio::time::sleep(DROP_LOG_INTERVAL - Duration::from_millis(1)).await;
    assert_eq!(drops.add(Dropped::Answer), None);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(drops.add(Dropped::Policy), report(1_002, true));
    let counters = drops.counters();
    assert_eq!(counters.unconfirmed_full_drops, 1_001);
    assert_eq!(counters.answer_full_drops, 1);
    assert_eq!(counters.policy_full_drops, 1);
    assert_eq!(counters.confirmed_full_drops, 0);

    // Drops after the last line wait for no further drop: ingress's end
    // flushes them, however recent that line.
    for _ in 0..5 {
        assert_eq!(drops.add(Dropped::Unconfirmed), None);
    }
    assert_eq!(drops.flush(), report(5, false));
    assert_eq!(drops.flush(), None);
}

/// A queue of capacity two or more keeps at least one slot Who-Is can't
/// take, so a confirmed request behind a burst of them still gets in; a
/// queue of one has no reserve (#2635).
#[tokio::test(start_paused = true)]
async fn every_queue_of_two_or_more_keeps_a_slot_from_unconfirmed_requests() {
    const WHO_IS: &[u8] = &[0x10, 0x08];
    for (capacity, who_is_kept) in [(1, 1), (2, 1), (3, 2), (4, 3), (8, 6)] {
        let (transport, handle) = test_transport();
        let mut endpoint = EndpointIngress::new(transport, capacity);
        let mut ingress = endpoint.start().await.unwrap();
        for _ in 0..capacity + 2 {
            inject(&handle, WHO_IS).await;
        }
        inject(&handle, &[0x00, 0x05, 0x01, 0x0c]).await;
        // A marker the policy queue takes, classified after the rest.
        inject(&handle, &[0x80]).await;
        receive(&mut ingress.policy_outcomes).await;

        let queued: Vec<_> = std::iter::from_fn(|| ingress.inbound_requests.try_recv().ok())
            .map(|received| received.apdu)
            .collect();
        let confirmed_kept = usize::from(capacity >= 2);
        assert_eq!(
            queued.len(),
            who_is_kept + confirmed_kept,
            "capacity {capacity}"
        );
        assert!(
            queued[..who_is_kept]
                .iter()
                .all(|apdu| apdu.as_ref() == WHO_IS)
        );
        let counters = endpoint.full_drop_counters();
        assert_eq!(
            counters.unconfirmed_full_drops,
            (capacity + 2 - who_is_kept) as u64,
            "capacity {capacity}"
        );
        assert_eq!(
            counters.confirmed_full_drops,
            u64::from(capacity < 2),
            "capacity {capacity}"
        );
        endpoint.stop().await.unwrap();
    }
}

/// With the endpoint's coordinator, an answer no lease holds may fill the
/// answer queue only down to its reserve: stray answers ahead of the
/// endpoint's own can't take its slot (#2635). The look claims nothing.
#[tokio::test(start_paused = true)]
async fn an_answer_no_lease_holds_cant_take_the_last_slot_from_one_a_lease_holds() {
    use crate::coordinator::{
        AdmissionOutcome, CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator,
        TerminalPolicy,
    };
    use bacnet_types::enums::ConfirmedServiceChoice;

    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let (transport, handle) = test_transport();
    let mut endpoint =
        EndpointIngress::new(transport, 4).with_coordinator(Arc::clone(&coordinator));
    let mut ingress = endpoint.start().await.unwrap();
    let peer = CanonicalPeer::direct(&[0x11]);
    let lease = coordinator
        .reserve(LeaseMetadata::requester(
            peer.clone(),
            ConfirmedServiceChoice::WRITE_PROPERTY,
            TerminalPolicy::SimpleAck,
        ))
        .unwrap();
    let ours = [0x20, lease.invoke_id(), 0x0f];
    let stray = |n: u8| [0x20, lease.invoke_id().wrapping_add(1 + n), 0x0f];

    // Four strays: three fit, the fourth would take the reserve.
    for n in 0..4 {
        inject(&handle, &stray(n)).await;
    }
    // Ours takes the reserve; one more stray finds the queue full.
    inject(&handle, &ours).await;
    inject(&handle, &stray(4)).await;
    inject(&handle, &[0x80]).await;
    receive(&mut ingress.policy_outcomes).await;

    let queued: Vec<_> = std::iter::from_fn(|| ingress.terminal_or_segment.try_recv().ok())
        .map(|received| received.apdu)
        .collect();
    assert_eq!(queued.len(), 4);
    assert_eq!(queued[3].as_ref(), ours);
    assert_eq!(endpoint.full_drop_counters().answer_full_drops, 2);
    // Nothing was claimed: the lease's owner still admits its answer.
    let Ok(Apdu::SimpleAck(ack)) = decode_apdu(Bytes::copy_from_slice(&ours)) else {
        panic!("a Simple-ACK");
    };
    assert!(matches!(
        coordinator.admit(&peer, &Apdu::SimpleAck(ack)).unwrap(),
        AdmissionOutcome::Admitted(_)
    ));
    endpoint.stop().await.unwrap();
}

/// Nothing reads the endpoint's queues: each class that finds its queue
/// full is dropped and counted, an Unconfirmed-Request already at the
/// quarter kept for confirmed ones, and classification goes on (#2635).
#[tokio::test(start_paused = true)]
async fn full_queues_drop_and_count_by_class_and_never_end_classification() {
    const WHO_IS: &[u8] = &[0x10, 0x08];
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 4);
    let mut ingress = endpoint.start().await.unwrap();

    // Three of four slots are the Who-Is share; the fourth is kept.
    for _ in 0..6 {
        inject(&handle, WHO_IS).await;
    }
    // The kept slot takes a confirmed request; the next finds the queue
    // full, and its reply sender goes with it: on MS/TP the MAC then sends
    // Reply Postponed at once.
    let mut kept = inject_expecting_reply(&handle, &[0x00, 0x05, 0x01, 0x0c]).await;
    let dropped = inject_expecting_reply(&handle, &[0x00, 0x05, 0x02, 0x0c]).await;
    // Answers no lease holds, and APDUs of a PDU type nothing defines.
    for invoke_id in 0..6 {
        inject(&handle, &[0x20, invoke_id, 0x0c]).await;
    }
    for _ in 0..6 {
        inject(&handle, &[0x80]).await;
    }

    until("every class counted", || {
        endpoint.full_drop_counters().policy_full_drops == 2
    })
    .await;
    let counters = endpoint.full_drop_counters();
    assert_eq!(counters.unconfirmed_full_drops, 3);
    assert_eq!(counters.confirmed_full_drops, 1);
    assert_eq!(counters.answer_full_drops, 2);
    assert!(
        dropped.await.is_err(),
        "a dropped request's reply sender is released"
    );
    assert!(matches!(
        kept.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    // What fit is queued in order, the kept request with its reply sender.
    for _ in 0..3 {
        assert_eq!(
            ingress.inbound_requests.recv().await.unwrap().apdu.as_ref(),
            WHO_IS
        );
    }
    let request = ingress.inbound_requests.recv().await.unwrap();
    assert_eq!(request.apdu[2], 1);
    assert!(request.reply_tx.is_some());
    for invoke_id in 0..4 {
        assert_eq!(
            ingress.terminal_or_segment.recv().await.unwrap().apdu[1],
            invoke_id
        );
    }
    for _ in 0..4 {
        let outcome = ingress.policy_outcomes.recv().await.unwrap();
        assert_eq!(outcome.reason, PolicyReason::UnsupportedPduType(8));
    }

    // Classification went on throughout.
    inject(&handle, &[0x00, 0x05, 0x03, 0x0c]).await;
    inject(&handle, WHO_IS).await;
    assert_eq!(receive(&mut ingress.inbound_requests).await.apdu[2], 3);
    assert_eq!(
        receive(&mut ingress.inbound_requests).await.apdu.as_ref(),
        WHO_IS
    );
    assert_eq!(endpoint.full_drop_counters(), counters);
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
    // The counts outlive the stop.
    assert_eq!(endpoint.full_drop_counters(), counters);
}

#[path = "endpoint_ingress_lifecycle_tests.rs"]
mod lifecycle;

/// The client seat: who receives what, and how the link stops (#2617).
#[path = "endpoint_seat_tests.rs"]
mod seat;

/// The memory bound `SessionConfig::queue_capacity` documents rests on
/// these sizes (#2635): a slot of at most 192 bytes in each of the four
/// queues. Shrink a type that grows past it, or rework that bound.
#[test]
fn queued_items_stay_within_the_documented_slot_size() {
    let sizes = [
        (
            "ReceivedApdu",
            std::mem::size_of::<bacnet_network::layer::ReceivedApdu>(),
        ),
        ("PolicyOutcome", std::mem::size_of::<PolicyOutcome>()),
        (
            "NetworkServiceCommand",
            std::mem::size_of::<egress::NetworkServiceCommand>(),
        ),
    ];
    for (name, size) in sizes {
        assert!(size <= 192, "{name} is {size} bytes, over 192");
    }
}
