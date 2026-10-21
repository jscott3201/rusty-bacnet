//! Extended frames (#1611): the extended frame types, 32 and 33, with
//! `MstpCoreConfig::extended_frames` on, the reserved COBS types, and the
//! buffers the flag needs (Clauses 9.3,
//! 9.3.9, 9.3.10, 9.5.4.9, 9.5.6). The flag-off path is the rest of the
//! suite, unchanged.

use bacnet_mstp_codec::crc::crc8;
use bacnet_mstp_codec::{
    decode_frame_in_place, Frame, FrameType, BROADCAST_MAC, MAX_COBS_FRAME_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH,
    MAX_STANDARD_MPDU_DATA,
};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, ActionBuffers, ConfigError, CoreBuffer, DequeuedNpdu, Direction, Duration,
    ExtendedActionBuffers, FixedQueues, Instant, MasterCore, MasterState, MstpCoreConfig,
    QueueCounters, QueueItemInfo, QueueKind, QueueRefusal, QueueStorage, ReplyDecision,
    RequestError, RequestId,
};

type Queues = FixedQueues<4, 4, MAX_EXTENDED_MPDU_DATA>;
type Actions = ExtendedActionBuffers<2>;
type Core<'a> = MasterCore<'a, Queues, MAX_EXTENDED_MPDU_DATA>;

const STATION: u8 = 1;
const PEER: u8 = 2;
/// Another station's address.
const OTHER: u8 = 9;

fn config(extended: bool) -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 3;
    if extended {
        config.with_extended_frames()
    } else {
        config
    }
}

/// Buffers sized for extended frames, boxed to keep them off the stack.
struct Storage {
    rx: Vec<u8>,
    actions: Box<Actions>,
}

impl Storage {
    fn new() -> Self {
        Self {
            rx: vec![0; MAX_EXTENDED_FRAME_LENGTH],
            actions: Box::default(),
        }
    }

    fn core(&mut self, extended: bool) -> Core<'_> {
        Core::from_parts(
            config(extended),
            Queues::new(),
            &mut self.rx,
            &mut self.actions,
            Instant::ZERO,
        )
        .unwrap()
    }
}

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let length = frame.encode_into(&mut wire).unwrap();
    wire.truncate(length);
    wire
}

/// A reserved COBS type's frame: a Type 33 frame with its type octet and
/// header CRC rewritten, since the encoded fields and their CRC-32K are the
/// same for every COBS type.
fn reserved(raw: u8, destination: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = encode(Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        destination,
        PEER,
        data,
    ));
    wire[2] = raw;
    wire[7] = crc8(&wire[2..7]);
    wire
}

/// An NPDU of `length` octets; `expecting` sets the control octet's
/// data-expecting-reply bit.
fn npdu(length: usize, expecting: bool) -> Vec<u8> {
    let mut npdu: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
    npdu[0] = 0x01;
    npdu[1] = if expecting { 0x04 } else { 0x00 };
    npdu
}

/// Feed `wire` octet by octet, 100 µs apart, after `start`; returns when the
/// last one arrived.
fn feed(core: &mut Core<'_>, wire: &[u8], start: Instant) -> Instant {
    let mut now = start;
    for &octet in wire {
        now += Duration::from_micros(100);
        core.on_octet(octet, now);
    }
    now
}

/// A frame's type, destination and decoded data.
fn decoded(octets: &[u8]) -> (FrameType, u8, Vec<u8>) {
    let mut copy = octets.to_vec();
    let (frame, used) = decode_frame_in_place(&mut copy).unwrap();
    assert_eq!(used, octets.len());
    (
        frame.header.frame_type,
        frame.header.destination,
        frame.data.to_vec(),
    )
}

/// Service the next deadline and send the frame it hands over; returns when
/// it was sent and its octets.
fn send_next(core: &mut Core<'_>) -> (Instant, Vec<u8>) {
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    send_handed_over(core, due)
}

/// Send the frame the core has handed over, starting at `due`.
fn send_handed_over(core: &mut Core<'_>, due: Instant) -> (Instant, Vec<u8>) {
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let (id, length) = match core.poll_action() {
        Action::Transmit { id, length, .. } => (id, length),
        other => panic!("expected a transmit, got {other:?}"),
    };
    let octets = core.on_transmit_started(id, due).unwrap().to_vec();
    assert_eq!(octets.len(), length);
    let done = due + Duration::from_millis(1);
    core.on_transmit_complete(id, done);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    (done, octets)
}

/// The next delivery, with its data copied out.
fn next_delivery(core: &mut Core<'_>) -> (u8, u8, Vec<u8>, Option<RequestId>) {
    match core.poll_action() {
        Action::DeliverNpdu {
            source,
            destination,
            npdu,
            request_id,
        } => (source, destination, npdu.to_vec(), request_id),
        other => panic!("expected an NPDU, got {other:?}"),
    }
}

/// Station 1 takes the token with `queued` waiting for the peer, sends it
/// and returns its octets and when it was sent.
fn take_token_and_send(core: &mut Core<'_>, queued: &[u8]) -> (Instant, Vec<u8>) {
    core.enqueue_npdu(PEER, queued, NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(
        Frame::new(FrameType::TOKEN, STATION, PEER, &[]),
        Instant::from_micros(10_000),
    );
    send_next(core)
}

/// Clause 9.3: the flag promises 1497-octet NPDUs, so every buffer must hold
/// an extended frame; the core names the first one that can't. Without the
/// flag the standard sizes are enough, as before.
#[test]
fn extended_frames_need_extended_buffers() {
    let refused = |error: Result<(), ConfigError>, buffer, length, minimum| {
        assert_eq!(
            error,
            Err(ConfigError::ExtendedStorageTooSmall {
                buffer,
                length,
                minimum,
            })
        );
    };
    let mut standard_rx = [0u8; MAX_STANDARD_FRAME_LENGTH];
    let mut extended_rx = vec![0u8; MAX_EXTENDED_FRAME_LENGTH];
    let mut extended_actions: Box<Actions> = Box::default();

    // Standard receive storage.
    let result = MasterCore::from_parts(
        config(true),
        Queues::new(),
        &mut standard_rx,
        &mut extended_actions,
        Instant::ZERO,
    );
    refused(
        result.map(drop),
        CoreBuffer::ReceiveStorage,
        MAX_STANDARD_FRAME_LENGTH,
        MAX_EXTENDED_FRAME_LENGTH,
    );
    // Standard action buffers: the frame buffer comes first.
    let mut standard_actions = ActionBuffers::<1>::new();
    let result = MasterCore::from_parts(
        config(true),
        FixedQueues::<4, 4, MAX_STANDARD_MPDU_DATA>::new(),
        &mut extended_rx,
        &mut standard_actions,
        Instant::ZERO,
    );
    refused(
        result.map(drop),
        CoreBuffer::TransmitFrame,
        MAX_STANDARD_FRAME_LENGTH,
        MAX_EXTENDED_FRAME_LENGTH,
    );
    // A frame buffer big enough, but standard delivery slots.
    let mut small_slots = Box::new(ActionBuffers::<
        1,
        MAX_STANDARD_MPDU_DATA,
        MAX_EXTENDED_FRAME_LENGTH,
    >::new());
    let result = MasterCore::from_parts(
        config(true),
        FixedQueues::<4, 4, MAX_STANDARD_MPDU_DATA>::new(),
        &mut extended_rx,
        &mut small_slots,
        Instant::ZERO,
    );
    refused(
        result.map(drop),
        CoreBuffer::DeliverySlot,
        MAX_STANDARD_MPDU_DATA,
        MAX_EXTENDED_MPDU_DATA,
    );
    // Everything but the queues.
    let result = MasterCore::from_parts(
        config(true),
        FixedQueues::<4, 4>::new(),
        &mut extended_rx,
        &mut extended_actions,
        Instant::ZERO,
    );
    refused(
        result.map(drop),
        CoreBuffer::QueueSlot,
        MAX_STANDARD_MPDU_DATA,
        MAX_EXTENDED_MPDU_DATA,
    );
    // All extended: accepted. All standard, flag off: accepted as before.
    assert!(MasterCore::from_parts(
        config(true),
        Queues::new(),
        &mut extended_rx,
        &mut extended_actions,
        Instant::ZERO,
    )
    .is_ok());
    assert!(MasterCore::from_parts(
        config(false),
        FixedQueues::<4, 4>::new(),
        &mut standard_rx,
        &mut standard_actions,
        Instant::ZERO,
    )
    .is_ok());
    assert_eq!(config(false).max_npdu(), MAX_STANDARD_MPDU_DATA);
    assert_eq!(config(true).max_npdu(), MAX_EXTENDED_MPDU_DATA);
}

/// Queues with extended slots behind a storage type that doesn't override
/// `QueueStorage::max_npdu`, so it reports the default.
struct Unsized(Queues);

impl QueueStorage for Unsized {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.0.set_starvation_limit(limit);
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        self.0
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: Instant) -> usize {
        self.0.expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.0.peek_kind(kind, now)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        self.0.dequeue_kind(kind, now, max_len)
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.0.depth(kind)
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.0.counters(kind)
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.0.take_counters(kind)
    }
}

/// Storage that doesn't say how long its slots are reports a standard
/// frame's 501 octets, so extended frames refuse it: the default fails
/// closed. The same queues saying 1497 are taken.
#[test]
fn queue_storage_that_does_not_say_is_refused_for_extended_frames() {
    let mut rx = vec![0u8; MAX_EXTENDED_FRAME_LENGTH];
    let mut actions: Box<Actions> = Box::default();
    let unsized_queues = Unsized(Queues::new());
    assert_eq!(unsized_queues.max_npdu(), MAX_STANDARD_MPDU_DATA);
    let result = MasterCore::from_parts(
        config(true),
        unsized_queues,
        &mut rx,
        &mut actions,
        Instant::ZERO,
    );
    assert_eq!(
        result.map(drop),
        Err(ConfigError::ExtendedStorageTooSmall {
            buffer: CoreBuffer::QueueSlot,
            length: MAX_STANDARD_MPDU_DATA,
            minimum: MAX_EXTENDED_MPDU_DATA,
        })
    );
    assert_eq!(Queues::new().max_npdu(), MAX_EXTENDED_MPDU_DATA);
}

/// Clauses 9.3.9, 9.3.10 and 9.5.6.3 (SendAndWait, SendNoWait): with the
/// flag on, an NPDU of 502 to 1497 octets goes out extended, Type 32 when it
/// expects a reply and Type 33 otherwise; a longer one is refused. Without
/// the flag anything over 501 octets is refused, as before.
#[test]
fn long_npdus_go_out_in_extended_frames() {
    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let request = npdu(MAX_EXTENDED_MPDU_DATA, true);
    let (_, octets) = take_token_and_send(&mut core, &request);
    assert_eq!(
        decoded(&octets),
        (
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            PEER,
            request
        )
    );
    assert_eq!(core.state(), MasterState::WaitForReply);
    assert_eq!(
        core.enqueue_npdu(
            PEER,
            &npdu(MAX_EXTENDED_MPDU_DATA + 1, false),
            NetworkPriority::Normal,
            Instant::from_micros(20_000),
        ),
        Err(QueueRefusal::Oversize)
    );
    assert_eq!(core.counters().unicast.drops_oversize, 1);

    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let data = npdu(MAX_STANDARD_MPDU_DATA + 1, false);
    let (_, octets) = take_token_and_send(&mut core, &data);
    assert_eq!(
        decoded(&octets),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            PEER,
            data
        )
    );
    // The token use goes on: no reply is awaited.
    assert_ne!(core.state(), MasterState::WaitForReply);

    // An NPDU a standard frame carries still goes in one.
    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let data = npdu(MAX_STANDARD_MPDU_DATA, true);
    let (_, octets) = take_token_and_send(&mut core, &data);
    assert_eq!(
        decoded(&octets),
        (FrameType::BACNET_DATA_EXPECTING_REPLY, PEER, data)
    );

    let mut storage = Storage::new();
    let mut core = storage.core(false);
    assert_eq!(
        core.enqueue_npdu(
            PEER,
            &npdu(MAX_STANDARD_MPDU_DATA + 1, false),
            NetworkPriority::Normal,
            Instant::ZERO,
        ),
        Err(QueueRefusal::Oversize)
    );
}

/// Clause 9.5.6.2 (ReceivedDataNoReply, BroadcastDataNeedingReply): in IDLE
/// a Type 33 for this station or broadcast goes up, and so does a broadcast
/// Type 32, with no request: nobody answers a broadcast. The station stays
/// idle. Both input paths agree.
#[test]
fn idle_takes_extended_data_that_needs_no_reply() {
    let long = npdu(900, false);
    let asking = npdu(700, true);
    let frames = [
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            STATION,
            &long,
        ),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            BROADCAST_MAC,
            &long,
        ),
        (
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            BROADCAST_MAC,
            &asking,
        ),
    ];
    for (frame_type, destination, data) in frames {
        for host in [false, true] {
            let mut storage = Storage::new();
            let mut core = storage.core(true);
            let frame = Frame::new(frame_type, destination, PEER, data);
            if host {
                core.on_validated_frame(frame, Instant::from_micros(1_000));
            } else {
                feed(&mut core, &encode(frame), Instant::ZERO);
            }
            assert_eq!(
                next_delivery(&mut core),
                (PEER, destination, data.clone(), None),
                "{frame_type:?} to {destination}, host {host}"
            );
            assert_eq!(core.poll_action(), Action::Idle);
            assert_eq!(core.state(), MasterState::Idle);
            let counters = core.counters();
            assert_eq!((counters.npdus_delivered, counters.parser_errors), (1, 0));
        }
    }
}

/// Clauses 9.5.6.2 (ReceivedDataNeedingReply) and 9.5.6.9 (Reply): a Type 32
/// for this station is a request. A long answer goes back as Type 33, a
/// short one as Type 6, both while the requester waits; one longer than an
/// extended frame carries is refused.
#[test]
fn a_type_32_request_is_answered() {
    for (answer_length, answer_type) in [
        (1200, FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY),
        (40, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY),
    ] {
        let mut storage = Storage::new();
        let mut core = storage.core(true);
        let request = npdu(1100, true);
        let last = feed(
            &mut core,
            &encode(Frame::new(
                FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                STATION,
                PEER,
                &request,
            )),
            Instant::ZERO,
        );
        assert_eq!(core.state(), MasterState::AnswerDataRequest);
        let (source, destination, data, id) = next_delivery(&mut core);
        assert_eq!((source, destination, data), (PEER, STATION, request));
        let id = id.expect("a request");
        let decided = last + Duration::from_millis(20);
        assert_eq!(
            core.complete_request(
                id,
                ReplyDecision::Data(&[0; MAX_EXTENDED_MPDU_DATA + 1]),
                decided
            ),
            Err(RequestError::Oversize {
                length: MAX_EXTENDED_MPDU_DATA + 1,
                maximum: MAX_EXTENDED_MPDU_DATA,
            })
        );
        let answer = npdu(answer_length, false);
        core.complete_request(id, ReplyDecision::Data(&answer), decided)
            .unwrap();
        let (sent, octets) = send_handed_over(&mut core, decided);
        assert_eq!(decoded(&octets), (answer_type, PEER, answer));
        assert!(sent < last + Duration::from_millis(250));
        assert_eq!(core.state(), MasterState::Idle);
        assert_eq!(core.counters().late_replies, 0);
    }
}

/// Clause 9.5.6.9 (DeferredReply): the application postpones a Type 32
/// request, so Reply Postponed goes back at once, and the answer, queued
/// like any NPDU, follows as Type 33 when this station next holds the token.
#[test]
fn a_postponed_type_32_request_is_answered_with_the_token() {
    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let last = feed(
        &mut core,
        &encode(Frame::new(
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            STATION,
            PEER,
            &npdu(800, true),
        )),
        Instant::ZERO,
    );
    let (.., id) = next_delivery(&mut core);
    let decided = last + Duration::from_millis(5);
    core.complete_request(id.unwrap(), ReplyDecision::Postponed, decided)
        .unwrap();
    let (sent, octets) = send_handed_over(&mut core, decided);
    assert_eq!(
        decoded(&octets),
        (FrameType::REPLY_POSTPONED, PEER, Vec::new())
    );
    let answer = npdu(1300, false);
    core.enqueue_npdu(PEER, &answer, NetworkPriority::Normal, sent)
        .unwrap();
    core.on_validated_frame(
        Frame::new(FrameType::TOKEN, STATION, PEER, &[]),
        sent + Duration::from_millis(30),
    );
    let (_, octets) = send_next(&mut core);
    assert_eq!(
        decoded(&octets),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            PEER,
            answer
        )
    );
}

/// Clause 9.5.6.9 and the core's handling of a frame that completes while
/// it answers (L05): a second Type 32 for this station is dropped and
/// counted in `discarded_while_busy`, since an answer after this station's
/// own would come too late, while a Type 33 and a broadcast Type 32 still go
/// up. The first request stays the one being answered.
#[test]
fn a_type_32_for_a_station_answering_a_request_is_discarded() {
    for host in [false, true] {
        let mut storage = Storage::new();
        let mut core = storage.core(true);
        let first = npdu(900, true);
        let last = feed(
            &mut core,
            &encode(Frame::new(
                FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                STATION,
                PEER,
                &first,
            )),
            Instant::ZERO,
        );
        let (.., id) = next_delivery(&mut core);
        assert_eq!(core.state(), MasterState::AnswerDataRequest);
        let second = npdu(700, true);
        let more = npdu(600, false);
        let frames = [
            (
                FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                STATION,
                &second,
            ),
            (
                FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
                STATION,
                &more,
            ),
            (
                FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                BROADCAST_MAC,
                &second,
            ),
        ];
        let mut at = last;
        for (frame_type, destination, data) in frames {
            let frame = Frame::new(frame_type, destination, OTHER, data);
            at += Duration::from_millis(1);
            if host {
                core.on_validated_frame(frame, at);
            } else {
                at = feed(&mut core, &encode(frame), at);
            }
        }
        assert_eq!(core.counters().discarded_while_busy, 1, "host {host}");
        assert_eq!(next_delivery(&mut core), (OTHER, STATION, more, None));
        assert_eq!(
            next_delivery(&mut core),
            (OTHER, BROADCAST_MAC, second.clone(), None)
        );
        assert_eq!(core.poll_action(), Action::Idle);
        assert_eq!(core.state(), MasterState::AnswerDataRequest);
        assert_eq!(core.snapshot().request_id, id);
    }
}

/// Clause 9.5.6.4 (ReceivedReply, ReceivedUnexpectedFrame): a Type 33 for
/// this station answers its request, from any station, and the token use
/// goes on; a Type 32 for it is no reply, and drops the token.
#[test]
fn wait_for_reply_takes_a_type_33() {
    for (frame_type, source, ends_as) in [
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            PEER,
            MasterState::PollForMaster,
        ),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            OTHER,
            MasterState::PollForMaster,
        ),
        (
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            PEER,
            MasterState::Idle,
        ),
    ] {
        for host in [false, true] {
            let mut storage = Storage::new();
            let mut core = storage.core(true);
            let (sent, _) = take_token_and_send(&mut core, &npdu(1000, true));
            assert_eq!(core.state(), MasterState::WaitForReply);
            let reply = npdu(1400, false);
            let frame = Frame::new(frame_type, STATION, source, &reply);
            if host {
                core.on_validated_frame(frame, sent + Duration::from_millis(30));
            } else {
                feed(&mut core, &encode(frame), sent);
            }
            // DONE_WITH_TOKEN polls for a successor here.
            assert_eq!(core.state(), ends_as, "{frame_type:?} from {source}");
            if ends_as == MasterState::PollForMaster {
                assert_eq!(next_delivery(&mut core), (source, STATION, reply, None));
            }
            assert_eq!(core.counters().parser_errors, 0);
        }
    }
}

/// Clause 9.5.4.9: with the flag on, a reserved COBS type is checked. A good
/// CRC-32K makes it a valid frame of a type the station doesn't know: IDLE
/// ignores it (ReceivedUnwantedFrame), and WAIT_FOR_REPLY takes it as
/// unexpected and drops the token. A bad CRC-32K makes it an invalid frame
/// for this station or broadcast, which ends WAIT_FOR_REPLY's token use;
/// another station's is skipped unjudged (Clause 9.5.4.4).
#[test]
fn reserved_cobs_types_are_checked_with_the_flag() {
    let data = [0x5A; 60];

    // IDLE, a good one: counted as received, nothing delivered.
    let mut storage = Storage::new();
    let mut core = storage.core(true);
    feed(&mut core, &reserved(0x22, STATION, &data), Instant::ZERO);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.state(), MasterState::Idle);
    let counters = core.counters();
    assert_eq!((counters.frames_received, counters.parser_errors), (1, 0));

    // WAIT_FOR_REPLY: (frame, invalid, state after).
    let mut damaged = reserved(0x7F, STATION, &data);
    damaged[20] ^= 0x10;
    let mut damaged_elsewhere = reserved(0x7F, OTHER, &data);
    damaged_elsewhere[20] ^= 0x10;
    for (wire, invalid, after) in [
        (reserved(0x22, STATION, &data), 0, MasterState::Idle),
        (damaged, 1, MasterState::PollForMaster),
        (damaged_elsewhere, 0, MasterState::WaitForReply),
    ] {
        let mut storage = Storage::new();
        let mut core = storage.core(true);
        let (sent, _) = take_token_and_send(&mut core, &npdu(10, true));
        feed(&mut core, &wire, sent);
        assert_eq!(core.state(), after);
        assert_eq!(core.counters().parser_errors, invalid);
    }

    // A host assembler's reserved frame is a valid one too.
    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let (sent, _) = take_token_and_send(&mut core, &npdu(10, true));
    let frame = Frame::new(FrameType::from_raw(0x40), STATION, PEER, &data);
    core.on_validated_frame(frame, sent + Duration::from_millis(5));
    assert_eq!(core.state(), MasterState::Idle);
}

/// Without the flag a host assembler's extended or reserved frame is
/// refused, as before; with it, Types 32 and 33 may carry 1497 octets and
/// no more.
#[test]
fn validated_extended_frames_follow_the_flag() {
    let long = [0x11; MAX_EXTENDED_MPDU_DATA + 1];
    let mut storage = Storage::new();
    let mut core = storage.core(false);
    let frame = Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        STATION,
        PEER,
        &long[..600],
    );
    core.on_validated_frame(frame, Instant::from_micros(1));
    let small = Frame::new(FrameType::from_raw(0x22), STATION, PEER, &long[..10]);
    core.on_validated_frame(small, Instant::from_micros(1));
    assert_eq!(core.counters().frames_received, 0);
    assert_eq!(core.counters().adapter_errors, 2);

    let mut storage = Storage::new();
    let mut core = storage.core(true);
    let frame = Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        STATION,
        PEER,
        &long,
    );
    core.on_validated_frame(frame, Instant::from_micros(1));
    assert_eq!(core.counters().frames_received, 0);
    assert_eq!(core.counters().adapter_errors, 1);
}
