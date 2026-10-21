//! Scenario (c): data requests, replies and Reply Postponed.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{AdapterSpec, Fault, MasterSpec, ReplyPolicy, Sim, SimConfig, WireFrame};

use common::*;

const ANSWER: [u8; 3] = [0x01, 0x00, 0xAB];

/// A two-master ring, 0 and 3, where station 3's application answers per
/// `policy`, and station 0 sends station 3 one request at 1 s.
fn requester_and_responder(policy: ReplyPolicy) -> Sim {
    requester_and_responder_with(SimConfig::new(BAUD), policy)
}

fn requester_and_responder_with(config: SimConfig, policy: ReplyPolicy) -> Sim {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 3, BAUD));
    sim.add_master(MasterSpec::new(3, 3, BAUD).application(policy));
    sim.enqueue(ms(1_000), 0, 3, &npdu(true, 7));
    sim
}

fn the_request(frames: &[WireFrame]) -> (usize, &WireFrame) {
    frames
        .iter()
        .enumerate()
        .find(|(_, frame)| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 3))
        .expect("station 0 sends the request")
}

/// Run until the request has gone out.
fn run_to_request(sim: &mut Sim) -> Instant {
    while !sim
        .frames()
        .iter()
        .any(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 3))
    {
        assert!(sim.now() < ms(3_000), "no request by 3 s");
        sim.run_for(millis(1));
    }
    the_request(sim.frames()).1.end
}

/// Clauses 9.5.6.2 (ReceivedDataNeedingReply), 9.5.6.9 (Reply) and 9.5.6.4
/// (ReceivedReply).
///
/// The responder's application answers within Treply_delay, so the reply is
/// the next frame on the wire; the requester takes it as its reply and goes
/// on to pass the token.
#[test]
fn a_request_is_answered_inside_treply_delay() {
    let mut sim = requester_and_responder(ReplyPolicy::Answer {
        delay: millis(5),
        payload: ANSWER.to_vec(),
    });
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    let (index, request) = the_request(frames);
    let reply = &frames[index + 1];
    assert!(
        reply.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 0),
        "{reply}"
    );
    assert_eq!(reply.data, ANSWER);
    assert_eq!(reply.start, request.end + millis(5));
    let pass = &frames[index + 2];
    assert!(pass.is(FrameType::TOKEN, 0, 3), "{pass}");
    assert_eq!(pass.start, reply.end + sim.timing().bits(40));
    assert!(sim.deliveries(3).iter().any(|delivery| delivery.request));
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.data == ANSWER));
    assert_eq!(sim.core(3).unwrap().counters().late_replies, 0);
    assert!(!sim.perturbed());
}

/// Clauses 9.5.6.9 (DeferredReply) and 9.5.6.4 (ReceivedPostpone).
///
/// The application is too slow, so the responder's core sends Reply
/// Postponed itself, its configured margin (25 ms) ahead of Treply_delay. The
/// requester passes the token at once; the answer goes out later as an
/// ordinary frame when the responder holds the token.
#[test]
fn a_slow_application_gets_reply_postponed_and_answers_with_the_token() {
    let mut sim = requester_and_responder(ReplyPolicy::Answer {
        delay: millis(240),
        payload: ANSWER.to_vec(),
    });
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    let (index, request) = the_request(frames);
    let postponed = &frames[index + 1];
    assert!(
        postponed.is(FrameType::REPLY_POSTPONED, 3, 0),
        "{postponed}"
    );
    assert_eq!(postponed.start, request.end + millis(225));
    let pass = &frames[index + 2];
    assert!(pass.is(FrameType::TOKEN, 0, 3), "{pass}");
    assert_eq!(pass.start, postponed.end + sim.timing().bits(40));

    let answer = first_after(frames, pass.end, |frame| {
        frame.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 0)
    })
    .expect("the answer follows with a later token");
    assert_eq!(answer.data, ANSWER);
    assert!(answer.start > request.end + millis(240));
    assert_eq!(sim.report().late_answers, 1);
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.data == ANSWER));
    sim.assert_no_violations();
}

/// Clause 9.5.6.9 (DeferredReply): an application may postpone on purpose.
#[test]
fn an_application_may_postpone_at_once() {
    let mut sim = requester_and_responder(ReplyPolicy::Postpone {
        delay: millis(10),
        payload: ANSWER.to_vec(),
    });
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    let (index, request) = the_request(frames);
    assert!(frames[index + 1].is(FrameType::REPLY_POSTPONED, 3, 0));
    assert_eq!(frames[index + 1].start, request.end + millis(10));
    assert!(first_after(frames, frames[index + 1].end, |frame| {
        frame.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 0) && frame.data == ANSWER
    })
    .is_some());
    assert_eq!(sim.report().late_answers, 0);
    assert!(!sim.perturbed());
}

/// Clause 9.5.6.4 (ReplyTimeout): with no answer, the requester waits
/// Treply_timeout of silence, then passes the token. That token use ends
/// there; its other queued frame waits for the next one.
#[test]
fn with_no_answer_the_requester_waits_treply_timeout() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 3, BAUD).max_info_frames(2));
    sim.add_master(MasterSpec::new(3, 3, BAUD));
    // Station 7 is not on the bus.
    sim.enqueue(ms(1_000), 0, 7, &npdu(true, 1));
    sim.enqueue(ms(1_000), 0, 3, &npdu(false, 2));
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 7))
        .expect("the request");
    let request = &frames[index];
    let pass = &frames[index + 1];
    assert!(pass.is(FrameType::TOKEN, 0, 3), "{pass}");
    assert_eq!(pass.start, request.end + millis(255));
    let data = first_after(frames, pass.end, |frame| {
        frame.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 3)
    })
    .expect("the other frame goes with the next token");
    let token_back = first_after(frames, pass.end, |frame| frame.is(FrameType::TOKEN, 3, 0))
        .expect("the token comes back");
    assert!(data.start > token_back.end);
    assert!(!sim.perturbed());
}

/// Clauses 9.5.6.9 and 9.5.3 (Treply_delay, Treply_timeout): the reply
/// window's edges as a slow adapter meets them.
///
/// A reply that starts after Treply_delay but before the requester can have
/// given up still goes out and is counted late. One that would start after
/// the requester's shortest Treply_timeout is withheld, so it cannot collide
/// with the requester passing the token on.
///
/// In the withheld case the requester's token pass arrives while the doomed
/// reply still waits in the adapter. The responder hears it (#1625): the
/// pass's first character, after the cutoff, drops the reply, and the Token
/// is taken up, so the requester never has to send it again.
#[test]
fn a_reply_the_adapter_starts_late_goes_out_until_the_requester_may_give_up() {
    let outcome = |start_delay: u64| {
        let config = SimConfig::new(BAUD);
        let mut sim = requester_and_responder_with(
            config,
            ReplyPolicy::Answer {
                delay: millis(220),
                payload: ANSWER.to_vec(),
            },
        );
        let request_end = run_to_request(&mut sim);
        // Slow only for the reply: the adapter takes its delay when the core
        // hands it the frame (at 220 ms), so the token passes after it are
        // punctual again.
        let slow = AdapterSpec {
            start_delay: Duration::from_millis(start_delay),
            ..AdapterSpec::default()
        };
        let now = sim.now();
        sim.inject(
            now,
            Fault::SetAdapter {
                station: 3,
                adapter: slow,
            },
        )
        .inject(
            request_end + millis(221),
            Fault::SetAdapter {
                station: 3,
                adapter: AdapterSpec::default(),
            },
        );
        sim.run_until(ms(2_500));
        (sim, request_end)
    };

    // Starts at 252 ms: after Treply_delay, before the cutoff (about 253.7 ms
    // at this baud rate).
    let (sim, request_end) = outcome(32);
    let frames = sim.frames();
    let reply = first_after(frames, request_end, |frame| frame.source() == Some(3))
        .expect("the late reply");
    assert!(
        reply.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 0),
        "{reply}"
    );
    assert_eq!(reply.start, request_end + millis(252));
    assert_eq!(sim.core(3).unwrap().counters().late_replies, 1);
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.data == ANSWER));
    sim.assert_no_violations();

    // Would start at 260 ms: withheld, and the requester times out at 255 ms.
    let (sim, request_end) = outcome(40);
    let frames = sim.frames();
    let next = first_after(frames, request_end, |_| true).expect("a frame after the request");
    assert!(next.is(FrameType::TOKEN, 0, 3), "{next}");
    assert_eq!(next.start, request_end + millis(255));
    // Taken back before the adapter got to it: no start was refused.
    assert_eq!(sim.report().refused_starts, 0);
    assert_eq!(sim.core(3).unwrap().counters().expired_requests, 1);
    assert_eq!(sim.report().collisions.len(), 0);
    let taken_up = first_after(frames, next.end, |_| true).expect("station 3 uses the token");
    assert_eq!(taken_up.source(), Some(3), "{taken_up}");
    assert_eq!(taken_up.start, next.end + sim.timing().bits(40));
    sim.assert_no_violations();
}
