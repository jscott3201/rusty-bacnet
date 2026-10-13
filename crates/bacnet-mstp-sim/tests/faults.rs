//! Scenario (f): noise and damaged frames. The ring survives them, and an
//! invalid frame and a valid but unexpected one each take their own
//! transition.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::analysis::token_users;
use bacnet_mstp_sim::{
    CollisionClass, Corruption, Fault, MasterSpec, ScriptSend, SeenFrame, Sim, SimConfig, WireFrame,
};

use common::*;

/// Data octets of the scripted replies, and where their data CRC starts.
const REPLY: [u8; 3] = [0x01, 0x00, 0x09];
const REPLY_DATA_CRC: usize = 8 + REPLY.len();
/// Octet 7 is the header CRC.
const HEADER_CRC: usize = 7;

/// Masters 0 and 3, and a scripted slave at 40 that answers any data
/// request with `reply(requester)`. Station 0 sends it a request at 1 s.
fn with_slave(reply: impl Fn(u8) -> ScriptSend + 'static) -> Sim {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 3, BAUD))
        .add_master(MasterSpec::new(3, 3, BAUD))
        .add_scripted(40, move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
            if frame.destination == 40 && frame.frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY
            {
                sends.push(reply(frame.source));
            }
        })
        .enqueue(ms(1_000), 0, 40, &npdu(true, 1));
    sim
}

fn request_and_next(frames: &[WireFrame]) -> (&WireFrame, &WireFrame, &WireFrame) {
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 40))
        .expect("the request");
    (&frames[index], &frames[index + 1], &frames[index + 2])
}

/// Clauses 9.5.4 and 9.5.6.4 (InvalidFrame): a reply whose data CRC fails
/// is an invalid frame, so the requester stops waiting and passes the token
/// Tturnaround after it, instead of after Treply_timeout.
#[test]
fn an_invalid_reply_ends_the_wait_at_once() {
    let mut sim = with_slave(|to| {
        ScriptSend::frame(
            millis(2),
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            to,
            40,
            &REPLY,
        )
        .corrupted(REPLY_DATA_CRC, 0x01)
    });
    sim.run_until(ms(2_000));
    let (request, reply, next) = request_and_next(sim.frames());
    assert_eq!(reply.driver, bacnet_mstp_sim::Driver::Station(40));
    assert!(next.is(FrameType::TOKEN, 0, 3), "{next}");
    assert_eq!(next.start, reply.end + sim.timing().bits(40));
    assert!(next.start < request.end + millis(255));
    assert_eq!(sim.core(0).unwrap().counters().parser_errors, 1);
    sim.assert_no_violations();
}

/// Clauses 9.5.4, 9.5.6.4 (InvalidFrame) and 9.5.5.1 (#1618 item 2).
///
/// A reply whose header CRC fails is invalid as soon as the header ends, so
/// the requester goes on to pass the token then. The rest of the damaged
/// frame arrives while that Token waits out its turnaround, and only delays
/// it: the Token goes out Tturnaround after the frame's last octet, and the
/// token is not lost.
#[test]
fn an_invalid_reply_header_still_passes_the_token() {
    let mut sim = with_slave(|to| {
        ScriptSend::frame(
            millis(2),
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            to,
            40,
            &REPLY,
        )
        .corrupted(HEADER_CRC, 0x01)
    });
    sim.run_until(ms(2_500));
    let (_, reply, next) = request_and_next(sim.frames());
    assert!(next.is(FrameType::TOKEN, 0, 3), "{next}");
    assert_eq!(next.start, reply.end + sim.timing().bits(40));
    let counters = sim.core(0).unwrap().counters();
    assert_eq!(counters.parser_errors, 1);
    assert_eq!(counters.cancelled_sends, 0);
    // Only the cold start's: the token was never lost.
    assert_eq!(counters.token_regenerations, 1);
    sim.assert_no_violations();
}

/// Clause 9.5.4.4: a frame for another station is skipped without its data
/// being checked, so its bad data CRC is not an invalid frame here and the
/// requester waits out Treply_timeout from the end of that frame.
#[test]
fn a_bad_data_crc_on_a_frame_for_someone_else_is_not_an_invalid_reply() {
    let mut sim = with_slave(|_| {
        ScriptSend::frame(
            millis(2),
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            3,
            40,
            &REPLY,
        )
        .corrupted(REPLY_DATA_CRC, 0x01)
    });
    sim.run_until(ms(2_000));
    let (_, reply, next) = request_and_next(sim.frames());
    assert!(next.is(FrameType::TOKEN, 0, 3), "{next}");
    assert_eq!(next.start, reply.end + millis(255));
    assert_eq!(sim.core(0).unwrap().counters().parser_errors, 0);
    sim.assert_no_violations();
}

/// Clause 9.5.6.4 (ReceivedUnexpectedFrame): a valid frame that is not a
/// reply to this station may mean a second token, so the requester drops
/// its own and goes to IDLE. The line goes quiet and the token is
/// regenerated after Tno_token.
#[test]
fn an_unexpected_frame_while_waiting_drops_the_token() {
    let mut sim = with_slave(|_| {
        ScriptSend::frame(
            millis(2),
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            0xFF,
            40,
            &REPLY,
        )
    });
    sim.run_until(ms(2_500));
    let (_, reply, next) = request_and_next(sim.frames());
    assert!(reply.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 40, 0xFF));
    assert!(next.is(FrameType::POLL_FOR_MASTER, 0, 1), "{next}");
    assert!(next.generation);
    assert_eq!(next.start, reply.end + millis(500));
    sim.assert_no_violations();
}

/// A lone master 0 and a scripted station 2 that answers Poll For Master
/// with `answer(poller)`.
fn with_poll_answer(answer: impl Fn(u8) -> ScriptSend + 'static) -> Sim {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 7, BAUD)).add_scripted(
        2,
        move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
            if frame.destination == 2 && frame.frame_type == FrameType::POLL_FOR_MASTER {
                sends.push(answer(frame.source));
            }
        },
    );
    sim
}

/// Clause 9.5.6.8 (SendNextPFM after ReceivedInvalidFrame): a damaged answer
/// to a poll moves the poller on to the next address at once, rather than
/// after Tusage_timeout.
#[test]
fn an_invalid_answer_to_a_poll_moves_on_at_once() {
    let mut sim = with_poll_answer(|to| {
        ScriptSend::frame(millis(1), FrameType::REPLY_TO_POLL_FOR_MASTER, to, 2, &[])
            .corrupted(HEADER_CRC, 0x01)
    });
    sim.run_until(ms(700));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::POLL_FOR_MASTER, 0, 2))
        .expect("the poll of 2");
    let answer = &frames[index + 1];
    let next = &frames[index + 2];
    assert_eq!(answer.driver, bacnet_mstp_sim::Driver::Station(2));
    assert!(next.is(FrameType::POLL_FOR_MASTER, 0, 3), "{next}");
    assert_eq!(next.start, answer.end + sim.timing().bits(40));
    sim.assert_no_violations();
}

/// Clause 9.5.6.8 (ReceivedUnexpectedFrame): a poll answered with a valid
/// frame addressed elsewhere costs the poller its token.
#[test]
fn an_unexpected_answer_to_a_poll_drops_the_token() {
    let mut sim = with_poll_answer(|_| {
        ScriptSend::frame(millis(1), FrameType::REPLY_TO_POLL_FOR_MASTER, 5, 2, &[])
    });
    sim.run_until(ms(1_500));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::POLL_FOR_MASTER, 0, 2))
        .expect("the poll of 2");
    let answer = &frames[index + 1];
    let next = &frames[index + 2];
    assert!(next.is(FrameType::POLL_FOR_MASTER, 0, 1), "{next}");
    assert!(next.generation);
    assert_eq!(next.start, answer.end + millis(500));
    sim.assert_no_violations();
}

/// Run until station 3's next Token to 10 has been sent damaged.
fn run_to_damaged_token(sim: &mut Sim, at: Instant) -> WireFrame {
    sim.inject(
        at,
        Fault::CorruptNext {
            station: 3,
            frame_type: Some(FrameType::TOKEN),
            offset: 3,
            corruption: Corruption::Xor(0x01),
        },
    );
    sim.run_until(at);
    loop {
        if let Some(frame) = sim
            .frames()
            .iter()
            .find(|frame| frame.start >= at && frame.corrupted)
        {
            return frame.clone();
        }
        assert!(sim.now() < at + millis(500), "no damaged token");
        let step = sim.timing().bits(1);
        sim.run_for(step);
    }
}

/// Clause 9.5.6.6 (RetrySendToken): a Token damaged on the wire reaches
/// nobody, the passer hears nothing for Tusage_timeout, and sends it again.
#[test]
fn a_damaged_token_is_sent_again() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    let damaged = run_to_damaged_token(&mut sim, ms(2_000));
    sim.run_until(ms(3_000));
    let again = first_after(sim.frames(), damaged.end, |_| true).expect("a frame after");
    assert!(again.is(FrameType::TOKEN, 3, 10), "{again}");
    assert_eq!(again.start, damaged.end + millis(20));
    assert_eq!(sim.core(10).unwrap().counters().parser_errors, 1);
    sim.assert_no_violations();
}

/// Clause 9.5.6.6 (SawTokenUser): the passer counts any octets or errors as
/// its successor using the token, and lets go once it has more than
/// Nmin_octets (4). After a damaged Token, five octets of noise are enough
/// to lose the token until it is regenerated; four are not, and the passer
/// sends the Token again Tusage_timeout after the noise.
#[test]
fn noise_after_a_damaged_token_counts_as_the_successor_from_five_octets() {
    for octets in [4usize, 5] {
        let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
        let damaged = run_to_damaged_token(&mut sim, ms(2_000));
        let noise_at = damaged.end + millis(2);
        sim.inject(
            noise_at,
            Fault::Noise {
                octets: vec![0xA5; octets],
            },
        );
        sim.run_until(ms(3_500));
        let frames = sim.frames();
        let noise_end = noise_at + sim.timing().boundary(octets);
        let next = first_after(frames, noise_end, |frame| {
            matches!(frame.driver, bacnet_mstp_sim::Driver::Station(_))
        })
        .expect("a frame after the noise");
        if octets == 4 {
            assert!(next.is(FrameType::TOKEN, 3, 10), "{next}");
            assert_eq!(next.start, noise_end + millis(20));
        } else {
            assert!(next.is(FrameType::POLL_FOR_MASTER, 0, 1), "{next}");
            assert!(next.generation);
            assert_eq!(next.start, noise_end + millis(500));
        }
        sim.assert_no_violations();
    }
}

/// Clause 9.5.5.1 has a frame wait until the line has been quiet for
/// Tturnaround, and activity during that wait only delays it (#1618 item
/// 2). One noise octet in station 3's turnaround after it receives the
/// token pushes its frame back to Tturnaround after the noise. Nothing is
/// dropped, and station 0 needs no retry.
#[test]
fn noise_in_the_turnaround_only_delays_a_pending_frame() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    let token = run_until_frame(&mut sim, ms(2_000), ms(3_000), |frame| {
        frame.is(FrameType::TOKEN, 0, 3)
    });
    let noise_at = token + Duration::from_micros(300);
    sim.inject(noise_at, Fault::Noise { octets: vec![0x00] });
    sim.run_until(ms(3_000));
    let noise_end = noise_at + sim.timing().octet();
    let next = first_after(sim.frames(), noise_end, |_| true).expect("a frame after");
    // Station 3's turn for a maintenance poll, which goes out late but whole.
    assert!(next.is(FrameType::POLL_FOR_MASTER, 3, 7), "{next}");
    assert_eq!(next.start, noise_end + sim.timing().bits(40));
    assert_eq!(sim.core(3).unwrap().counters().cancelled_sends, 0);
    sim.assert_no_violations();
}

/// Clauses 9.5.4 and 9.5.6: random noise bursts and damaged octets over
/// several seconds of traffic. Receivers notice them, nothing collides
/// except with the noise, and once the faults stop every master is using
/// the token again.
#[test]
fn noise_and_damaged_frames_do_not_break_the_ring() {
    let stations = [0u8, 3, 10, 30];
    for seed in 0..6 {
        let mut config = SimConfig::new(BAUD);
        config.seed = seed;
        let mut sim = Sim::new(config);
        for station in stations {
            sim.add_master(MasterSpec::new(station, 31, BAUD).max_info_frames(2));
        }
        for step in 0..120u64 {
            let from = stations[(step % 4) as usize];
            let to = stations[((step + 1) % 4) as usize];
            sim.enqueue(
                ms(1_000 + step * 40),
                from,
                to,
                &npdu(step % 4 == 0, step as u8),
            );
        }
        let mut at = 1_200;
        while at < 6_000 {
            if sim.rng().chance(60) {
                let length = sim.rng().range(1, 12) as usize;
                let octets = sim.rng().bytes(length);
                sim.inject(ms(at), Fault::Noise { octets });
            } else {
                let station = stations[sim.rng().below(4) as usize];
                let offset = sim.rng().range(0, 9) as usize;
                sim.inject(
                    ms(at),
                    Fault::CorruptNext {
                        station,
                        frame_type: None,
                        offset,
                        corruption: Corruption::Xor(0x10),
                    },
                );
            }
            at += sim.rng().range(30, 250);
        }
        sim.run_until(ms(9_000));
        let report = sim.report();
        assert_eq!(
            report.collisions.len(),
            report.collisions_of(CollisionClass::Noise),
            "seed {seed}"
        );
        let errors: u32 = stations
            .iter()
            .map(|&station| sim.core(station).unwrap().counters().parser_errors)
            .sum();
        assert!(errors > 0, "seed {seed}");
        assert_eq!(
            token_users(sim.frames(), ms(7_000))
                .into_iter()
                .collect::<Vec<u8>>(),
            stations,
            "seed {seed}"
        );
        sim.assert_no_violations();
    }
}
