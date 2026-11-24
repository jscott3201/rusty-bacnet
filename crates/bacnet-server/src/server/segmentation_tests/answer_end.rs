//! Which requester PDUs end a segmented answer's pending request as the
//! dispatch loop takes them (#2767): the final Segment-ACK it queues,
//! positive or negative, and the requester's Abort; nothing else. A handle
//! holding a real tracker entry stands in for the sending task, and each
//! PDU goes through `dispatch`.

use super::*;

const INVOKE: u8 = 0x51;
/// Segments of the stand-in answer.
const TOTAL: usize = 3;

fn received(mac: &MacAddr) -> bacnet_network::layer::ReceivedApdu {
    bacnet_network::layer::ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: mac.clone(),
        ingress_network: None,
        source_network: None,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// How the stand-in answer is set when the PDU comes.
#[derive(Clone, Copy)]
enum Queue {
    Open,
    Full,
    Closed,
}

/// Whether `apdu`, from `from`, ends the pending request of an answer whose
/// segment `current` is in flight.
async fn ends(current: u16, queue: Queue, from: &MacAddr, apdu: Apdu) -> bool {
    let (network, _sent) = recording_network();
    let ctx =
        DispatchContext::for_test(RequestServices::for_test(network, ServerConfig::default()));
    let peer = test_mac(40);
    let request = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 50,
        invoke_id: INVOKE,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from_static(b"read"),
    };
    let tracker = &ctx.confirmed_request_tracker;
    let begin = || {
        tracker.begin(
            peer.as_slice(),
            None,
            TransportProvenance::unverified(),
            request.clone(),
        )
    };
    let ConfirmedRequestAdmission::New(pending) = begin() else {
        panic!("a new request")
    };
    let (tx, rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = watch::channel(None);
    let handle = Arc::new(SegmentedSendHandle::new(
        tx,
        control_tx,
        TOTAL,
        pending.id(),
    ));
    handle.current_sequence.store(current, Ordering::Release);
    match queue {
        Queue::Open => {}
        Queue::Full => handle
            .segment_ack_tx
            .try_send(segment_ack(INVOKE, false, 0))
            .unwrap(),
        Queue::Closed => drop(rx),
    }
    let key = segmented_transaction_key(
        peer.as_slice(),
        None,
        INVOKE,
        TransportProvenance::unverified(),
    );
    ctx.services.seg_ack_senders.lock().insert(key, handle);
    BACnetServer::<TestTransport>::dispatch(&ctx, from.as_slice(), apdu, received(from)).await;
    let ended = matches!(begin(), ConfirmedRequestAdmission::New(_));
    drop(pending);
    ended
}

#[tokio::test]
async fn only_the_final_ack_or_the_requesters_abort_ends_the_answered_request() {
    let peer = test_mac(40);
    let ack = |negative, sequence| Apdu::SegmentAck(segment_ack(INVOKE, negative, sequence));
    let abort = |sent_by_server| {
        Apdu::Abort(AbortPdu {
            sent_by_server,
            invoke_id: INVOKE,
            abort_reason: AbortReason::OTHER,
        })
    };
    let mut server_ack = segment_ack(INVOKE, false, 2);
    server_ack.sent_by_server = true;
    let cases = [
        (
            "the first segment's ack",
            0,
            Queue::Open,
            &peer,
            ack(false, 0),
            false,
        ),
        (
            "a middle segment's ack",
            1,
            Queue::Open,
            &peer,
            ack(false, 1),
            false,
        ),
        ("the final ack", 2, Queue::Open, &peer, ack(false, 2), true),
        (
            "a negative ack naming the last segment",
            2,
            Queue::Open,
            &peer,
            ack(true, 2),
            true,
        ),
        (
            "a negative ack asking for a segment again",
            2,
            Queue::Open,
            &peer,
            ack(true, 1),
            false,
        ),
        ("a stale ack", 2, Queue::Open, &peer, ack(false, 1), false),
        (
            "the final ack, its queue full",
            2,
            Queue::Full,
            &peer,
            ack(false, 2),
            false,
        ),
        (
            "the final ack, its sender gone",
            2,
            Queue::Closed,
            &peer,
            ack(false, 2),
            false,
        ),
        (
            "the final ack from another peer",
            2,
            Queue::Open,
            &test_mac(41),
            ack(false, 2),
            false,
        ),
        (
            "a server's ack",
            2,
            Queue::Open,
            &peer,
            Apdu::SegmentAck(server_ack),
            false,
        ),
        (
            "the requester's Abort",
            0,
            Queue::Open,
            &peer,
            abort(false),
            true,
        ),
        (
            "a server's Abort",
            0,
            Queue::Open,
            &peer,
            abort(true),
            false,
        ),
    ];
    for (what, current, queue, from, apdu, expected) in cases {
        assert_eq!(ends(current, queue, from, apdu).await, expected, "{what}");
    }
}

/// A send closed while it waits for an acknowledgment sends nothing more,
/// not even the segment its timeout would send again: once a requester's
/// Abort is routed, a new request under the same invoke ID may already be
/// running, and a stale segment would reach it. On a multi-thread runtime
/// the Abort can land after the wait chose its timeout; `send_control`
/// stores the flag before the event, so setting the flag alone holds that
/// moment still.
#[tokio::test(start_paused = true)]
async fn a_send_closed_as_its_timeout_fires_sends_no_segment_again() {
    let (network, sent) = recording_network();
    let registry = Arc::new(segmented_send::SegmentedSendRegistry::default());
    let peer = test_mac(42);
    let task = spawn_segmented_complex_ack_with_options(
        network,
        Arc::clone(&registry),
        peer.clone(),
        INVOKE,
        vec![0xA5; 128],
        SegmentedSendOptions {
            segment_timeout: Duration::from_secs(1),
            max_retries: 2,
        },
    );
    wait_for_sent_len(&sent, 1).await;
    // Every ready task runs before the paused clock moves: the send is
    // parked in its wait for segment zero's acknowledgment.
    tokio::time::sleep(Duration::from_millis(1)).await;
    let key = segmented_transaction_key(
        peer.as_slice(),
        None,
        INVOKE,
        TransportProvenance::unverified(),
    );
    let handle = registry.lock().get(&key).cloned().expect("registered");
    handle.closed.store(true, Ordering::Release);
    drop(handle);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(task.is_finished(), "the send ended");
    assert_eq!(sent_count(&sent), 1, "segment zero was not sent again");
    task.await.unwrap();
}

/// The final Segment-ACK also stops the answer from sending again (#2770):
/// a resend of the last segment its timer chose just before it reads that
/// acknowledgment is refused, so no old segment follows a new request under
/// the reused invoke ID. An acknowledgment short of the last leaves it be.
#[tokio::test]
async fn the_final_ack_refuses_a_resend_of_the_last_segment() {
    for current in [2u16, 1] {
        let (network, _sent) = recording_network();
        let ctx =
            DispatchContext::for_test(RequestServices::for_test(network, ServerConfig::default()));
        let peer = test_mac(43);
        let (tx, _rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = watch::channel(None);
        let handle = Arc::new(SegmentedSendHandle::new(tx, control_tx, TOTAL, None));
        handle.current_sequence.store(current, Ordering::Release);
        let last = usize::from(current) + 1 == TOTAL;
        if last {
            handle.last_sent();
        }
        let key = segmented_transaction_key(
            peer.as_slice(),
            None,
            INVOKE,
            TransportProvenance::unverified(),
        );
        ctx.services
            .seg_ack_senders
            .lock()
            .insert(key, Arc::clone(&handle));
        let ack = Apdu::SegmentAck(segment_ack(INVOKE, false, current as u8));
        BACnetServer::<TestTransport>::dispatch(&ctx, peer.as_slice(), ack, received(&peer)).await;
        assert_eq!(handle.sending(), !last, "ack of segment {current}");
    }
}
