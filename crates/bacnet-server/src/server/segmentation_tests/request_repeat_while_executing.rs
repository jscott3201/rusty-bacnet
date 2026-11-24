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

    /// The next reply, and the routed peer it went to.
    async fn reply(&mut self) -> (Apdu, NpduAddress) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while sent_count(&self.sent) <= self.index {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for a reply");
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
