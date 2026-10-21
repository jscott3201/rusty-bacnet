//! The statistics engine against scripted lines, with hand-set times: no
//! port, no task, no clock.

use std::time::Duration;

use bacnet_mstp_codec::{FrameHeader, FrameType};

use super::script::{analyse, Damage, LineScript, ScriptedRing, TURNAROUND, USAGE_TIMEOUT};
use super::stats::{
    octet_time, LineErrorKind, LineEvent, MaxMaster, MonitorStats, NO_TOKEN_SILENCE,
    SILENCE_BUCKET_BOUNDS,
};

const BAUD: u32 = 38_400;
/// A request NPDU (control octet 0x04: a reply is expected).
const REQUEST: [u8; 3] = [0x01, 0x04, 0x10];
/// A reply NPDU.
const REPLY: [u8; 3] = [0x01, 0x00, 0x20];
/// A broadcast NPDU.
const ANNOUNCE: [u8; 3] = [0x01, 0x00, 0x30];

/// The issue's ring: masters 1 and 2 pass the token, station 2 runs with
/// Max_Master 3 so it never polls past 3, and station 5, a master the
/// ring never finds, answers the requests station 1 sends it.
fn acceptance_ring(holds: usize) -> ScriptedRing {
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    ring.rotate(holds, answer_from_5);
    ring
}

fn answer_from_5(line: &mut LineScript, holder: u8) {
    if holder == 1 {
        line.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &REQUEST)
            .frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 1, 5, &REPLY);
    }
}

fn analyse_ring(ring: &ScriptedRing) -> MonitorStats {
    analyse(BAUD, ring.line.chunks())
}

#[test]
fn ring_reports_present_stations_token_holders_and_max_master() {
    let stats = analyse_ring(&acceptance_ring(40));

    assert_eq!(stats.present_stations().collect::<Vec<_>>(), [1, 2, 5]);
    let five = stats.station(5).unwrap();
    assert_eq!(five.tokens_received, 0, "station 5 never gets the token");
    assert_eq!(five.tokens_passed, 0);
    assert!(!five.is_master());
    assert_eq!(five.frames_sent.data_not_expecting_reply, 20);
    assert_eq!(five.frames_received.data_expecting_reply, 20);

    let two = stats.station(2).unwrap();
    // Its polls wrap from 3 to 0, which shows Max_Master exactly.
    assert_eq!(two.max_master, Some(MaxMaster::Exact(3)));
    assert_eq!(two.next_station, Some(1));
    assert_eq!(two.tokens_received, 20);
    assert_eq!(two.tokens_passed, 20);
    // Its polls go to 3, then 0, then it passes without one, and again.
    assert_eq!(two.frames_sent.poll_for_master, 14);

    let one = stats.station(1).unwrap();
    // Station 1's successor is the next address, so it never polls.
    assert_eq!(one.max_master, None);
    assert_eq!(one.next_station, Some(2));
    assert!(one.is_master() && two.is_master());

    // Stations 0 and 3 are polled but never answer.
    assert!(!stats.station(0).unwrap().is_present());
    assert!(!stats.station(3).unwrap().is_present());
    assert_eq!(stats.token_passes, 40);
    assert_eq!(stats.lost_tokens, 0);
    assert_eq!(stats.errors, Default::default());
    assert_eq!(stats.frames.total(), 40 + 14 + 40);
}

#[test]
fn rotation_and_usage_delay_are_timed() {
    let stats = analyse_ring(&acceptance_ring(40));
    let one = stats.station(1).unwrap();
    // Twenty Tokens from station 1 give nineteen rotations.
    assert_eq!(one.token_rotation.count, 19);
    let shortest = one.token_rotation.min.unwrap();
    let longest = one.token_rotation.max.unwrap();
    assert!(shortest < longest, "{shortest:?} {longest:?}");
    assert!(one.token_rotation.mean().unwrap() > shortest);
    // Each station takes the token up a turnaround after it arrives;
    // nothing follows the last Token, to station 1.
    let two = stats.station(2).unwrap();
    assert_eq!(two.token_usage_delay.count, 20);
    assert_eq!(two.token_usage_delay.max, Some(TURNAROUND));
    assert_eq!(one.token_usage_delay.count, 19);
    assert_eq!(one.token_usage_delay.min, Some(TURNAROUND));
}

/// A hold runs from the Token to the holder to the Token it passes on;
/// its data frames count, its polls and the answers it gets don't.
#[test]
fn holds_are_timed_and_their_frames_counted() {
    let mut ring = acceptance_ring(40);
    let stats = analyse_ring(&ring);
    let one = stats.station(1).unwrap();
    // Station 1 holds first, before any Token to it: 19 of its 20 holds.
    assert_eq!(one.token_hold.count, 19);
    let frames = &one.frames_per_token;
    assert_eq!((frames.max, frames.total), (Some(1), 19));
    let two = stats.station(2).unwrap();
    assert_eq!(two.token_hold.count, 20);
    let frames = &two.frames_per_token;
    assert_eq!((frames.max, frames.total), (Some(0), 0));
    // A hold with a poll that nobody answers lasts at least its timeout.
    assert!(two.token_hold.max.unwrap() >= USAGE_TIMEOUT);
    assert!(one.token_hold.min.unwrap() >= TURNAROUND);

    // Across a lost token the monitor can't tell who held it: the new
    // token's first holder isn't timed, and the ring goes on as before.
    ring.lose_token(Duration::from_millis(600));
    ring.rotate(4, answer_from_5);
    let after = analyse_ring(&ring);
    assert_eq!(after.station(1).unwrap().token_hold.count, 19 + 2);
    assert_eq!(after.station(2).unwrap().token_hold.count, 20 + 2);
}

/// A Token or poll from a station that doesn't hold the token means a
/// frame went missing: the hold in progress is dropped, not measured.
#[test]
fn a_frame_from_someone_else_drops_the_hold() {
    let mut script = LineScript::new(BAUD);
    script
        .frame(FrameType::TOKEN, 2, 1, &[])
        .frame(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            255,
            2,
            &ANNOUNCE,
        )
        .frame(FrameType::POLL_FOR_MASTER, 5, 4, &[])
        .frame(FrameType::TOKEN, 1, 2, &[])
        .frame(FrameType::TOKEN, 2, 1, &[]);
    let stats = analyse(BAUD, script.chunks());
    assert_eq!(stats.station(2).unwrap().token_hold.count, 0);
    assert_eq!(stats.station(1).unwrap().token_hold.count, 1);
}

/// A retried Token ends its sender's hold once, and the successor's hold
/// runs from the retry.
#[test]
fn a_token_retry_gives_one_hold_sample() {
    let mut script = LineScript::new(BAUD);
    script
        .frame(FrameType::TOKEN, 1, 2, &[])
        .frame(FrameType::TOKEN, 2, 1, &[])
        .idle(USAGE_TIMEOUT)
        .frame(FrameType::TOKEN, 2, 1, &[])
        .frame(FrameType::TOKEN, 1, 2, &[]);
    let stats = analyse(BAUD, script.chunks());
    let one = stats.station(1).unwrap();
    assert_eq!((one.token_retries, one.token_hold.count), (1, 1));
    let two = stats.station(2).unwrap().token_hold;
    assert_eq!(two.count, 1);
    assert!(two.max.unwrap() < USAGE_TIMEOUT, "timed from the retry");
}

#[test]
fn crc_errors_are_counted_exactly() {
    const HEADER_ERRORS: usize = 3;
    const DATA_ERRORS: usize = 5;
    let mut ring = ScriptedRing::new(BAUD).master(1, 127).master(2, 3);
    let mut sent = 0;
    ring.rotate(40, |line, holder| {
        if holder != 2 {
            return;
        }
        let damage = match sent {
            n if n < HEADER_ERRORS => Some(Damage::HeaderCrc),
            n if n < HEADER_ERRORS + DATA_ERRORS => Some(Damage::DataCrc),
            _ => None,
        };
        sent += 1;
        match damage {
            Some(damage) => line.damaged(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                2,
                &ANNOUNCE,
                damage,
            ),
            None => line.frame(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                2,
                &ANNOUNCE,
            ),
        };
    });
    let stats = analyse_ring(&ring);
    assert_eq!(stats.errors.header_crc, HEADER_ERRORS as u64);
    assert_eq!(stats.errors.data_crc, DATA_ERRORS as u64);
    assert_eq!(stats.errors.malformed, 0);
    assert_eq!(stats.errors.frame_aborts, 0);
    let two = stats.station(2).unwrap();
    assert_eq!(
        two.frames_sent.data_not_expecting_reply,
        (20 - HEADER_ERRORS - DATA_ERRORS) as u64
    );
    // The damage cost nothing else.
    assert_eq!(stats.token_passes, 40);
    assert_eq!(two.max_master, Some(MaxMaster::Exact(3)));
}

#[test]
fn one_lost_token_is_one_event() {
    let mut ring = acceptance_ring(20);
    ring.lose_token(Duration::from_millis(600));
    ring.rotate(20, answer_from_5);
    let stats = analyse_ring(&ring);
    assert_eq!(stats.lost_tokens, 1);
    let lost = stats.last_lost_token.unwrap();
    assert_eq!(lost.regenerated_by, Some(1));
    assert!(lost.silence >= Duration::from_millis(600), "{lost:?}");
    assert!(lost.silence < Duration::from_millis(602), "{lost:?}");
    assert_eq!(stats.silences.counts[SILENCE_BUCKET_BOUNDS.len()], 1);
    assert_eq!(stats.silences.longest, Some(lost.silence));
    // The generation poll gives station 1 a Max_Master estimate too, but
    // only a lower bound: its polls never wrap.
    assert_eq!(
        stats.station(1).unwrap().max_master,
        Some(MaxMaster::AtLeast(2))
    );
    assert_eq!(
        stats.station(2).unwrap().max_master,
        Some(MaxMaster::Exact(3))
    );
}

#[test]
fn a_silence_just_short_of_no_token_is_not_a_lost_token() {
    let mut ring = acceptance_ring(4);
    ring.line.idle(NO_TOKEN_SILENCE - Duration::from_millis(2));
    ring.rotate(4, answer_from_5);
    let stats = analyse_ring(&ring);
    assert_eq!(stats.lost_tokens, 0);
    assert!(stats.silences.longest.unwrap() < NO_TOKEN_SILENCE);
}

#[test]
fn a_coarse_adapter_changes_times_not_counts() {
    let mut ring = acceptance_ring(30);
    ring.lose_token(Duration::from_millis(700));
    ring.rotate(30, answer_from_5);
    let fine = analyse_ring(&ring);
    let coarse = analyse(BAUD, &ring.line.rechunked(Duration::from_millis(16)));
    assert!(coarse.timing.max_octets_per_read > fine.timing.max_octets_per_read);
    assert!(coarse.timing.reads < fine.timing.reads);
    assert_eq!(coarse.octets, fine.octets);
    assert_eq!(coarse.frames, fine.frames);
    assert_eq!(coarse.lost_tokens, 1);
    for (address, station) in &fine.stations {
        let other = coarse.station(*address).unwrap();
        assert_eq!(other.frames_sent, station.frames_sent, "station {address}");
        assert_eq!(other.tokens_received, station.tokens_received);
        assert_eq!(other.max_master, station.max_master);
    }
}

#[test]
fn a_frame_cut_off_by_silence_is_a_frame_abort() {
    let mut line = LineScript::new(BAUD);
    line.frame(FrameType::TOKEN, 2, 1, &[]);
    // Half a header, then nothing for 200 ms.
    line.raw(&[0x55, 0xFF, 0x00, 0x03])
        .idle(Duration::from_millis(200));
    line.frame(FrameType::TOKEN, 3, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.errors.frame_aborts, 1);
    assert_eq!(stats.frames.token, 2);
    assert_eq!(stats.station(2).unwrap().tokens_passed, 1);
}

#[test]
fn a_token_sent_twice_is_a_retry() {
    let mut line = LineScript::new(BAUD);
    line.frame(FrameType::TOKEN, 2, 1, &[])
        .idle(Duration::from_millis(20))
        .frame(FrameType::TOKEN, 2, 1, &[])
        .frame(FrameType::POLL_FOR_MASTER, 3, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    let one = stats.station(1).unwrap();
    // The second Token is a retry, not a second pass (Clause 9.5.6.6).
    assert_eq!(one.token_retries, 1);
    assert_eq!(one.tokens_passed, 1);
    assert_eq!(stats.token_passes, 1);
    assert_eq!(stats.station(2).unwrap().tokens_received, 1);
    assert_eq!(one.token_rotation.count, 0);
}

/// Stations 1 and 2 pass the token back and forth four times each; with
/// `retry`, station 1 sends its second Token twice.
fn back_and_forth(retry: bool) -> MonitorStats {
    let mut line = LineScript::new(BAUD);
    for round in 0..4 {
        line.frame(FrameType::TOKEN, 2, 1, &[]);
        if retry && round == 1 {
            line.idle(Duration::from_millis(20))
                .frame(FrameType::TOKEN, 2, 1, &[]);
        }
        line.frame(FrameType::TOKEN, 1, 2, &[]);
    }
    analyse(BAUD, line.chunks())
}

#[test]
fn a_retry_takes_no_rotation_sample() {
    let quiet = back_and_forth(false);
    let rotation = quiet.station(1).unwrap().token_rotation;
    assert_eq!(rotation.count, 3);
    assert_eq!(rotation.min, rotation.max);

    let stats = back_and_forth(true);
    let one = stats.station(1).unwrap();
    assert_eq!(one.token_retries, 1);
    assert_eq!(one.tokens_passed, 4);
    // Every sample is a whole rotation, measured from the first of the two
    // Tokens: the retry neither ends one nor starts one.
    assert_eq!(one.token_rotation.count, 3);
    assert_eq!(one.token_rotation.min, rotation.min);
    assert!(one.token_rotation.max > rotation.max);
}

#[test]
fn a_line_without_masters_never_loses_a_token() {
    // Only slaves answering, and noise, with long silences between.
    let mut line = LineScript::new(BAUD);
    for _ in 0..3 {
        line.frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 255, 130, &REPLY)
            .idle(Duration::from_millis(700))
            .raw(&[0x00, 0xF0])
            .idle(Duration::from_millis(700));
    }
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.lost_tokens, 0);
    assert_eq!(stats.silences.counts[SILENCE_BUCKET_BOUNDS.len()], 5);
}

#[test]
fn a_frame_abort_is_line_activity() {
    // Silences of 300 ms either side of a partial frame: no silence long
    // enough to lose the token in.
    let mut line = LineScript::new(BAUD);
    line.frame(FrameType::TOKEN, 2, 1, &[])
        .idle(Duration::from_millis(300))
        .raw(&[0x55, 0xFF, 0x00, 0x03])
        .idle(Duration::from_millis(300))
        .frame(FrameType::TOKEN, 1, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.errors.frame_aborts, 1);
    assert_eq!(stats.lost_tokens, 0);
    assert!(stats.silences.longest.unwrap() < Duration::from_millis(310));
}

#[test]
fn the_lost_token_threshold_allows_for_the_resolution() {
    let token = |destination, source| LineEvent::Frame {
        header: FrameHeader::new(FrameType::TOKEN, destination, source),
        wire_length: 8,
    };
    let eight_octets = octet_time(BAUD) * 8;
    for (resolution, lost) in [(Duration::ZERO, 0), (Duration::from_millis(16), 1)] {
        let mut stats = MonitorStats::new(BAUD, resolution);
        stats.record(Duration::from_millis(10), &token(2, 1));
        // 490 ms of silence, then the next frame.
        stats.record(Duration::from_millis(500) + eight_octets, &token(1, 2));
        assert_eq!(stats.lost_tokens, lost, "{resolution:?}");
    }
}

#[test]
fn cobs_frames_are_decoded_and_counted() {
    let npdu: Vec<u8> = (0..600u16).map(|value| value as u8).collect();
    let mut line = LineScript::new(BAUD);
    line.frame(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        255,
        7,
        &npdu,
    )
    .damaged(
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        4,
        7,
        &npdu,
        Damage::DataCrc,
    );
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.cobs_frames, 1);
    assert_eq!(stats.frames.extended_data_not_expecting_reply, 1);
    assert_eq!(stats.errors.data_crc, 1);
}

#[test]
fn noise_between_frames_is_counted_and_harmless() {
    let mut line = LineScript::new(BAUD);
    line.raw(&[0x00, 0x12, 0x34])
        .frame(FrameType::TOKEN, 2, 1, &[])
        .raw(&[0xFF, 0xFE])
        .frame(FrameType::TOKEN, 1, 2, &[]);
    let stats = analyse(BAUD, line.chunks());
    assert_eq!(stats.errors.noise_octets, 5);
    assert_eq!(stats.token_passes, 2);
}

#[test]
fn events_fed_by_hand() {
    let mut stats = MonitorStats::new(BAUD, Duration::from_millis(16));
    let token = |destination, source| LineEvent::Frame {
        header: FrameHeader::new(FrameType::TOKEN, destination, source),
        wire_length: 8,
    };
    stats.record(Duration::from_millis(10), &token(2, 1));
    stats.record(Duration::from_millis(15), &token(1, 2));
    stats.record(Duration::from_millis(20), &token(2, 1));
    stats.record(
        Duration::from_millis(25),
        &LineEvent::Error {
            kind: LineErrorKind::Malformed,
            destination: Some(3),
            octets: 8,
        },
    );
    assert_eq!(stats.timing.resolution, Duration::from_millis(16));
    assert_eq!(stats.timing.octet_time, octet_time(BAUD));
    assert_eq!(stats.errors.malformed, 1);
    let one = stats.station(1).unwrap();
    assert_eq!(one.token_rotation.count, 1);
    assert_eq!(one.token_rotation.last, Some(Duration::from_millis(10)));
    // Usage delay: from the end of the Token to the start of the answer.
    let eight_octets = octet_time(BAUD) * 8;
    assert_eq!(
        stats.station(2).unwrap().token_usage_delay.last,
        Some(Duration::from_millis(5) - eight_octets)
    );
    // Three silences, all in the first few buckets.
    assert_eq!(stats.silences.counts.iter().sum::<u64>(), 3);
}

#[test]
fn octet_time_is_ten_bit_times() {
    assert_eq!(octet_time(9_600), Duration::from_nanos(1_041_666));
    assert_eq!(octet_time(76_800), Duration::from_nanos(130_208));
    assert_eq!(octet_time(0), Duration::ZERO);
}

#[test]
fn max_master_is_exact_only_after_a_wrap_from_above() {
    let mut stats = MonitorStats::new(BAUD, Duration::ZERO);
    let mut at = Duration::ZERO;
    let mut poll = |stats: &mut MonitorStats, destination: u8| {
        at += Duration::from_millis(30);
        stats.record(
            at,
            &LineEvent::Frame {
                header: FrameHeader::new(FrameType::POLL_FOR_MASTER, destination, 9),
                wire_length: 8,
            },
        );
        stats.station(9).unwrap().max_master
    };
    // Below its own address and then 0 is no wrap past Max_Master.
    assert_eq!(poll(&mut stats, 1), Some(MaxMaster::AtLeast(1)));
    assert_eq!(poll(&mut stats, 0), Some(MaxMaster::AtLeast(1)));
    // Up through its gap and round to 0: Max_Master is the last one.
    assert_eq!(poll(&mut stats, 10), Some(MaxMaster::AtLeast(10)));
    assert_eq!(poll(&mut stats, 12), Some(MaxMaster::AtLeast(12)));
    assert_eq!(poll(&mut stats, 0), Some(MaxMaster::Exact(12)));
    assert_eq!(poll(&mut stats, 1), Some(MaxMaster::Exact(12)));
    // A later poll above it shows that wasn't the wrap after all.
    assert_eq!(poll(&mut stats, 15), Some(MaxMaster::AtLeast(15)));
    assert_eq!(MaxMaster::Exact(12).value(), 12);
}
