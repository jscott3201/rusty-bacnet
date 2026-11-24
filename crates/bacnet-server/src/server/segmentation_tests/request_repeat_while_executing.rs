//! A segment repeated while its request executes is acknowledged again with
//! the last sequence number and window, not aborted (Clause 5.4.5.3, #2751).
//! The request is held executing by the database's write lock, which the
//! WriteProperty it carries needs; the repeats come before its answer.

use super::*;
use request_peer_quota::assert_server_abort;
use request_reassembly::{
    inject_routed_apdu, present_value, split_into, start_routed_reassembly_server,
    write_property_payload,
};

struct Rig {
    server: BACnetServer<TestTransport>,
    incoming: mpsc::Sender<bacnet_transport::port::ReceivedNpdu>,
    sent: SendLog,
    index: usize,
    router: MacAddr,
    peer: NpduAddress,
}

impl Rig {
    async fn start() -> Self {
        let (server, incoming, sent) = start_routed_reassembly_server().await;
        Self {
            server,
            incoming,
            sent,
            index: 0,
            router: test_mac(30),
            peer: routed_address(400, 9),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn segment_from(
        &self,
        peer: &NpduAddress,
        invoke_id: u8,
        seq: u8,
        window: u8,
        more_follows: bool,
        data: &[u8],
    ) {
        let request = ConfirmedRequestPdu {
            segmented: true,
            more_follows,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: Some(seq),
            proposed_window_size: Some(window),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::copy_from_slice(data),
        };
        inject_routed_apdu(
            &self.incoming,
            &self.router,
            peer,
            &Apdu::ConfirmedRequest(request),
        )
        .await;
    }

    async fn segment(&self, invoke_id: u8, seq: u8, window: u8, more: bool, data: &[u8]) {
        let peer = self.peer.clone();
        self.segment_from(&peer, invoke_id, seq, window, more, data)
            .await;
    }

    /// The next reply, and the routed peer it went to. Waits in steps of a
    /// millisecond of the paused clock (a busy yield wouldn't advance it),
    /// for at most two seconds of it.
    async fn reply(&mut self) -> (Apdu, NpduAddress) {
        for _ in 0..2_000 {
            if sent_count(&self.sent) > self.index {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(sent_count(&self.sent) > self.index, "no reply came");
        let frame = self.sent.frame(self.index);
        self.index += 1;
        let npdu = frame.decode_npdu();
        (
            decode_apdu(npdu.payload).unwrap(),
            npdu.destination.expect("a routed reply"),
        )
    }

    async fn expect_ack(&mut self, invoke_id: u8, seq: u8, window: u8, context: &str) {
        let (reply, to) = self.reply().await;
        assert_eq!(to, self.peer, "{context}");
        match reply {
            Apdu::SegmentAck(ack) => {
                assert!(ack.sent_by_server && !ack.negative_ack, "{context}");
                assert_eq!(
                    (ack.invoke_id, ack.sequence_number, ack.actual_window_size),
                    (invoke_id, seq, window),
                    "{context}"
                );
            }
            other => panic!("{context}: expected a SegmentAck, got {other:?}"),
        }
    }

    /// Nothing more goes out for a while.
    async fn quiet(&self) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sent_count(&self.sent), self.index, "nothing more was sent");
    }

    fn notes(&self) -> usize {
        self.server.confirmed_request_tracker.segmented_notes()
    }

    /// Segment `seq` of a ReadProperty in `chunks`, at `window`, whose answer
    /// comes in segments: at a 50-octet APDU the long Present_Value
    /// [`long_value`] writes doesn't fit one.
    async fn read_segment(&self, invoke_id: u8, seq: usize, window: u8, chunks: &[Vec<u8>]) {
        let request = ConfirmedRequestPdu {
            segmented: true,
            more_follows: seq + 1 < chunks.len(),
            segmented_response_accepted: true,
            max_segments: Some(16),
            max_apdu_length: 50,
            invoke_id,
            sequence_number: Some(seq as u8),
            proposed_window_size: Some(window),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::copy_from_slice(&chunks[seq]),
        };
        let apdu = Apdu::ConfirmedRequest(request);
        inject_routed_apdu(&self.incoming, &self.router, &self.peer, &apdu).await;
    }

    /// Every segment of that ReadProperty at a window of two, back to back:
    /// queued together, before the server takes any.
    async fn read_request(&self, invoke_id: u8, chunks: &[Vec<u8>]) {
        for seq in 0..chunks.len() {
            self.read_segment(invoke_id, seq, 2, chunks).await;
        }
    }

    /// Take a segmented answer to `invoke_id`, acknowledging every segment
    /// it has, the last one too; returns how many there were. Nothing is
    /// awaited after the last acknowledgment is queued.
    async fn take_segmented_answer(&mut self, invoke_id: u8) -> u8 {
        let mut sequence = 0;
        loop {
            let (reply, _) = self.reply().await;
            let Apdu::ComplexAck(segment) = reply else {
                panic!("expected a segment of the answer, got {reply:?}");
            };
            assert!(segment.segmented);
            assert_eq!(
                (segment.invoke_id, segment.sequence_number),
                (invoke_id, Some(sequence))
            );
            let ack = Apdu::SegmentAck(segment_ack(invoke_id, false, sequence));
            inject_routed_apdu(&self.incoming, &self.router, &self.peer, &ack).await;
            sequence += 1;
            if !segment.more_follows {
                return sequence;
            }
        }
    }
}

/// The CSV object's Present_Value, long enough that a ReadProperty of it at
/// a 50-octet APDU is answered in segments; and that ReadProperty, in two.
async fn long_value(server: &BACnetServer<TestTransport>) -> Vec<Vec<u8>> {
    let oid = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    server
        .database()
        .write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString("v".repeat(300)),
            None,
        )
        .unwrap();
    let mut read = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut read);
    split_into(&read, 2)
}

#[tokio::test(start_paused = true)]
async fn repeated_segments_before_the_answer_are_acknowledged_again() {
    let mut rig = Rig::start().await;
    let text = "repeated while executing";
    let chunks = split_into(&write_property_payload(text), 6);
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    // Six segments at a window of three: answered at 0, 3 and 5.
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(4, seq as u8, 3, seq < 5, chunk).await;
    }
    rig.expect_ack(4, 0, 3, "segment zero").await;
    rig.expect_ack(4, 3, 3, "the first window").await;
    rig.expect_ack(4, 5, 3, "the last segment").await;
    rig.quiet().await;
    assert_eq!(rig.notes(), 1, "noted while it executes");
    // The final segment again, then a whole window again, segment zero
    // among them: each is answered with the last acknowledgment.
    rig.segment(4, 5, 3, false, &chunks[5]).await;
    rig.expect_ack(4, 5, 3, "the final segment again").await;
    for seq in 0..3 {
        rig.segment(4, seq, 3, true, &chunks[usize::from(seq)])
            .await;
        rig.expect_ack(4, 5, 3, "a whole window again").await;
    }
    rig.quiet().await;
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 4));
    assert_eq!(present_value(&rig.server).await, text);
    rig.quiet().await;
    rig.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_repeat_after_the_answer_or_from_another_peer_or_unknown_draws_an_abort() {
    let mut rig = Rig::start().await;
    let text = "answered, then repeated";
    let chunks = split_into(&write_property_payload(text), 3);
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(6, seq as u8, 1, seq < 2, chunk).await;
        rig.expect_ack(6, seq as u8, 1, "in order").await;
    }
    // Another peer, the same invoke ID: no transfer of its own, no note.
    let other = routed_address(400, 10);
    rig.segment_from(&other, 6, 1, 1, true, &chunks[1]).await;
    let (reply, to) = rig.reply().await;
    assert_eq!(to, other);
    assert_server_abort(reply, 6, AbortReason::INVALID_APDU_IN_THIS_STATE);
    // A transfer nobody opened.
    rig.segment(7, 1, 1, true, &chunks[1]).await;
    let (reply, _) = rig.reply().await;
    assert_server_abort(reply, 7, AbortReason::INVALID_APDU_IN_THIS_STATE);
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 6));
    assert_eq!(rig.notes(), 0, "the note ends with the request");
    // Answered: the server is idle for this transaction again.
    rig.segment(6, 2, 1, false, &chunks[2]).await;
    let (reply, _) = rig.reply().await;
    assert_server_abort(reply, 6, AbortReason::INVALID_APDU_IN_THIS_STATE);
    assert_eq!(present_value(&rig.server).await, text);
    rig.server.stop().await.unwrap();
}

/// An oversized request (over the 64 KiB the tracker keeps whole) has its
/// note too (#2751): its last window sent again, and the whole request sent
/// again from segment zero, while it executes, are each acknowledged again
/// with the last segment, and it runs once.
#[tokio::test(start_paused = true)]
async fn an_oversized_request_repeated_while_executing_runs_once() {
    let mut rig = Rig::start().await;
    let text = "o".repeat(70 * 1024);
    let payload = write_property_payload(&text);
    let chunks: Vec<&[u8]> = payload.chunks(1400).collect();
    let total = chunks.len();
    let last = (total - 1) as u8;
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(5, seq as u8, 8, seq + 1 < total, chunk).await;
    }
    for seq in (0..total).filter(|seq| seq % 8 == 0 || seq + 1 == total) {
        rig.expect_ack(5, seq as u8, 8, "in order").await;
    }
    rig.quiet().await;
    assert_eq!(rig.notes(), 1, "noted, though oversized");
    for (seq, chunk) in chunks.iter().enumerate().skip(total - 8) {
        rig.segment(5, seq as u8, 8, seq + 1 < total, chunk).await;
        rig.expect_ack(5, last, 8, "the last window again").await;
    }
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(5, seq as u8, 8, seq + 1 < total, chunk).await;
        rig.expect_ack(5, last, 8, "the whole request again").await;
    }
    rig.quiet().await;
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 5));
    rig.quiet().await;
    assert_eq!(present_value(&rig.server).await, text);
    rig.server.stop().await.unwrap();
}

/// A requester's Abort ends the note (#2751): a new transfer under that
/// invoke ID opens as new, with its own acknowledgment, not the old one.
#[tokio::test(start_paused = true)]
async fn a_requesters_abort_ends_the_note() {
    let mut rig = Rig::start().await;
    let chunks = split_into(&write_property_payload("aborted, then opened again"), 3);
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(8, seq as u8, 1, seq < 2, chunk).await;
        rig.expect_ack(8, seq as u8, 1, "in order").await;
    }
    rig.quiet().await;
    assert_eq!(rig.notes(), 1);
    let abort = Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id: 8,
        abort_reason: AbortReason::OTHER,
    });
    inject_routed_apdu(&rig.incoming, &rig.router, &rig.peer, &abort).await;
    rig.segment(8, 0, 2, true, &chunks[0]).await;
    rig.expect_ack(8, 0, 2, "a new transfer").await;
    assert_eq!(rig.notes(), 0);
    inject_routed_apdu(&rig.incoming, &rig.router, &rig.peer, &abort).await;
    rig.quiet().await;
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 8));
    rig.server.stop().await.unwrap();
}

/// Another peer's Abort with the same invoke ID ends nothing of this one's:
/// its note stays, and a repeat of its final segment is acknowledged again.
#[tokio::test(start_paused = true)]
async fn another_peers_abort_leaves_the_note() {
    let mut rig = Rig::start().await;
    let chunks = split_into(&write_property_payload("not yours to abort"), 3);
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    for (seq, chunk) in chunks.iter().enumerate() {
        rig.segment(9, seq as u8, 1, seq < 2, chunk).await;
        rig.expect_ack(9, seq as u8, 1, "in order").await;
    }
    let abort = Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id: 9,
        abort_reason: AbortReason::OTHER,
    });
    let other = routed_address(400, 10);
    inject_routed_apdu(&rig.incoming, &rig.router, &other, &abort).await;
    rig.quiet().await;
    assert_eq!(rig.notes(), 1);
    rig.segment(9, 2, 1, false, &chunks[2]).await;
    rig.expect_ack(9, 2, 1, "the final segment again").await;
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 9));
    rig.server.stop().await.unwrap();
}

/// A requester that gave up on a request (no Abort) and reuses its invoke
/// ID while the server still executes it: the new request's segments are,
/// by Clause 5.4.5.3, repeats of the one executing, each answered with its
/// last acknowledgment, and open no transfer. (So a second note under one
/// requester and invoke ID can't come from the wire this way; the newest
/// one wins where two exist: confirmed_tracker_tests.)
#[tokio::test(start_paused = true)]
async fn an_invoke_id_reused_while_its_request_executes_opens_nothing() {
    let mut rig = Rig::start().await;
    let first = split_into(&write_property_payload("the first request"), 6);
    let second = split_into(&write_property_payload("the second request"), 3);
    let db = Arc::clone(rig.server.database());
    let guard = db.write().await;
    for (seq, chunk) in first.iter().enumerate() {
        rig.segment(5, seq as u8, 1, seq < 5, chunk).await;
        rig.expect_ack(5, seq as u8, 1, "the first, in order").await;
    }
    for (seq, chunk) in second.iter().enumerate() {
        rig.segment(5, seq as u8, 1, seq < 2, chunk).await;
        rig.expect_ack(5, 5, 1, "the second: the first's last ack")
            .await;
    }
    rig.quiet().await;
    drop(guard);
    let (answer, _) = rig.reply().await;
    assert!(matches!(answer, Apdu::SimpleAck(ack) if ack.invoke_id == 5));
    rig.quiet().await;
    assert_eq!(present_value(&rig.server).await, "the first request");
    rig.server.stop().await.unwrap();
}

/// A segmented answer's transaction ends at its final Segment-ACK, in the
/// order frames arrive (Clause 5.4.5.4): the requester may reuse the invoke
/// ID once it has sent that acknowledgment, so the same request sent again
/// right behind it is a new one. Its segment zero is acknowledged as such,
/// not with the last acknowledgment of the request just answered, and it is
/// executed and answered again, not discarded as a duplicate. The final
/// acknowledgment and both segments are queued before the server runs, so
/// on this single-threaded runtime the dispatch loop takes all three in one
/// turn, ahead of the sending task the acknowledgment wakes.
#[tokio::test(start_paused = true)]
async fn the_same_request_right_behind_the_final_ack_of_its_answer_runs_again() {
    let mut rig = Rig::start().await;
    let chunks = long_value(&rig.server).await;
    for seq in 0..2 {
        rig.read_segment(81, seq, 1, &chunks).await;
        rig.expect_ack(81, seq as u8, 1, "in order").await;
    }
    let segments = rig.take_segmented_answer(81).await;
    assert!(segments > 1);
    // Right behind the final acknowledgment, the whole request at a window
    // of two.
    rig.read_request(81, &chunks).await;
    rig.expect_ack(81, 0, 2, "segment zero of the request sent again")
        .await;
    rig.expect_ack(81, 1, 2, "its final segment").await;
    assert_eq!(rig.take_segmented_answer(81).await, segments);
    rig.quiet().await;
    assert_eq!(rig.notes(), 0);
    rig.server.stop().await.unwrap();
}

/// So does a requester's Abort of the answer (Clause 5.4.5.4): the same
/// request queued right behind it is new, and answered.
#[tokio::test(start_paused = true)]
async fn the_same_request_right_behind_an_abort_of_its_answer_runs_again() {
    let mut rig = Rig::start().await;
    let chunks = long_value(&rig.server).await;
    rig.read_request(81, &chunks).await;
    rig.expect_ack(81, 0, 2, "segment zero").await;
    rig.expect_ack(81, 1, 2, "the final segment").await;
    let (reply, _) = rig.reply().await;
    assert!(matches!(reply, Apdu::ComplexAck(ref s) if s.sequence_number == Some(0)));
    let abort = Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id: 81,
        abort_reason: AbortReason::OTHER,
    });
    inject_routed_apdu(&rig.incoming, &rig.router, &rig.peer, &abort).await;
    rig.read_request(81, &chunks).await;
    rig.expect_ack(81, 0, 2, "segment zero of the request sent again")
        .await;
    rig.expect_ack(81, 1, 2, "its final segment").await;
    assert!(rig.take_segmented_answer(81).await > 1);
    rig.quiet().await;
    rig.server.stop().await.unwrap();
}
