//! Scenario (b): steady-state token rotation.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_sim::analysis::{holds, holds_between_polls, token_passes};
use bacnet_mstp_sim::{MasterSpec, Sim, SimConfig};

use common::*;

const RING: [u8; 4] = [0, 3, 10, 30];

/// Clauses 9.5.6.3 and 9.5.6.5: each holder sends at most Max_Info_Frames
/// information frames per token (FrameCount), then passes the token on, so
/// the token visits every master in turn however much each has queued.
#[test]
fn rotation_is_round_robin_and_respects_max_info_frames() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    let max_info_frames = [1u8, 2, 3, 4];
    for (&station, &frames) in RING.iter().zip(&max_info_frames) {
        sim.add_master(
            MasterSpec::new(station, 31, BAUD)
                .max_info_frames(frames)
                .queues(32, 4),
        );
    }
    let queued = 30;
    for (index, &station) in RING.iter().enumerate() {
        let to = RING[(index + 1) % RING.len()];
        for tag in 0..queued {
            sim.enqueue(ms(1_200), station, to, &npdu(false, tag));
        }
    }
    sim.run_until(ms(4_000));
    let frames = sim.frames();

    for (&station, &limit) in RING.iter().zip(&max_info_frames) {
        let used: Vec<usize> = holds(frames, station)
            .iter()
            .filter(|hold| hold.start >= ms(1_200))
            .map(|hold| hold.info_frames)
            .take_while(|&count| count > 0)
            .collect();
        let limit = usize::from(limit);
        let mut expected = vec![limit; usize::from(queued) / limit];
        if usize::from(queued) % limit != 0 {
            expected.push(usize::from(queued) % limit);
        }
        assert_eq!(used, expected, "station {station}");
    }
    // Every NPDU arrived.
    for (index, &station) in RING.iter().enumerate() {
        let to = RING[(index + 1) % RING.len()];
        let got = sim
            .deliveries(to)
            .iter()
            .filter(|delivery| delivery.source == station)
            .count();
        assert_eq!(got, usize::from(queued), "{station} -> {to}");
    }
    // Round robin: once the ring has formed, every pass goes to the next
    // master in address order.
    let passes = token_passes(frames);
    let formed = passes
        .iter()
        .position(|&(_, from, _)| from == 30)
        .expect("the ring closes");
    for &(at, from, to) in &passes[formed..] {
        let next = RING[(RING.iter().position(|&s| s == from).unwrap() + 1) % RING.len()];
        assert_eq!(to, next, "pass at {at:?}");
    }
    assert!(!sim.perturbed());
}

/// Clause 9.5.6.5, SendToken, SendMaintenancePFM and ResetMaintenancePFM;
/// Npoll is 50 (Clause 9.5.3).
///
/// A master counts its token uses and, once it has used the token Npoll
/// times, polls one address between itself and its successor per token use
/// until it reaches the successor; then it starts counting again. So between
/// two maintenance cycles there are exactly 50 token uses with no poll.
#[test]
fn maintenance_polls_come_every_npoll_token_uses() {
    let mut sim = ring(BAUD, &RING, 31);
    sim.run_until(ms(15_000));
    let frames = sim.frames();
    for station in RING {
        let gaps = holds_between_polls(&holds(frames, station));
        assert!(gaps.len() >= 2, "station {station}: {gaps:?}");
        assert!(
            gaps.iter().all(|&gap| gap == 50),
            "station {station}: {gaps:?}"
        );
    }
    // Station 3's maintenance polls walk 4..=9, one per token use, and
    // start over after reaching its successor.
    let polls: Vec<u8> = sent_by(frames, 3, FrameType::POLL_FOR_MASTER)
        .into_iter()
        .skip(7)
        .take(12)
        .filter_map(|poll| poll.destination())
        .collect();
    assert_eq!(polls, [4, 5, 6, 7, 8, 9, 4, 5, 6, 7, 8, 9]);
    assert!(!sim.perturbed());
}

/// Clause 9.5.6.5, SendToken: with no address between a master and its
/// successor there is nobody to find, so the master never polls.
#[test]
fn adjacent_successors_never_poll() {
    let mut sim = ring(BAUD, &[0, 1, 2], 2);
    sim.run_until(ms(6_000));
    let frames = sim.frames();
    let formed = token_passes(frames)
        .iter()
        .find(|&&(_, from, _)| from == 2)
        .map(|&(at, ..)| at)
        .expect("the ring closes");
    let late_polls = frames
        .iter()
        .filter(|frame| frame.start > formed)
        .filter(|frame| frame.frame_type() == Some(FrameType::POLL_FOR_MASTER))
        .count();
    assert_eq!(late_polls, 0);
    assert!(token_passes(frames).len() > 1_000);
}
