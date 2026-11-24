//! A confirmed request that comes while the server still sends a segmented
//! answer under the same requester and invoke ID, before the requester's
//! final Segment-ACK (Clause 5.4.5.4, #2770). The same request again keeps
//! the handling it had: dropped as a duplicate whole, or, for a segment of
//! a request that came in segments, acknowledged again; the answer goes on.
//! A different one, midway, draws one Abort (INVALID_APDU_IN_THIS_STATE),
//! is not executed, and ends the answer: nothing of it follows the Abort,
//! requests under the invoke ID are dropped until that Abort is handed to
//! the transport, and then the invoke ID is free. Once every segment has
//! gone out, with none on its way and only the final Segment-ACK due, a
//! different one takes that acknowledgment as lost and is served, with no
//! Abort and nothing more of the answer.
//!
//! The requester is a routed peer; its ReadProperty of a long Present_Value
//! at a 50-octet APDU is answered in segments, which the test acknowledges
//! one by one, or leaves unacknowledged so the answer stays in progress.

use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::SentFrame;
use request_peer_quota::assert_server_abort;
use request_reassembly::{
    inject_routed_apdu, present_value, routed_injection_transport, split_into,
    write_property_payload,
};

const INVOKE: u8 = 0x5A;
/// Seconds of quiet that outlast every resend of an answer segment (four
/// sends, a 5 s segment timeout each).
const PAST_EVERY_RESEND: Duration = Duration::from_secs(30);
/// Quiet short of the 5 s segment timeout, so the resend of a segment comes
/// within the two seconds a reply is waited for.
const BEFORE_THE_RESEND: Duration = Duration::from_secs(4);

fn long_value() -> String {
    "v".repeat(300)
}

fn csv() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap()
}

/// A ReadProperty of the long Present_Value, whose answer comes in segments.
fn read(invoke_id: u8) -> ConfirmedRequestPdu {
    let mut body = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: csv(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut body);
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(16),
        max_apdu_length: 50,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: body.freeze(),
    }
}

/// A WriteProperty of `text` to that Present_Value.
fn write(invoke_id: u8, text: &str) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        max_apdu_length: 1476,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::from(write_property_payload(text)),
        ..read(invoke_id)
    }
}

/// A LifeSafetyOperation, which the CSV object can't take: executed, it
/// draws an Error.
fn life_safety_operation(invoke_id: u8) -> ConfirmedRequestPdu {
    let mut body = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 41,
        requesting_source: "operator".into(),
        request: bacnet_types::enums::LifeSafetyOperation::SILENCE,
        object_identifier: None,
    }
    .encode(&mut body)
    .unwrap();
    ConfirmedRequestPdu {
        service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        service_request: body.freeze(),
        ..write(invoke_id, "")
    }
}

/// `request` sent in `count` segments at a window of one.
fn in_segments(request: &ConfirmedRequestPdu, count: usize) -> Vec<Apdu> {
    let chunks = split_into(&request.service_request, count);
    chunks
        .iter()
        .enumerate()
        .map(|(seq, chunk)| {
            Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: true,
                more_follows: seq + 1 < count,
                sequence_number: Some(seq as u8),
                proposed_window_size: Some(1),
                service_request: Bytes::copy_from_slice(chunk),
                ..request.clone()
            })
        })
        .collect()
}

struct Rig {
    server: BACnetServer<TestTransport>,
    incoming: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    link: TestTransportHandle,
    index: usize,
    router: MacAddr,
    peer: NpduAddress,
}

impl Rig {
    async fn start() -> Self {
        let (transport, incoming, sent) = routed_injection_transport();
        let link = transport.handle();
        let mut db = ObjectDatabase::new();
        let mut csv = CharacterStringValueObject::new(1, "CSV-1").unwrap();
        csv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::CharacterString(long_value()),
            None,
        )
        .unwrap();
        db.add(Box::new(csv)).unwrap();
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::default()
                .with_segmentation(bacnet_types::enums::Segmentation::SEGMENTED_BOTH),
            ..ServerConfig::default()
        };
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        Self {
            server,
            incoming,
            sent,
            link,
            index: 0,
            router: test_mac(30),
            peer: routed_address(400, 9),
        }
    }

    async fn send_from(&self, peer: &NpduAddress, apdu: Apdu) {
        inject_routed_apdu(&self.incoming, &self.router, peer, &apdu).await;
    }

    async fn send(&self, apdu: Apdu) {
        self.send_from(&self.peer, apdu).await;
    }

    async fn request(&self, request: ConfirmedRequestPdu) {
        self.send(Apdu::ConfirmedRequest(request)).await;
    }

    /// `apdu` from a peer on the server's own network at link address
    /// `mac`, not through a router.
    async fn send_direct(&self, mac: &MacAddr, apdu: Apdu) {
        let mut apdu_buf = BytesMut::new();
        encode_apdu(&mut apdu_buf, &apdu).unwrap();
        let npdu = bacnet_encoding::npdu::Npdu {
            payload: apdu_buf.freeze(),
            ..Default::default()
        };
        let mut npdu_buf = BytesMut::new();
        bacnet_encoding::npdu::encode_npdu(&mut npdu_buf, &npdu).unwrap();
        self.incoming
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu_buf.freeze(),
                source_mac: mac.clone(),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    /// The next frame sent, with its routed destination and link address.
    /// Waits in steps of a millisecond of the paused clock, for at most two
    /// seconds of it: less than a segment timeout, so a segment sent again
    /// can't pass for a reply.
    async fn reply(&mut self) -> (Apdu, Option<NpduAddress>, MacAddr) {
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
            npdu.destination,
            frame.mac,
        )
    }

    /// The next frame, which goes to the requester.
    async fn reply_to_peer(&mut self) -> Apdu {
        let (apdu, to, _) = self.reply().await;
        assert_eq!(to.as_ref(), Some(&self.peer), "{apdu:?}");
        apdu
    }

    /// Nothing more goes out for `time`.
    async fn quiet(&self, time: Duration) {
        tokio::time::sleep(time).await;
        assert_eq!(
            sent_count(&self.sent),
            self.index,
            "nothing more was sent: {:?}",
            self.sent.frames().get(self.index).map(SentFrame::apdu)
        );
    }

    /// The next frame is segment `seq` of the answer to `invoke_id`;
    /// returns whether more follow.
    async fn answer_segment(&mut self, invoke_id: u8, seq: u8) -> bool {
        match self.reply_to_peer().await {
            Apdu::ComplexAck(segment) => {
                assert!(segment.segmented);
                assert_eq!(
                    (segment.invoke_id, segment.sequence_number),
                    (invoke_id, Some(seq))
                );
                segment.more_follows
            }
            other => panic!("expected segment {seq} of the answer, got {other:?}"),
        }
    }

    /// Acknowledge segment `seq` of the answer to `invoke_id`.
    async fn ack(&self, invoke_id: u8, seq: u8) {
        self.send(Apdu::SegmentAck(segment_ack(invoke_id, false, seq)))
            .await;
    }

    /// The rest of the answer whose segment `from` was the last one taken,
    /// each acknowledged; returns the number of segments.
    async fn rest_of_answer(&mut self, invoke_id: u8, from: u8) -> u8 {
        let mut seq = from;
        loop {
            self.ack(invoke_id, seq).await;
            seq += 1;
            if !self.answer_segment(invoke_id, seq).await {
                self.ack(invoke_id, seq).await;
                return seq + 1;
            }
        }
    }

    /// Segments `seqs` of the answer to `invoke_id`, none the last, each
    /// taken and acknowledged.
    async fn take_and_ack(&mut self, invoke_id: u8, seqs: std::ops::Range<u8>) {
        for seq in seqs {
            assert!(self.answer_segment(invoke_id, seq).await);
            self.ack(invoke_id, seq).await;
        }
    }

    /// How many segments the answer to [`read`] has: one read under
    /// another invoke ID, taken whole.
    async fn segments_in_answer(&mut self) -> u8 {
        let probe = INVOKE + 1;
        self.request(read(probe)).await;
        assert!(self.answer_segment(probe, 0).await);
        let total = self.rest_of_answer(probe, 0).await;
        assert!(total > 2);
        total
    }

    async fn expect_ack(&mut self, invoke_id: u8, seq: u8, window: u8, context: &str) {
        match self.reply_to_peer().await {
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

    /// The server's Abort for a request that came during an answer.
    async fn expect_abort(&mut self, invoke_id: u8) {
        let reply = self.reply_to_peer().await;
        assert_server_abort(reply, invoke_id, AbortReason::INVALID_APDU_IN_THIS_STATE);
    }

    async fn expect_simple_ack(&mut self, invoke_id: u8) {
        let reply = self.reply_to_peer().await;
        assert!(
            matches!(reply, Apdu::SimpleAck(ref ack) if ack.invoke_id == invoke_id),
            "{reply:?}"
        );
    }

    async fn value(&self) -> String {
        present_value(&self.server).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_different_request_during_a_segmented_answer_draws_one_abort() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    rig.request(write(INVOKE, "not executed")).await;
    rig.expect_abort(INVOKE).await;
    // The answer ended with it: no segment again, past every resend.
    rig.quiet(PAST_EVERY_RESEND).await;
    assert_eq!(rig.value().await, long_value());
    // The invoke ID is free: the same request again is a new one.
    rig.request(write(INVOKE, "not executed")).await;
    rig.expect_simple_ack(INVOKE).await;
    assert_eq!(rig.value().await, "not executed");
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// The service choice is part of the request: the same service data under
/// another service is a different request.
#[tokio::test(start_paused = true)]
async fn the_same_data_under_another_service_draws_the_abort() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    let mut other = read(INVOKE);
    other.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
    rig.request(other).await;
    rig.expect_abort(INVOKE).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A request that comes in segments is known whole only once its last one
/// is in: its segments are acknowledged, then the Abort comes.
#[tokio::test(start_paused = true)]
async fn a_different_request_in_segments_is_acknowledged_then_aborted() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    let segments = in_segments(&write(INVOKE, "not executed either"), 3);
    for (seq, segment) in segments.iter().enumerate() {
        rig.send(segment.clone()).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "a segment of the new request")
            .await;
    }
    rig.expect_abort(INVOKE).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    assert_eq!(rig.value().await, long_value());
    for (seq, segment) in segments.into_iter().enumerate() {
        rig.send(segment).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "sent again, a new request")
            .await;
    }
    rig.expect_simple_ack(INVOKE).await;
    assert_eq!(rig.value().await, "not executed either");
    rig.server.stop().await.unwrap();
}

/// The answer to a request that came in segments: segment zero of a
/// different request opens a transfer of its own, rather than drawing the
/// last acknowledgment of the request being answered, and is aborted once
/// whole.
#[tokio::test(start_paused = true)]
async fn after_a_segmented_request_a_different_one_in_segments_is_aborted() {
    let mut rig = Rig::start().await;
    for (seq, segment) in in_segments(&read(INVOKE), 2).into_iter().enumerate() {
        rig.send(segment).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "the read").await;
    }
    assert!(rig.answer_segment(INVOKE, 0).await);
    let segments = in_segments(&write(INVOKE, "not executed"), 3);
    for (seq, segment) in segments.into_iter().enumerate() {
        rig.send(segment).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "a segment of the new request")
            .await;
    }
    rig.expect_abort(INVOKE).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    assert_eq!(rig.value().await, long_value());
    rig.server.stop().await.unwrap();
}

/// The same request again, whole, is dropped as a duplicate, and the answer
/// goes on: a requester that lost the answer's first segment and sent its
/// request again still gets it from the server's own resend.
#[tokio::test(start_paused = true)]
async fn the_same_request_again_is_dropped_and_the_answer_completes() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    rig.request(read(INVOKE)).await;
    rig.quiet(BEFORE_THE_RESEND).await;
    // The segment timeout passes: segment zero again.
    assert!(rig.answer_segment(INVOKE, 0).await);
    assert!(rig.rest_of_answer(INVOKE, 0).await > 2);
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// The same request again in segments, from segment zero, after one that
/// came in segments: a transfer of its own, acknowledged as one, dropped as
/// a duplicate once whole; the answer goes on.
#[tokio::test(start_paused = true)]
async fn the_same_segmented_request_again_is_reassembled_and_dropped() {
    let mut rig = Rig::start().await;
    let segments = in_segments(&read(INVOKE), 2);
    for (seq, segment) in segments.iter().enumerate() {
        rig.send(segment.clone()).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "the read").await;
    }
    assert!(rig.answer_segment(INVOKE, 0).await);
    for (seq, segment) in segments.into_iter().enumerate() {
        rig.send(segment).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "the read again").await;
    }
    rig.quiet(BEFORE_THE_RESEND).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    assert!(rig.rest_of_answer(INVOKE, 0).await > 2);
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A segment other than zero of the request being answered, sent again, is
/// acknowledged again with its last acknowledgment, as while it executed
/// (#2751); the answer goes on.
#[tokio::test(start_paused = true)]
async fn a_repeated_segment_of_the_request_is_acknowledged_again() {
    let mut rig = Rig::start().await;
    let segments = in_segments(&read(INVOKE), 2);
    for (seq, segment) in segments.iter().enumerate() {
        rig.send(segment.clone()).await;
        rig.expect_ack(INVOKE, seq as u8, 1, "the read").await;
    }
    assert!(rig.answer_segment(INVOKE, 0).await);
    rig.send(segments[1].clone()).await;
    rig.expect_ack(INVOKE, 1, 1, "its last segment again").await;
    assert!(rig.rest_of_answer(INVOKE, 0).await > 2);
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// Other requesters under the same invoke ID end nothing: another address
/// behind the router, the same address on another network, and the router
/// itself as a peer on the server's own network. Each is served, and the
/// answer goes on.
#[tokio::test(start_paused = true)]
async fn another_requester_under_the_invoke_id_is_served() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    for (peer, text) in [
        (routed_address(400, 10), "another address"),
        (routed_address(401, 9), "another network"),
    ] {
        rig.send_from(&peer, Apdu::ConfirmedRequest(write(INVOKE, text)))
            .await;
        let (reply, to, _) = rig.reply().await;
        assert_eq!(to, Some(peer));
        assert!(matches!(reply, Apdu::SimpleAck(ack) if ack.invoke_id == INVOKE));
        assert_eq!(rig.value().await, text);
    }
    let router = rig.router.clone();
    rig.send_direct(&router, Apdu::ConfirmedRequest(write(INVOKE, "direct")))
        .await;
    let (reply, to, mac) = rig.reply().await;
    assert_eq!((to, mac), (None, router));
    assert!(matches!(reply, Apdu::SimpleAck(ack) if ack.invoke_id == INVOKE));
    assert_eq!(rig.value().await, "direct");
    assert!(rig.rest_of_answer(INVOKE, 0).await > 2);
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A LifeSafetyOperation goes through a replay record of its own, not the
/// tracker's entries; it is still a different request.
#[tokio::test(start_paused = true)]
async fn a_life_safety_operation_during_the_answer_draws_the_abort() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    rig.request(life_safety_operation(INVOKE)).await;
    rig.expect_abort(INVOKE).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A segment already on its way when the different request comes goes out
/// first, and the Abort after it: the sending task sends the Abort once it
/// stops, never the dispatch loop beside it. Until that Abort is handed to
/// the transport,
/// requests under the invoke ID are dropped, so none runs ahead of it and
/// has its answer meet that Abort; after it, the invoke ID is free.
#[tokio::test(start_paused = true)]
async fn the_abort_follows_a_segment_already_on_its_way() {
    let mut rig = Rig::start().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    rig.link.block_next_send();
    rig.ack(INVOKE, 0).await;
    // Recorded as the send starts, then held.
    assert!(rig.answer_segment(INVOKE, 1).await);
    rig.request(write(INVOKE, "not executed")).await;
    rig.request(write(INVOKE, "not executed")).await;
    rig.request(write(INVOKE, "a third")).await;
    rig.request(life_safety_operation(INVOKE)).await;
    rig.quiet(Duration::from_secs(1)).await;
    rig.link.release_sends(1);
    rig.expect_abort(INVOKE).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    assert_eq!(rig.value().await, long_value());
    rig.request(write(INVOKE, "a third")).await;
    rig.expect_simple_ack(INVOKE).await;
    assert_eq!(rig.value().await, "a third");
    rig.server.stop().await.unwrap();
}

/// Once every segment has gone out, with none on its way and only the
/// final Segment-ACK due, a different request takes that acknowledgment as
/// lost: no Abort, nothing more of the answer, and the request is served,
/// whole or in segments.
#[tokio::test(start_paused = true)]
async fn after_the_last_segment_a_different_request_is_served() {
    let mut rig = Rig::start().await;
    let total = rig.segments_in_answer().await;
    for (round, segmented) in [false, true].into_iter().enumerate() {
        // As long as the value read, so the next read's answer has as many
        // segments.
        let text = if segmented { "s" } else { "w" }.repeat(long_value().len());
        rig.request(read(INVOKE)).await;
        rig.take_and_ack(INVOKE, 0..total - 1).await;
        assert!(!rig.answer_segment(INVOKE, total - 1).await);
        if segmented {
            let segments = in_segments(&write(INVOKE, &text), 3);
            for (seq, segment) in segments.into_iter().enumerate() {
                rig.send(segment).await;
                rig.expect_ack(INVOKE, seq as u8, 1, "a segment of the new request")
                    .await;
            }
        } else {
            rig.request(write(INVOKE, &text)).await;
        }
        rig.expect_simple_ack(INVOKE).await;
        assert_eq!(rig.value().await, text, "round {round}");
        // Not the last segment again: the answer is over.
        rig.quiet(PAST_EVERY_RESEND).await;
    }
    rig.server.stop().await.unwrap();
}

/// The same request again after the last segment is still a duplicate: the
/// answer goes on, its last segment sent again until acknowledged.
#[tokio::test(start_paused = true)]
async fn after_the_last_segment_the_same_request_again_is_dropped() {
    let mut rig = Rig::start().await;
    let total = rig.segments_in_answer().await;
    rig.request(read(INVOKE)).await;
    rig.take_and_ack(INVOKE, 0..total - 1).await;
    assert!(!rig.answer_segment(INVOKE, total - 1).await);
    rig.request(read(INVOKE)).await;
    rig.quiet(BEFORE_THE_RESEND).await;
    assert!(!rig.answer_segment(INVOKE, total - 1).await);
    rig.ack(INVOKE, total - 1).await;
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A last segment still on its way, the first time or sent again, is not
/// yet out: a different request draws the Abort, after that segment.
#[tokio::test(start_paused = true)]
async fn a_last_segment_on_its_way_leaves_the_answer_midway() {
    for resent in [false, true] {
        let mut rig = Rig::start().await;
        let total = rig.segments_in_answer().await;
        rig.request(read(INVOKE)).await;
        rig.take_and_ack(INVOKE, 0..total - 2).await;
        assert!(rig.answer_segment(INVOKE, total - 2).await);
        if resent {
            rig.ack(INVOKE, total - 2).await;
            assert!(!rig.answer_segment(INVOKE, total - 1).await);
            rig.link.block_next_send();
            rig.quiet(BEFORE_THE_RESEND).await;
        } else {
            rig.link.block_next_send();
            rig.ack(INVOKE, total - 2).await;
        }
        // Recorded as its send starts, then held.
        assert!(!rig.answer_segment(INVOKE, total - 1).await);
        rig.request(write(INVOKE, "not executed")).await;
        rig.quiet(Duration::from_secs(1)).await;
        rig.link.release_sends(1);
        rig.expect_abort(INVOKE).await;
        rig.quiet(PAST_EVERY_RESEND).await;
        assert_eq!(rig.value().await, long_value(), "resent: {resent}");
        rig.server.stop().await.unwrap();
    }
}

/// A request in segments is judged when it is whole: one whose segment
/// zero came midway, completed after the last segment went out, is served.
#[tokio::test(start_paused = true)]
async fn a_request_in_segments_is_judged_when_whole() {
    let mut rig = Rig::start().await;
    let total = rig.segments_in_answer().await;
    rig.request(read(INVOKE)).await;
    assert!(rig.answer_segment(INVOKE, 0).await);
    let mut segments = in_segments(&write(INVOKE, "judged when whole"), 3).into_iter();
    rig.send(segments.next().unwrap()).await;
    rig.expect_ack(INVOKE, 0, 1, "segment zero, midway").await;
    rig.ack(INVOKE, 0).await;
    rig.take_and_ack(INVOKE, 1..total - 1).await;
    assert!(!rig.answer_segment(INVOKE, total - 1).await);
    for (seq, segment) in (1u8..).zip(segments) {
        rig.send(segment).await;
        rig.expect_ack(INVOKE, seq, 1, "the rest, after the last segment")
            .await;
    }
    rig.expect_simple_ack(INVOKE).await;
    assert_eq!(rig.value().await, "judged when whole");
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}

/// A LifeSafetyOperation after the last segment is served too.
#[tokio::test(start_paused = true)]
async fn after_the_last_segment_a_life_safety_operation_is_served() {
    let mut rig = Rig::start().await;
    let total = rig.segments_in_answer().await;
    rig.request(read(INVOKE)).await;
    rig.take_and_ack(INVOKE, 0..total - 1).await;
    assert!(!rig.answer_segment(INVOKE, total - 1).await);
    rig.request(life_safety_operation(INVOKE)).await;
    let reply = rig.reply_to_peer().await;
    assert!(
        matches!(reply, Apdu::Error(ref error) if error.invoke_id == INVOKE),
        "executed, as it would be in IDLE: {reply:?}"
    );
    rig.quiet(PAST_EVERY_RESEND).await;
    rig.server.stop().await.unwrap();
}
