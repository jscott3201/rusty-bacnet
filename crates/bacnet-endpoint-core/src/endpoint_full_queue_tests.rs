//! A full endpoint queue (#2635): each class is dropped and counted when
//! it finds no room, each queue keeps its reserve for what can be spared
//! least, drops are logged at a rate that can't flood, and classification
//! never ends by the volume of traffic.
use super::*;

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
/// class makes the line a warning, on an interval of its own that no debug
/// line delays.
#[tokio::test(start_paused = true)]
async fn full_queue_drops_log_at_most_once_per_interval_with_a_count() {
    use crate::endpoint_ingress::routing::{DropReport, Dropped, IngressDrops};
    let report = |dropped, warned| Some(DropReport { dropped, warned });
    let drops = IngressDrops::default();
    assert_eq!(drops.flush(), None, "nothing to report");
    assert_eq!(drops.add(Dropped::Unconfirmed), report(1, false));
    for _ in 0..1_000 {
        assert_eq!(drops.add(Dropped::Unconfirmed), None);
    }
    // A confirmed request dropped soon after a debug line is warned of at
    // once, with the unconfirmed drops since that line.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(drops.add(Dropped::Confirmed), report(1_001, true));
    // Within the warning's own interval, more wait, whatever their class.
    assert_eq!(drops.add(Dropped::Answer), None);
    assert_eq!(drops.add(Dropped::Unconfirmed), None);
    tokio::time::sleep(DROP_LOG_INTERVAL - Duration::from_millis(1)).await;
    assert_eq!(drops.add(Dropped::Unconfirmed), None);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(drops.add(Dropped::Policy), report(4, true));
    let counters = drops.counters();
    assert_eq!(counters.unconfirmed_full_drops, 1_003);
    assert_eq!(counters.confirmed_full_drops, 1);
    assert_eq!(counters.answer_full_drops, 1);
    assert_eq!(counters.policy_full_drops, 1);

    // Drops after the last line wait for no further drop: ingress's end
    // flushes them, however recent that line.
    for _ in 0..5 {
        assert_eq!(drops.add(Dropped::Unconfirmed), None);
    }
    assert_eq!(drops.flush(), report(5, false));
    assert_eq!(drops.flush(), None);
}

/// A capacity-1 endpoint nothing reads, and four Who-Is: the first is
/// queued, the first drop is logged, and two drops wait for a line.
async fn endpoint_with_an_unreported_tail() -> (EndpointIngress<TestTransport>, IngressReceivers) {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    for _ in 0..4 {
        inject(&handle, &[0x10, 0x08]).await;
    }
    inject(&handle, &[0x80]).await;
    receive(&mut ingress.policy_outcomes).await;
    assert_eq!(endpoint.full_drop_counters().unconfirmed_full_drops, 3);
    (endpoint, ingress)
}

/// The drops after the last line are reported when classification ends,
/// at stop (#2635).
#[tokio::test(start_paused = true)]
async fn stop_reports_the_drops_no_line_has_reported() {
    let (mut endpoint, _ingress) = endpoint_with_an_unreported_tail().await;
    let drops = Arc::clone(&endpoint.drops);
    endpoint.stop().await.unwrap();
    assert_eq!(drops.flush(), None, "the session task reported the tail");
}

/// And when the endpoint is dropped without a stop, its task aborted.
#[tokio::test(start_paused = true)]
async fn drop_reports_the_drops_no_line_has_reported() {
    let (endpoint, _ingress) = endpoint_with_an_unreported_tail().await;
    let drops = Arc::clone(&endpoint.drops);
    drop(endpoint);
    assert_eq!(drops.flush(), None, "Drop reported the tail");
}

/// Which answers count as held by a lease at the answer queue's reserve: a
/// segment, a Segment-ACK and an Abort from the answering side for the
/// lease. A requester's Segment-ACK or Abort is about a request the node
/// serves, so none is held, whatever lease its invoke ID matches (#2640).
#[tokio::test(start_paused = true)]
async fn the_lease_look_holds_only_the_answering_sides_pdus() {
    use crate::coordinator::{
        CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator, TerminalPolicy,
    };
    use bacnet_types::enums::ConfirmedServiceChoice;
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let (transport, handle) = test_transport();
    let mut endpoint =
        EndpointIngress::new(transport, 16).with_coordinator(Arc::clone(&coordinator));
    let mut ingress = endpoint.start().await.unwrap();
    let lease = coordinator
        .reserve(LeaseMetadata::segmented_requester(
            CanonicalPeer::direct(&[0x11]),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::EitherAck,
        ))
        .unwrap();
    let id = lease.invoke_id();
    // Twelve strays fill the queue down to its reserve of four; a
    // thirteenth is dropped.
    for n in 0..13u8 {
        inject(&handle, &[0x20, id.wrapping_add(1 + n), 0x0f]).await;
    }
    let segment = [0x3C, id, 0x00, 0x04, 0x0C, 0x09, 0x55];
    let segment_ack_from_server = [0x41, id, 0x00, 0x04];
    let abort_from_server = [0x71, id, 0x04];
    let segment_ack_from_requester = [0x40, id, 0x00, 0x04];
    let stray_abort_from_requester = [0x70, id.wrapping_add(7), 0x04];
    inject(&handle, &segment).await;
    inject(&handle, &segment_ack_from_server).await;
    inject(&handle, &stray_abort_from_requester).await;
    inject(&handle, &abort_from_server).await;
    inject(&handle, &segment_ack_from_requester).await;
    inject(&handle, &[0x80]).await;
    receive(&mut ingress.policy_outcomes).await;

    let queued: Vec<Vec<u8>> = std::iter::from_fn(|| ingress.terminal_or_segment.try_recv().ok())
        .map(|received| received.apdu.to_vec())
        .collect();
    assert_eq!(queued.len(), 15);
    assert_eq!(
        queued[12..],
        [
            segment.to_vec(),
            segment_ack_from_server.to_vec(),
            abort_from_server.to_vec(),
        ]
    );
    assert_eq!(endpoint.full_drop_counters().answer_full_drops, 3);
    endpoint.stop().await.unwrap();
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
            std::mem::size_of::<crate::endpoint_ingress::egress::NetworkServiceCommand>(),
        ),
    ];
    for (name, size) in sizes {
        assert!(size <= 192, "{name} is {size} bytes, over 192");
    }
}
