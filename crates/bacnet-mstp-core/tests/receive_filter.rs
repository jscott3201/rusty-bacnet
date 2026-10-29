//! Which received frames end WAIT_FOR_REPLY, and which dropped candidates
//! count as invalid frames for this station (Clauses 9.5.4, 9.5.6.4, 9.5.8).

use bacnet_mstp_codec::crc::crc8;
use bacnet_mstp_codec::{
    BROADCAST_MAC, Frame, FrameType, MAX_COBS_FRAME_LENGTH, MAX_EXTENDED_FRAME_LENGTH,
    MAX_EXTENDED_MPDU_DATA, PREAMBLE,
};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Direction, Duration, FixedQueues, Instant, MasterCore, MasterState, MstpCoreConfig,
    ReceiveError,
};

type Queues = FixedQueues<4, 4>;
type Actions = bacnet_mstp_core::ActionBuffers;

const STATION: u8 = 1;
/// The station this one sends its request to.
const ASKED: u8 = 3;
/// Another station's address.
const OTHER: u8 = 9;

fn config() -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 3;
    config
}

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let length = frame.encode_into(&mut wire).unwrap();
    wire.truncate(length);
    wire
}

/// A header with a correct CRC, for field values encode refuses.
fn raw_header(frame_type: u8, destination: u8, source: u8, length: u16) -> Vec<u8> {
    let [high, low] = length.to_be_bytes();
    let fields = [frame_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&fields);
    wire.push(crc8(&fields));
    wire
}

/// Send a data-expecting-reply frame to `ASKED` and wait for its reply.
fn wait_for_reply(core: &mut MasterCore<'_, Queues>) -> Instant {
    core.enqueue_npdu(ASKED, &[1, 0x04], NetworkPriority::Normal, Instant::ZERO)
        .unwrap();
    core.on_validated_frame(Frame::new(FrameType::TOKEN, STATION, 0, &[]), Instant::ZERO);
    let due = core.next_deadline().unwrap();
    core.on_deadline(due);
    assert_eq!(
        core.poll_action(),
        Action::SetDirection(Direction::Transmit)
    );
    let Action::Transmit { id, .. } = core.poll_action() else {
        panic!("expected the request transmit");
    };
    core.on_transmit_started(id, due).unwrap();
    let done = due + Duration::from_millis(1);
    core.on_transmit_complete(id, done);
    assert_eq!(core.poll_action(), Action::SetDirection(Direction::Receive));
    assert_eq!(core.state(), MasterState::WaitForReply);
    done
}

/// Feed `wire` octet by octet, 100 µs apart, and return the last octet's time.
fn feed(core: &mut MasterCore<'_, Queues>, wire: &[u8], start: Instant) -> Instant {
    let mut now = start;
    for &octet in wire {
        now += Duration::from_micros(100);
        core.on_octet(octet, now);
    }
    now
}

/// While waiting for a reply, feed `wire` through receive storage of
/// `storage` octets and report whether it ended the wait as an invalid frame.
/// An invalid frame ends this token use, which here polls for a master
/// (Clause 9.5.6.4); a frame ignored as another station's leaves the wait
/// running.
fn counted_invalid(storage: usize, wire: &[u8]) -> bool {
    let mut rx = vec![0u8; storage];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let start = wait_for_reply(&mut core);
    feed(&mut core, wire, start);
    let counters = core.counters();
    assert_eq!(counters.frames_received, 1, "only the token was valid");
    match (core.state(), counters.parser_errors) {
        // Octets still arriving after the invalid header cancel the poll
        // while it waits out the turnaround, which leaves the core idle.
        (MasterState::PollForMaster | MasterState::Idle, 1) => true,
        (MasterState::WaitForReply, 0) => false,
        other => panic!("unexpected outcome {other:?}"),
    }
}

#[test]
fn a_reply_type_frame_from_any_station_ends_the_wait() {
    for (frame_type, source) in [
        (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 5),
        (FrameType::REPLY_POSTPONED, 4),
        (FrameType::TEST_RESPONSE, 6),
    ] {
        let mut rx = [0u8; 64];
        let mut actions = Actions::new();
        let mut core = MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        let now = wait_for_reply(&mut core);
        let data: &[u8] = if frame_type.allows_data() { &[7] } else { &[] };
        core.on_validated_frame(Frame::new(frame_type, STATION, source, data), now);
        // DONE_WITH_TOKEN, which polls for a successor here, not IDLE.
        assert_eq!(core.state(), MasterState::PollForMaster, "{frame_type:?}");
        if frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            assert_eq!(
                core.poll_action(),
                Action::DeliverNpdu {
                    source,
                    destination: STATION,
                    npdu: &[7],
                    request_id: None,
                }
            );
        }
    }

    // A reply addressed elsewhere is an unexpected frame: back to IDLE.
    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let now = wait_for_reply(&mut core);
    core.on_validated_frame(
        Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 2, ASKED, &[7]),
        now,
    );
    assert_eq!(core.state(), MasterState::Idle);
}

#[test]
fn validated_frame_the_codec_would_refuse_changes_nothing() {
    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let late = Instant::from_micros(10_000);
    let data = [0u8; 502];
    // Too long for its type, from station 255, and a COBS type without
    // extended frames: each one ignored and counted as the adapter's error.
    core.on_validated_frame(
        Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            STATION,
            5,
            &data,
        ),
        late,
    );
    assert_eq!(core.counters().adapter_errors, 1);
    core.on_validated_frame(Frame::new(FrameType::TOKEN, STATION, 255, &[]), late);
    assert_eq!(core.counters().adapter_errors, 2);
    core.on_validated_frame(
        Frame::new(
            FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            STATION,
            5,
            &[1],
        ),
        late,
    );
    assert_eq!(core.counters().adapter_errors, 3);
    assert_eq!(core.counters().frames_received, 0);
    assert_eq!(core.state(), MasterState::Idle);
    assert_eq!(core.poll_action(), Action::Idle);
    assert_eq!(core.next_deadline(), Some(Instant::from_micros(500_000)));
    // The refused frames did not move the core's clock either.
    core.on_silence(Instant::from_micros(1));
}

#[test]
fn a_frame_too_long_for_storage_counts_only_for_this_station() {
    // 40 data octets make a 50-octet frame; the storage holds 32.
    let frame = |destination| {
        encode(Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            destination,
            5,
            &[0; 40],
        ))
    };
    assert!(!counted_invalid(32, &frame(OTHER)));
    assert!(counted_invalid(32, &frame(STATION)));
    assert!(counted_invalid(32, &frame(BROADCAST_MAC)));
}

#[test]
fn a_reserved_cobs_frame_counts_only_for_this_station() {
    // Reserved COBS type 34 with a Length of 506, inside the COBS range: a
    // good header of a type the codec doesn't decode. The codec would skip
    // the rest of the frame by count; here it never comes.
    let header = |destination| raw_header(0x22, destination, 5, 506);
    assert!(!counted_invalid(600, &header(OTHER)));
    assert!(counted_invalid(600, &header(STATION)));
    assert!(counted_invalid(600, &header(BROADCAST_MAC)));
}

/// Without `extended_frames` the core takes no extended frame: the codec
/// decodes them, but the core drops one its storage holds as an unsupported
/// type, and one its storage can't hold as too long. Either way it counts
/// only when it is for this station or broadcast, as a node without COBS
/// support counts it. Both types, up to the largest NPDU.
#[test]
fn an_extended_frame_counts_only_for_this_station() {
    let npdus: [&[u8]; 2] = [&[0x11; 100], &[0x22; MAX_EXTENDED_MPDU_DATA]];
    for frame_type in [
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
    ] {
        for npdu in npdus {
            let frame = |destination| encode(Frame::new(frame_type, destination, 5, npdu));
            for storage in [64, MAX_EXTENDED_FRAME_LENGTH] {
                let case = format!("{frame_type:?}, {} octets, storage {storage}", npdu.len());
                assert!(!counted_invalid(storage, &frame(OTHER)), "{case}");
                assert!(counted_invalid(storage, &frame(STATION)), "{case}");
                assert!(counted_invalid(storage, &frame(BROADCAST_MAC)), "{case}");
            }
        }
    }
}

/// When the storage holds an extended frame the core counts it at its last
/// octet, once the codec has decoded it, as a receiver without COBS counts a
/// frame it can't take after its data and data CRC. When the storage can't
/// hold it, it counts at the header.
#[test]
fn an_extended_frame_for_this_station_counts_when_it_completes() {
    let wire = encode(Frame::new(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        STATION,
        5,
        &[0x22; MAX_EXTENDED_MPDU_DATA],
    ));
    for (storage, counted_at) in [(MAX_EXTENDED_FRAME_LENGTH, wire.len()), (64, 8)] {
        let mut rx = vec![0u8; storage];
        let mut actions = Actions::new();
        let mut core = MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        let start = wait_for_reply(&mut core);
        let before = feed(&mut core, &wire[..counted_at - 1], start);
        assert_eq!(core.counters().parser_errors, 0, "storage {storage}");
        assert_eq!(core.state(), MasterState::WaitForReply);
        feed(&mut core, &wire[counted_at - 1..counted_at], before);
        assert_eq!(core.counters().parser_errors, 1, "storage {storage}");
        assert_ne!(core.state(), MasterState::WaitForReply);
    }
}

/// While the codec skips another station's frame by count, its receive
/// phase is the data phase, so a silence longer than T_frame_abort or a
/// receive error in the middle of it counts one invalid frame, as the skip
/// state's Timeout and Error transitions do (Clause 9.5.4.7). In
/// WAIT_FOR_REPLY that ends the token use (Clause 9.5.6.4), which here
/// polls for a master.
#[test]
fn an_abort_while_skipping_another_stations_frame_counts_once() {
    // 40 data octets make a 50-octet frame; the storage holds 32, so the
    // codec reports it at its header, for another station, and skips it.
    let wire = encode(Frame::new(
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        OTHER,
        5,
        &[0; 40],
    ));
    for silence in [true, false] {
        let mut rx = vec![0u8; 32];
        let mut actions = Actions::new();
        let mut core = MasterCore::from_parts(
            config(),
            Queues::new(),
            &mut rx,
            &mut actions,
            Instant::ZERO,
        )
        .unwrap();
        let start = wait_for_reply(&mut core);
        let last = feed(&mut core, &wire[..20], start);
        assert_eq!(core.counters().parser_errors, 0);
        assert_eq!(core.state(), MasterState::WaitForReply);
        if silence {
            let abort = last + core.config().t_frame_abort + Duration::from_micros(1);
            core.on_silence(abort);
            assert_eq!(core.counters().frame_aborts, 1);
        } else {
            core.on_receive_error(ReceiveError::Framing, last + Duration::from_micros(100));
        }
        assert_eq!(core.counters().parser_errors, 1, "silence: {silence}");
        assert_eq!(
            core.state(),
            MasterState::PollForMaster,
            "silence: {silence}"
        );
    }
}

#[test]
fn a_bad_data_crc_counts_only_for_this_station() {
    let frame = |destination| {
        let mut wire = encode(Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            destination,
            5,
            &[1, 2, 3],
        ));
        wire[9] ^= 0x01;
        wire
    };
    assert!(!counted_invalid(600, &frame(OTHER)));
    assert!(counted_invalid(600, &frame(STATION)));
    assert!(counted_invalid(600, &frame(BROADCAST_MAC)));
}

#[test]
fn a_bad_header_counts_whatever_its_destination() {
    let mut bad_crc = encode(Frame::new(FrameType::TOKEN, OTHER, 5, &[]));
    bad_crc[7] ^= 0x01;
    assert!(counted_invalid(600, &bad_crc), "header CRC");
    // Source 255 (Clause 9.3).
    assert!(
        counted_invalid(600, &raw_header(0x06, OTHER, 255, 0)),
        "source"
    );
    // A Length over 501 on a type that isn't COBS-encoded.
    assert!(
        counted_invalid(600, &raw_header(0x06, OTHER, 5, 600)),
        "length"
    );
    // A COBS header with source 255, or a Length outside the COBS range
    // (Clause 9.5.8): the codec checks both before the type now, so neither
    // passes for a good header of a type it doesn't decode (#1611).
    assert!(
        counted_invalid(600, &raw_header(0x22, OTHER, 255, 506)),
        "COBS source"
    );
    for length in [4, 2044] {
        assert!(
            counted_invalid(600, &raw_header(0x20, OTHER, 5, length)),
            "COBS length {length}"
        );
    }
}

#[test]
fn the_decoder_is_drained_after_a_dropped_candidate() {
    // A header whose CRC fails, with a Token's first octets in its last
    // three. The codec drops the first octet and rescans the rest, finding
    // the Token's start.
    let fields = [0x06, OTHER, 5, 0x55, 0xFF];
    assert_ne!(crc8(&fields), 0x00);
    let mut bad = PREAMBLE.to_vec();
    bad.extend_from_slice(&fields);
    bad.push(0x00);
    let token = encode(Frame::new(FrameType::TOKEN, STATION, 2, &[]));
    assert_eq!(token[..3], bad[5..]);

    let mut rx = [0u8; 64];
    let mut actions = Actions::new();
    let mut core = MasterCore::from_parts(
        config(),
        Queues::new(),
        &mut rx,
        &mut actions,
        Instant::ZERO,
    )
    .unwrap();
    let last = feed(&mut core, &bad, Instant::ZERO);
    assert_eq!(core.counters().parser_errors, 1);
    // Draining at once leaves the Token's partial header current, so its
    // frame-abort deadline runs from this octet.
    let abort = core.config().t_frame_abort;
    assert_eq!(
        core.next_deadline(),
        Some(last + abort + Duration::from_micros(1))
    );
    feed(&mut core, &token[3..], last);
    assert_eq!(core.counters().frames_received, 1);
    // The Token was for this station; with nothing queued it polls for a
    // successor.
    assert_eq!(core.state(), MasterState::PollForMaster);
}
