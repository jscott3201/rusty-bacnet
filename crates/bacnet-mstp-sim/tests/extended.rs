//! Extended frames (#1611): Types 32 and 33 between masters that take
//! them, and a ring where one master doesn't.

mod common;

use bacnet_mstp_codec::{FrameHeader, FrameType, BROADCAST_MAC, MAX_EXTENDED_MPDU_DATA};
use bacnet_mstp_core::Instant;
use bacnet_mstp_sim::{MasterSpec, ReplyPolicy, Sim, SimConfig, WireFrame};

use common::*;

/// An NPDU of `length` octets (network version 1); `expecting_reply` sets
/// the control octet's data-expecting-reply bit.
fn long_npdu(length: usize, expecting_reply: bool, tag: u8) -> Vec<u8> {
    let mut npdu: Vec<u8> = (0..length)
        .map(|index| (index as u8).wrapping_mul(7) ^ tag)
        .collect();
    npdu[0] = 0x01;
    npdu[1] = if expecting_reply { 0x04 } else { 0x00 };
    npdu
}

/// A master at `station` of masters 0 to 3, with extended frames or not.
fn master(station: u8, extended: bool) -> MasterSpec {
    let spec = MasterSpec::new(station, 3, BAUD);
    if extended {
        spec.extended_frames()
    } else {
        spec
    }
}

/// A two-master ring, 0 and 3, both taking extended frames, where station
/// 3's application answers per `policy`; station 0 sends station 3 one
/// 1000-octet request at 1 s.
fn extended_pair(policy: ReplyPolicy) -> (Sim, Vec<u8>) {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(master(0, true));
    sim.add_master(master(3, true).reply(policy));
    let request = long_npdu(1000, true, 0x31);
    sim.enqueue(ms(1_000), 0, 3, &request);
    (sim, request)
}

fn position(frames: &[WireFrame], predicate: impl Fn(&WireFrame) -> bool) -> usize {
    frames
        .iter()
        .position(predicate)
        .unwrap_or_else(|| panic!("no such frame in:\n{}", log(frames)))
}

/// Clauses 9.3.9, 9.3.10, 9.5.6.2 (ReceivedDataNeedingReply), 9.5.6.9
/// (Reply) and 9.5.6.4 (ReceivedReply): a Type 32 request is answered
/// inside Treply_delay with a Type 33, which the requester takes as its
/// reply before it passes the token on.
#[test]
fn a_type_32_request_is_answered_with_a_type_33() {
    let answer = long_npdu(1200, false, 0x5C);
    let (mut sim, request) = extended_pair(ReplyPolicy::Answer {
        delay: millis(5),
        payload: answer.clone(),
    });
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let index = position(frames, |frame| {
        frame.is(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 0, 3)
    });
    assert_eq!(frames[index].data, request);
    let reply = &frames[index + 1];
    assert!(
        reply.is(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, 3, 0),
        "{reply}"
    );
    assert_eq!(reply.data, answer);
    assert_eq!(reply.start, frames[index].end + millis(5));
    let pass = &frames[index + 2];
    assert!(pass.is(FrameType::TOKEN, 0, 3), "{pass}");
    assert_eq!(pass.start, reply.end + sim.timing().bits(40));
    assert!(sim
        .deliveries(3)
        .iter()
        .any(|delivery| delivery.request && delivery.data == request));
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.data == answer));
    assert!(!sim.perturbed());
    sim.assert_no_violations();
}

/// Clauses 9.5.6.9 (DeferredReply) and 9.5.6.4 (ReceivedPostpone): the
/// application postpones the Type 32 request, so Reply Postponed goes back
/// and the requester passes the token; the answer follows as a Type 33 when
/// the responder next holds the token.
#[test]
fn a_postponed_type_32_request_is_answered_later_with_a_type_33() {
    let answer = long_npdu(MAX_EXTENDED_MPDU_DATA, false, 0x22);
    let (mut sim, _) = extended_pair(ReplyPolicy::Postpone {
        delay: millis(10),
        payload: answer.clone(),
    });
    sim.run_until(ms(4_000));
    let frames = sim.frames();
    let index = position(frames, |frame| {
        frame.is(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 0, 3)
    });
    let postponed = &frames[index + 1];
    assert!(
        postponed.is(FrameType::REPLY_POSTPONED, 3, 0),
        "{postponed}"
    );
    assert_eq!(postponed.start, frames[index].end + millis(10));
    assert!(frames[index + 2].is(FrameType::TOKEN, 0, 3));
    let late = first_after(frames, postponed.end, |frame| {
        frame.is(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, 3, 0)
    })
    .expect("the answer follows with a later token");
    assert_eq!(late.data, answer);
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.data == answer));
    assert!(!sim.perturbed());
    sim.assert_no_violations();
}

/// Clause 9.5.6.3 (SendNoWait) and 9.5.6.2 (ReceivedDataNoReply): a long
/// broadcast goes out as one Type 33, and every master that takes extended
/// frames delivers it.
#[test]
fn a_broadcast_type_33_reaches_every_capable_master() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    for station in [0, 3, 5] {
        sim.add_master(MasterSpec::new(station, 7, BAUD).extended_frames());
    }
    let broadcast = long_npdu(800, false, 0x7E);
    sim.enqueue(ms(1_000), 3, BROADCAST_MAC, &broadcast);
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let index = position(frames, |frame| {
        frame.is(
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            3,
            BROADCAST_MAC,
        )
    });
    assert_eq!(frames[index].data, broadcast);
    for station in [0, 5] {
        assert!(
            sim.deliveries(station)
                .iter()
                .any(|delivery| delivery.destination == BROADCAST_MAC
                    && delivery.data == broadcast
                    && !delivery.request),
            "station {station}"
        );
    }
    assert!(!sim.perturbed());
    sim.assert_no_violations();
}

/// Rounds of traffic in [`mixed_ring`].
const ROUNDS: usize = 3;

/// A ring of masters 0, 3 and 5 at 9600 baud, where station 5 takes
/// extended frames only when `capable`. In each of three rounds, 3 s apart,
/// stations 0 and 3 send extended frames that need no reply (to station 5,
/// broadcast, and to each other) and a standard one to station 5, while the
/// ring rotates. Returns the run, through 12 s. At 9600 baud the longest of
/// them, 1200 octets, takes 1.3 s on the wire.
fn mixed_ring(capable: bool) -> Sim {
    let mut sim = Sim::new(SimConfig::new(SLOW_BAUD));
    sim.add_master(MasterSpec::new(0, 7, SLOW_BAUD).extended_frames());
    sim.add_master(MasterSpec::new(3, 7, SLOW_BAUD).extended_frames());
    let five = MasterSpec::new(5, 7, SLOW_BAUD);
    sim.add_master(if capable {
        five.extended_frames()
    } else {
        five
    });
    for round in 0..ROUNDS {
        let at = 1_000 + 3_000 * round as u64;
        let tag = round as u8;
        sim.enqueue(ms(at), 0, 5, &long_npdu(600 + 50 * round, false, tag));
        sim.enqueue(ms(at), 3, BROADCAST_MAC, &long_npdu(900, false, tag));
        sim.enqueue(ms(at + 100), 3, 0, &long_npdu(1200, false, tag));
        // Standard frames keep flowing to station 5.
        sim.enqueue(ms(at + 200), 0, 5, &npdu(false, tag));
    }
    sim.run_until(ms(12_000));
    sim
}

/// One frame on the wire: its header, if it decodes, and when it ran.
type Moment = (Option<FrameHeader>, Instant, Instant);

/// The wire log's frames as headers and times.
fn timeline(frames: &[WireFrame]) -> Vec<Moment> {
    frames
        .iter()
        .map(|frame| (frame.header, frame.start, frame.end))
        .collect()
}

/// Clauses 9.3 and 9.5.4.4: a master without COBS support counts an
/// extended frame for it or broadcast as an invalid frame, and skips one
/// for another station, and that is all. The ring around it runs exactly as
/// when it takes extended frames: the same frames at the same moments, so
/// no lost token, no token generated again and no extra poll, while the
/// standard frames sent to it still arrive.
#[test]
fn a_master_without_extended_frames_counts_them_invalid_and_the_ring_is_unchanged() {
    let without = mixed_ring(false);
    let with = mixed_ring(true);
    without.assert_no_violations();
    with.assert_no_violations();
    assert!(!without.perturbed() && !with.perturbed());
    assert_eq!(
        timeline(without.frames()),
        timeline(with.frames()),
        "the wire differs:\n{}",
        log(without.frames())
    );

    let extended_for_five = without
        .frames()
        .iter()
        .filter(|frame| {
            frame.frame_type() == Some(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY)
                && matches!(frame.destination(), Some(5 | BROADCAST_MAC))
        })
        .count();
    // Every round went out whole: two for station 5 and one for station 0.
    assert_eq!(extended_for_five, 2 * ROUNDS, "{}", log(without.frames()));
    let to_zero = sent_by(
        without.frames(),
        3,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
    )
    .iter()
    .filter(|frame| frame.destination() == Some(0))
    .count();
    assert_eq!(to_zero, ROUNDS);
    let counters = without.core(5).expect("station 5 is powered").counters();
    assert_eq!(counters.parser_errors, extended_for_five as u32);
    assert_eq!(counters.token_regenerations, 0);
    // Only the standard frames reach station 5's application.
    assert!(without
        .deliveries(5)
        .iter()
        .all(|delivery| delivery.data.len() == 3));
    assert_eq!(without.deliveries(5).len(), ROUNDS);
    assert_eq!(with.core(5).unwrap().counters().parser_errors, 0);
    assert_eq!(with.deliveries(5).len(), ROUNDS + extended_for_five);
    assert_eq!(
        without.core(0).unwrap().counters().token_regenerations
            + without.core(3).unwrap().counters().token_regenerations,
        with.core(0).unwrap().counters().token_regenerations
            + with.core(3).unwrap().counters().token_regenerations,
    );
}

/// Clause 9.5.6.4 (ReplyTimeout): a Type 32 request to a master without
/// COBS support goes unanswered, since that master counts it as an invalid
/// frame, so the requester waits Treply_timeout and passes the token on.
#[test]
fn a_type_32_request_to_a_master_without_extended_frames_times_out() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(master(0, true));
    sim.add_master(master(3, false));
    sim.enqueue(ms(1_000), 0, 3, &long_npdu(700, true, 1));
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let index = position(frames, |frame| {
        frame.is(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 0, 3)
    });
    let pass = &frames[index + 1];
    assert!(pass.is(FrameType::TOKEN, 0, 3), "{pass}");
    assert_eq!(pass.start, frames[index].end + millis(255));
    assert_eq!(sim.core(3).unwrap().counters().parser_errors, 1);
    assert!(sim.deliveries(3).is_empty());
    assert!(!sim.perturbed());
    sim.assert_no_violations();
}
