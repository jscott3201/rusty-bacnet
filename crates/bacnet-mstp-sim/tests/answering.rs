//! A master answering a request: frames that arrive meanwhile, and requests
//! sent to everyone. A scripted station 5 plays a master that sends them.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::{MasterSpec, ReplyPolicy, ScriptSend, SeenFrame, Sim, SimConfig};

use common::*;

/// The scripted station's reaction delay: just over Tturnaround at 38400
/// baud.
const PAUSE: Duration = Duration::from_micros(1_100);
const ANSWER: [u8; 3] = [0x01, 0x00, 0xAB];

/// Scripted master 5: it answers polls, and the first time it holds the
/// token it sends `first`; every later token goes straight on to `next`.
fn tester(sim: &mut Sim, next: u8, first: Vec<ScriptSend>) {
    let mut first = Some(first);
    sim.add_scripted(5, move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
        if frame.destination != 5 {
            return;
        }
        match frame.frame_type {
            FrameType::POLL_FOR_MASTER => sends.push(ScriptSend::frame(
                PAUSE,
                FrameType::REPLY_TO_POLL_FOR_MASTER,
                frame.source,
                5,
                &[],
            )),
            FrameType::TOKEN => match first.take() {
                Some(first) => sends.extend(first),
                None => sends.push(ScriptSend::frame(PAUSE, FrameType::TOKEN, next, 5, &[])),
            },
            _ => {}
        }
    });
}

/// Clauses 9.5.6.9 and 9.5.6.2 (ReceivedToken).
///
/// A Token for a master that is busy answering a request waits until the
/// answer is out, and the master takes it up on the way back to IDLE. Here
/// station 5 sends master 3 a request, then hands it the token while its
/// application is still working; master 3 answers, then uses the token
/// straight away instead of letting it be lost.
#[test]
fn a_token_that_arrives_during_an_answer_is_used_after_it() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(
        MasterSpec::new(3, 7, BAUD).application(ReplyPolicy::Answer {
            delay: millis(50),
            payload: ANSWER.to_vec(),
        }),
    );
    tester(
        &mut sim,
        3,
        vec![
            ScriptSend::frame(
                PAUSE,
                FrameType::BACNET_DATA_EXPECTING_REPLY,
                3,
                5,
                &npdu(true, 1),
            ),
            ScriptSend::frame(millis(8), FrameType::TOKEN, 3, 5, &[]),
        ],
    );
    sim.run_until(ms(2_000));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 3))
        .expect("the request");
    assert!(frames[index + 1].is(FrameType::TOKEN, 5, 3));
    let answer = &frames[index + 2];
    assert!(
        answer.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 3, 5),
        "{answer}"
    );
    assert_eq!(answer.data, ANSWER);
    assert_eq!(answer.start, frames[index].end + millis(50));
    let next = &frames[index + 3];
    assert_eq!(next.source(), Some(3), "{next}");
    assert!(!next.generation);
    assert_eq!(next.start, answer.end + sim.timing().bits(40));
    sim.assert_no_violations();
}

/// Clause 9.5.6.2 (BroadcastDataNeedingReply): a data-expecting-reply frame
/// sent to everyone is passed up but answered by nobody, and no master
/// treats it as a request of its own.
#[test]
fn a_broadcast_request_is_delivered_and_left_unanswered() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 7, BAUD))
        .add_master(MasterSpec::new(3, 7, BAUD));
    tester(
        &mut sim,
        0,
        vec![
            ScriptSend::frame(
                PAUSE,
                FrameType::BACNET_DATA_EXPECTING_REPLY,
                0xFF,
                5,
                &npdu(true, 2),
            ),
            ScriptSend::frame(millis(300), FrameType::TOKEN, 0, 5, &[]),
        ],
    );
    sim.run_until(ms(2_500));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 0xFF))
        .expect("the broadcast request");
    let next = &frames[index + 1];
    assert!(next.is(FrameType::TOKEN, 5, 0), "{next}");
    for station in [0, 3] {
        let delivered: Vec<bool> = sim
            .deliveries(station)
            .iter()
            .filter(|delivery| delivery.data == npdu(true, 2))
            .map(|delivery| delivery.request)
            .collect();
        assert_eq!(delivered, [false], "station {station}");
    }
    sim.assert_no_violations();
}
