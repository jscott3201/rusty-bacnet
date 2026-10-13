//! Scenario (d): masters joining and leaving a running ring.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_sim::analysis::{token_passes, token_users};
use bacnet_mstp_sim::{Fault, MasterSpec, Sim, SimConfig};

use common::*;

/// Clauses 9.5.6.5 (SendMaintenancePFM), 9.5.6.8 (ReceivedReplyToPFM) and
/// 9.5.6.5 again (NextStationUnknown, at the newcomer).
///
/// A master powered up inside a running ring stays quiet in IDLE until the
/// master below it polls its address during maintenance. It answers, takes
/// the token, finds its own successor by polling upwards, and the ring now
/// runs through it.
#[test]
fn a_new_master_is_found_by_maintenance_polling() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    for station in [0, 3, 10] {
        sim.add_master(MasterSpec::new(station, 15, BAUD));
    }
    sim.add_master(MasterSpec::new(6, 15, BAUD).unpowered());
    sim.inject(ms(1_500), Fault::PowerOn { station: 6 });
    sim.run_until(ms(6_000));
    let frames = sim.frames();

    let reply = first_after(frames, ms(1_500), |frame| {
        frame.is(FrameType::REPLY_TO_POLL_FOR_MASTER, 6, 3)
    })
    .expect("station 6 answers station 3's maintenance poll");
    // Station 3 walked 4 and 5 first, one per token use.
    let polls: Vec<u8> = sent_by(frames, 3, FrameType::POLL_FOR_MASTER)
        .into_iter()
        .filter(|poll| poll.start > ms(1_500) && poll.end <= reply.start)
        .filter_map(|poll| poll.destination())
        .collect();
    assert!(polls.ends_with(&[4, 5, 6]), "{polls:?}");
    // Station 3 reaches address 6 within Npoll token uses plus the polls of
    // 4 and 5, well inside 2 s of the power-up.
    assert!(reply.start < ms(3_500));

    let passes: Vec<(u8, u8)> = token_passes(frames)
        .into_iter()
        .filter(|&(at, ..)| at > reply.end)
        .map(|(_, from, to)| (from, to))
        .collect();
    assert_eq!(passes[0], (3, 6));
    // Station 6 starts with no successor, polls 7..=10, and hands on to 10.
    let sweep: Vec<u8> = sent_by(frames, 6, FrameType::POLL_FOR_MASTER)
        .into_iter()
        .take(4)
        .filter_map(|poll| poll.destination())
        .collect();
    assert_eq!(sweep, [7, 8, 9, 10]);
    assert_eq!(passes[1], (6, 10));
    assert_eq!(&passes[2..6], &[(10, 0), (0, 3), (3, 6), (6, 10)]);
    sim.assert_no_violations();
}

/// Clause 9.5.6.6 (RetrySendToken, then FindNewSuccessor); Nretry_token is
/// one (Clause 9.5.3).
///
/// When the successor has died, nothing follows the token. After
/// Tusage_timeout the passer sends it once more; after another, it gives the
/// successor up and polls from the address after it until someone answers.
#[test]
fn a_dead_successor_gets_one_retry_then_is_replaced() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.inject(ms(2_000), Fault::PowerOff { station: 10 });
    sim.run_until(ms(4_000));
    let frames = sim.frames();
    let usage_timeout = millis(20);

    let index = frames
        .iter()
        .position(|frame| frame.start > ms(2_000) && frame.is(FrameType::TOKEN, 3, 10))
        .expect("station 3 passes to the dead station");
    let first = &frames[index];
    let retry = &frames[index + 1];
    assert!(retry.is(FrameType::TOKEN, 3, 10), "{retry}");
    assert_eq!(retry.start, first.end + usage_timeout);
    let poll = &frames[index + 2];
    assert!(poll.is(FrameType::POLL_FOR_MASTER, 3, 11), "{poll}");
    assert_eq!(poll.start, retry.end + usage_timeout);
    assert_eq!(sim.core(3).unwrap().counters().token_retries, 1);

    let answer = first_after(frames, poll.start, |frame| {
        frame.frame_type() == Some(FrameType::REPLY_TO_POLL_FOR_MASTER)
    })
    .expect("a new successor answers");
    assert_eq!(answer.source(), Some(30));
    let passes: Vec<(u8, u8)> = token_passes(frames)
        .into_iter()
        .filter(|&(at, ..)| at > answer.end)
        .map(|(_, from, to)| (from, to))
        .take(6)
        .collect();
    assert_eq!(passes, [(3, 30), (30, 0), (0, 3), (3, 30), (30, 0), (0, 3)]);
    sim.assert_no_violations();
}

/// Clause 9.5.6.6 (FindNewSuccessorUnknown) and 9.5.6.8 (DeclareSoleMaster).
///
/// When the address after the dead successor is the passer's own, it starts
/// polling from the address after itself instead. Here nobody else is left,
/// so the sweep ends with the passer declaring itself sole master.
#[test]
fn a_dead_successor_just_below_the_passer_leaves_it_sole_master() {
    let mut sim = ring(BAUD, &[4, 5], 5);
    // Station 4 dies just after handing the token to 5.
    run_until_frame(&mut sim, ms(1_500), ms(2_000), |frame| {
        frame.is(FrameType::TOKEN, 4, 5)
    });
    assert_eq!(sim.core(5).unwrap().snapshot().next_station, 4);
    let now = sim.now();
    sim.inject(now, Fault::PowerOff { station: 4 });
    sim.run_until(now + millis(1_500));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.start > now && frame.is(FrameType::TOKEN, 5, 4))
        .expect("station 5 passes to the dead station");
    assert!(frames[index + 1].is(FrameType::TOKEN, 5, 4));
    let sweep: Vec<u8> = frames[index + 2..index + 7]
        .iter()
        .filter(|frame| frame.frame_type() == Some(FrameType::POLL_FOR_MASTER))
        .filter_map(|frame| frame.destination())
        .collect();
    assert_eq!(sweep, [0, 1, 2, 3, 4]);
    assert!(sim.core(5).unwrap().snapshot().sole_master);
    sim.assert_no_violations();
}

/// Clause 9.5.6.6 (SawTokenUser): the passer lets go of the token only once
/// it has seen more than Nmin_octets (4) octets or errors. A successor that
/// dies four octets into its first frame leaves the passer still waiting, so
/// it sends the token again.
#[test]
fn four_octets_from_a_successor_are_not_enough_to_let_go() {
    let mut sim = ring(BAUD, &[4, 5], 5);
    run_until_frame(&mut sim, ms(1_500), ms(2_000), |frame| {
        frame.is(FrameType::TOKEN, 5, 4)
    });
    let now = sim.now();
    sim.inject(
        now,
        Fault::PowerOffMidFrame {
            station: 4,
            after_octets: 4,
        },
    );
    sim.run_until(now + millis(200));
    let frames = sim.frames();
    let cut = frames
        .iter()
        .find(|frame| frame.start > now && frame.truncated)
        .expect("station 4 is cut off");
    assert_eq!(cut.end - cut.start, sim.timing().boundary(4));
    let again = first_after(frames, cut.end, |_| true).expect("station 5 goes on");
    assert!(again.is(FrameType::TOKEN, 5, 4), "{again}");
    assert_eq!(again.start, cut.end + millis(20));
    sim.assert_no_violations();
}

/// Clauses 9.5.6.1 and 9.5.6.2 (ReceivedToken): a master that restarts
/// before anyone has replaced it is still its predecessor's successor. The
/// next Token addressed to it finds it in IDLE, and it takes it up, starting
/// with no known successor of its own.
#[test]
fn a_master_back_before_it_is_replaced_takes_the_next_token() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.inject(ms(2_000), Fault::PowerOff { station: 10 })
        .inject(ms(2_400), Fault::PowerOn { station: 10 });
    sim.run_until(ms(4_000));
    let frames = sim.frames();
    let index = frames
        .iter()
        .position(|frame| frame.start > ms(2_400) && frame.is(FrameType::TOKEN, 3, 10))
        .expect("station 3 still passes to 10");
    let next = &frames[index + 1];
    assert!(next.is(FrameType::POLL_FOR_MASTER, 10, 11), "{next}");
    assert_eq!(next.start, frames[index].end + sim.timing().bits(40));
    assert!(!frames.iter().any(
        |frame| frame.start > ms(2_000) && frame.is(FrameType::REPLY_TO_POLL_FOR_MASTER, 10, 3)
    ));
    assert_eq!(
        token_users(frames, next.end)
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 10, 30]
    );
    sim.assert_no_violations();
}

/// Clauses 9.5.6.6 and 9.5.6.5 together: once a dead master has been
/// replaced as successor, it rejoins only when the maintenance polls reach
/// its address again.
#[test]
fn a_replaced_master_is_polled_back_in() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.inject(ms(2_000), Fault::PowerOff { station: 10 })
        .inject(ms(4_000), Fault::PowerOn { station: 10 });
    sim.run_until(ms(4_000));
    assert_eq!(sim.core(3).unwrap().snapshot().next_station, 30);
    sim.run_until(ms(9_000));
    let frames = sim.frames();
    let rejoined = first_after(frames, ms(4_000), |frame| {
        frame.is(FrameType::REPLY_TO_POLL_FOR_MASTER, 10, 3)
    })
    .expect("station 3 polls 10 back in");
    assert_eq!(
        token_users(frames, rejoined.end)
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 10, 30]
    );
    sim.assert_no_violations();
}
