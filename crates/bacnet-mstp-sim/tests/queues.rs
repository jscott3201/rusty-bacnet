//! Scenario (h): a master whose queue is full, or whose application hands it
//! an NPDU too long for a frame, still passes the token.

mod common;

use bacnet_mstp_codec::{FrameType, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::QueueRefusal;
use bacnet_mstp_sim::analysis::{holds, token_users};
use bacnet_mstp_sim::{MasterSpec, Sim, SimConfig};

use common::*;

fn ring_with(station_3: MasterSpec) -> Sim {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 15, BAUD))
        .add_master(station_3)
        .add_master(MasterSpec::new(10, 15, BAUD));
    sim
}

/// Clause 9.5.6.3 (SendNoWait, NothingToSend) with bounded queues (local
/// policy): a full queue refuses what it can't hold, what it holds goes out
/// one frame per token use, and the token keeps moving.
#[test]
fn a_full_queue_refuses_npdus_and_the_token_keeps_moving() {
    let mut sim = ring_with(MasterSpec::new(3, 15, BAUD).queues(2, 2));
    for tag in 0..10 {
        sim.enqueue(ms(1_500), 3, 10, &npdu(false, tag));
        sim.enqueue(ms(1_500), 3, 0xFF, &npdu(false, 100 + tag));
    }
    sim.run_until(ms(3_000));
    let rejects = sim.rejects(3);
    assert_eq!(rejects.len(), 16);
    assert!(
        rejects
            .iter()
            .all(|&(_, reason)| reason == QueueRefusal::Full)
    );
    assert_eq!(
        sent_by(sim.frames(), 3, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY).len(),
        4
    );
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(counters.unicast.drops_full, 8);
    assert_eq!(counters.broadcast.drops_full, 8);
    assert_eq!(
        token_users(sim.frames(), ms(2_000))
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 10]
    );
    sim.assert_no_violations();
}

/// Clause 9.3 caps a standard frame's data at 501 octets. A longer NPDU is
/// refused when it is queued and never holds the token; the largest legal
/// one goes out as a 512-octet frame.
#[test]
fn an_oversized_npdu_is_refused_and_the_token_keeps_moving() {
    let mut sim = ring_with(MasterSpec::new(3, 15, BAUD));
    let mut largest = vec![0x5Au8; MAX_STANDARD_MPDU_DATA];
    largest[0] = 0x01;
    largest[1] = 0x00;
    let mut too_long = largest.clone();
    too_long.push(0);
    sim.enqueue(ms(1_500), 3, 10, &too_long)
        .enqueue(ms(1_500), 3, 10, &largest);
    sim.run_until(ms(3_000));
    assert_eq!(sim.rejects(3), [(ms(1_500), QueueRefusal::Oversize)]);
    let sent = sent_by(sim.frames(), 3, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].octets.len(), MAX_STANDARD_FRAME_LENGTH);
    assert!(
        sim.deliveries(10)
            .iter()
            .any(|delivery| delivery.data == largest)
    );
    assert_eq!(sim.core(3).unwrap().counters().unicast.drops_oversize, 1);
    assert_eq!(
        token_users(sim.frames(), ms(2_000))
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 10]
    );
    sim.assert_no_violations();
}

/// Clauses 9.5.6.3 (NothingToSend) and 9.5.3 (Max_Info_Frames up to 255):
/// the limit is a ceiling, not a quota. A master allowed 255 frames per token
/// with two queued sends two and passes the token at once.
#[test]
fn a_high_max_info_frames_does_not_hold_the_token_on_an_empty_queue() {
    let mut sim = ring_with(
        MasterSpec::new(3, 15, BAUD)
            .max_info_frames(255)
            .queues(2, 2),
    );
    sim.enqueue(ms(1_500), 3, 10, &npdu(false, 1))
        .enqueue(ms(1_500), 3, 0, &npdu(false, 2));
    sim.run_until(ms(3_000));
    let frames = sim.frames();
    let busy: Vec<_> = holds(frames, 3)
        .into_iter()
        .filter(|hold| hold.info_frames > 0)
        .collect();
    assert_eq!(busy.len(), 1);
    assert_eq!(busy[0].info_frames, 2);
    let second = sent_by(frames, 3, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY)[1];
    let next = first_after(frames, second.end, |_| true).expect("the token goes on");
    assert_eq!(next.source(), Some(3));
    assert_ne!(
        next.frame_type(),
        Some(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY)
    );
    assert_eq!(next.start, second.end + sim.timing().bits(40));
    sim.assert_no_violations();
}

/// Clause 9.3 and the transmit watchdog: at 4800 baud the largest standard
/// frame takes 1.06 s on the wire, longer than a one-second watchdog. The
/// default for that rate outlasts it, so the frame goes out whole, nothing
/// is aborted, and the token moves on.
#[test]
fn the_largest_standard_frame_goes_out_whole_at_4800_baud() {
    const SLOWER: u32 = 4_800;
    let mut sim = Sim::new(SimConfig::new(SLOWER));
    for station in [0, 3] {
        sim.add_master(MasterSpec::new(station, 3, SLOWER));
    }
    let mut largest = vec![0x3Cu8; MAX_STANDARD_MPDU_DATA];
    largest[0] = 0x01;
    largest[1] = 0x00;
    sim.enqueue(ms(1_500), 3, 0, &largest);
    sim.run_until(ms(8_000));
    let sent = sent_by(sim.frames(), 3, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].octets.len(), MAX_STANDARD_FRAME_LENGTH);
    assert!(sent[0].end - sent[0].start > millis(1_000));
    assert!(
        sim.deliveries(0)
            .iter()
            .any(|delivery| delivery.data == largest)
    );
    let counters = sim.core(3).unwrap().counters();
    assert_eq!(counters.cancelled_sends, 0);
    assert_eq!(counters.frames_transmitted, sent_by_count(sim.frames(), 3));
    assert_eq!(
        token_users(sim.frames(), ms(6_000))
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3]
    );
    assert!(!sim.perturbed());
    sim.assert_no_violations();
}

/// Clean frames `source` sent.
fn sent_by_count(frames: &[bacnet_mstp_sim::WireFrame], source: u8) -> u32 {
    frames
        .iter()
        .filter(|frame| frame.is_clean() && frame.source() == Some(source))
        .count() as u32
}
