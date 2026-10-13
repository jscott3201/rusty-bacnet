//! Scenario (a): cold start. Masters powered up together find the token
//! missing, the lowest station generates one in its slot, and Poll For
//! Master sweeps build the ring.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::analysis::{token_passes, token_users};
use bacnet_mstp_sim::{AdapterSpec, Fault, MasterSpec, Sim, SimConfig};

use common::*;

/// Clauses 9.5.6.1, 9.5.6.2, 9.5.6.7 and 9.5.6.8.
///
/// After INITIALIZE every master waits Tno_token in IDLE and moves to
/// NO_TOKEN; only the master whose slot (Tno_token + Tslot x TS) comes first
/// hears nothing in its slot and generates the token. It polls upwards one
/// address at a time, moving on after Tusage_timeout of silence; the first
/// master to answer gets the token and repeats the sweep from its own address,
/// until the ring closes back at the generator.
#[test]
fn cold_start_forms_the_ring_and_every_master_gets_the_token() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let octet8 = sim.timing().boundary(8);
    let turnaround = sim.timing().bits(40);
    let usage_timeout = millis(20);

    // Station 0's slot is the first: Tno_token + Tslot x 0.
    let first = &frames[0];
    assert!(
        first.is(FrameType::POLL_FOR_MASTER, 0, 1),
        "{}",
        log(frames)
    );
    assert!(first.generation);
    assert_eq!(first.start, ms(500));

    // Polls of absent stations 1 and 2 each wait Tusage_timeout of silence.
    let polls = sent_by(frames, 0, FrameType::POLL_FOR_MASTER);
    assert_eq!(polls[1].start, polls[0].end + usage_timeout);
    assert_eq!(polls[2].start, polls[1].end + usage_timeout);
    assert_eq!(polls[2].destination(), Some(3));
    // Station 3 answers after Tturnaround, and gets the token as quickly.
    let reply = &frames[3];
    assert!(reply.is(FrameType::REPLY_TO_POLL_FOR_MASTER, 3, 0));
    assert_eq!(reply.start, polls[2].end + turnaround);
    let pass = &frames[4];
    assert!(pass.is(FrameType::TOKEN, 0, 3));
    assert_eq!(pass.start, reply.end + turnaround);
    assert_eq!(pass.end - pass.start, octet8);

    // Each new holder sweeps from its own address to its successor.
    let passes: Vec<(u8, u8)> = token_passes(frames)
        .into_iter()
        .map(|(_, from, to)| (from, to))
        .collect();
    assert_eq!(
        &passes[..8],
        &[
            (0, 3),
            (3, 10),
            (10, 30),
            (30, 0),
            (0, 3),
            (3, 10),
            (10, 30),
            (30, 0)
        ]
    );
    let sweep = |station: u8, length: usize| -> Vec<u8> {
        sent_by(frames, station, FrameType::POLL_FOR_MASTER)
            .into_iter()
            .take(length)
            .filter_map(|poll| poll.destination())
            .collect()
    };
    assert_eq!(sweep(3, 7), (4..=10).collect::<Vec<u8>>());
    assert_eq!(sweep(10, 20), (11..=30).collect::<Vec<u8>>());
    // Station 30 wraps past Max_Master (31) to address 0.
    assert_eq!(sweep(30, 2), [31, 0]);

    assert_eq!(
        token_users(frames, ms(0)).into_iter().collect::<Vec<u8>>(),
        [0, 3, 10, 30]
    );
    assert!(!sim.perturbed());
    assert_eq!(sim.report().collisions.len(), 0);
}

/// Clauses 9.2.3 and 9.5.6.7 at the other required baud rate, with no master
/// at address 0: station 4's slot comes first, 40 ms into NO_TOKEN.
#[test]
fn cold_start_at_9600_baud_starts_from_the_lowest_slot_present() {
    let mut sim = ring(SLOW_BAUD, &[4, 7, 21], 25);
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    assert!(
        frames[0].is(FrameType::POLL_FOR_MASTER, 4, 5),
        "{}",
        log(frames)
    );
    assert!(frames[0].generation);
    assert_eq!(frames[0].start, ms(540));
    // A 9600-baud octet takes 1041.67 us; an 8-octet frame 8.333 ms.
    assert_eq!(
        frames[0].end - frames[0].start,
        Duration::from_micros(8_334)
    );
    let passes: Vec<(u8, u8)> = token_passes(frames)
        .into_iter()
        .map(|(_, from, to)| (from, to))
        .collect();
    assert_eq!(&passes[..4], &[(4, 7), (7, 21), (21, 4), (4, 7)]);
    assert!(!sim.perturbed());
}

/// Clauses 9.5.6.5 and 9.5.6.8, DeclareSoleMaster and the sole master's
/// maintenance polls.
///
/// A master that polls every other address without an answer declares itself
/// the sole master and uses the token without passing it. It keeps polling for
/// others every Npoll uses, so a master that appears later is found.
#[test]
fn a_lone_master_becomes_sole_master_and_still_finds_a_newcomer() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(5, 7, BAUD));
    sim.add_master(MasterSpec::new(2, 7, BAUD).unpowered());
    for tag in 0..3 {
        sim.enqueue(ms(1_000), 5, 6, &npdu(false, tag));
    }
    sim.run_until(ms(1_500));
    let frames = sim.frames();
    let sweep: Vec<u8> = sent_by(frames, 5, FrameType::POLL_FOR_MASTER)
        .into_iter()
        .take(7)
        .filter_map(|poll| poll.destination())
        .collect();
    assert_eq!(sweep, [6, 7, 0, 1, 2, 3, 4]);
    assert!(sim.core(5).unwrap().snapshot().sole_master);
    // The queued data goes out with no Token on the wire.
    assert_eq!(
        sent_by(frames, 5, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY).len(),
        3
    );
    assert!(token_passes(frames).is_empty(), "{}", log(frames));

    sim.inject(ms(1_600), Fault::PowerOn { station: 2 });
    sim.run_until(ms(4_000));
    let frames = sim.frames();
    let reply = first_after(frames, ms(1_600), |frame| {
        frame.frame_type() == Some(FrameType::REPLY_TO_POLL_FOR_MASTER)
    })
    .expect("the newcomer answers a maintenance poll");
    assert_eq!(reply.source(), Some(2));
    assert!(!sim.core(5).unwrap().snapshot().sole_master);
    assert!(token_users(frames, reply.end).contains(&5));
    assert!(token_users(frames, reply.end).contains(&2));
    sim.assert_no_violations();
}

/// Clause 9.5.4: a node that hears its own transmissions must ignore them.
/// The core ignores octets while its transmit is active, so adapters whose
/// receivers stay on produce exactly the same bus. That includes a token
/// passed to a dead successor, where a passer that took its own echo for the
/// successor's activity would never send the token again.
#[test]
fn adapters_that_hear_their_own_echo_change_nothing() {
    let run = |echo: bool| {
        let mut sim = Sim::new(SimConfig::new(BAUD));
        for station in [0, 3, 10, 30] {
            sim.add_master(MasterSpec::new(station, 31, BAUD).adapter(AdapterSpec {
                echo,
                ..AdapterSpec::default()
            }));
        }
        sim.enqueue(ms(1_000), 3, 10, &npdu(false, 1));
        sim.enqueue(ms(1_000), 10, 0, &npdu(true, 2));
        sim.inject(ms(2_000), Fault::PowerOff { station: 10 });
        sim.run_until(ms(3_000));
        sim.frames()
            .iter()
            .map(|frame| (frame.start, frame.header))
            .collect::<Vec<_>>()
    };
    let plain = run(false);
    assert!(plain.len() > 100);
    assert_eq!(plain, run(true));
}
