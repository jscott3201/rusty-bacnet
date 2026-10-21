//! Actual receiver observations, independent of the common physical timing.
use bacnet_mstp_core::{Duration, Instant};

use crate::line::Character;
use crate::{ContentionPolicy, Driver, HostSpec, MasterSpec, ScriptSend, Sim, SimConfig};

#[test]
fn one_physical_overlap_has_three_receiver_observations() {
    let mut config = SimConfig::new(10_000);
    config.tolerance.scripted_collisions = true;
    let mut sim = Sim::new(config);
    for station in [10, 11, 12] {
        sim.add_master(MasterSpec::new(station, 12, 10_000).host(HostSpec {
            receive_latency: Duration::from_millis(16),
            ..HostSpec::default()
        }));
    }
    sim.set_contention(10, ContentionPolicy::Capture(Driver::Station(1)));
    sim.set_contention(
        11,
        ContentionPolicy::Xor {
            driver: Driver::Station(1),
            offset: 0,
            mask: 0xFF,
        },
    );
    for station in [1, 2] {
        sim.add_scripted(station, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
        let node = sim.index_of(station);
        sim.script_send(
            node,
            ScriptSend {
                after: Duration::ZERO,
                inter_octet_gap: Duration::ZERO,
                post_drive: Duration::ZERO,
                octets: vec![if station == 1 { 0x55 } else { 0x33 }],
                malformed: true,
            },
        );
    }
    sim.run_until(Instant::from_micros(1_000));
    for (station, expected) in [
        (10, Character::Clean(0x55)),
        (11, Character::Clean(0xAA)),
        (12, Character::Garbled),
    ] {
        let input = &sim.master(station).host.as_ref().unwrap().input;
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].wire_end, Instant::from_micros(1_000));
        assert_eq!(input[0].character, expected, "station {station}");
    }
    assert_eq!(sim.frames().len(), 2);
    for frame in sim.frames() {
        assert_eq!(frame.start, Instant::ZERO);
        assert_eq!(frame.end, Instant::from_micros(1_000));
        assert!(frame.collided);
        assert!(!frame.is_clean());
    }
    assert_eq!(sim.report().collisions.len(), 1);
    sim.assert_no_violations();
}

pub(super) fn bus(baud: u32, host: bool, extended: bool) -> Sim {
    seeded_bus(baud, host, extended, 0)
}

pub(super) fn seeded_bus(baud: u32, host: bool, extended: bool, seed: u64) -> Sim {
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    config.panic_on_violation = false;
    config.tolerance.scripted_collisions = true;
    let mut sim = Sim::new(config);
    for station in [10, 11, 12, 13] {
        let mut spec = MasterSpec::new(station, 13, baud);
        if extended {
            spec = spec.extended_frames();
        }
        if host {
            spec = spec.host(HostSpec::default());
        }
        sim.add_master(spec);
    }
    for station in [1, 2, 3] {
        sim.add_scripted(station, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    }
    sim
}

pub(super) fn send(sim: &mut Sim, station: u8, at: Instant, frame: ScriptSend) {
    sim.schedule(
        at,
        crate::event::Class::Scheduled,
        station,
        crate::sim::Event::ScriptSend {
            node: sim.index_of(station),
            send: frame,
        },
    );
}

pub(super) fn frame(source: u8, extended: bool, data: &[u8]) -> ScriptSend {
    ScriptSend::frame(
        Duration::ZERO,
        if extended {
            bacnet_mstp_codec::FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
        } else {
            bacnet_mstp_codec::FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
        },
        255,
        source,
        data,
    )
}

pub(super) fn capture(sim: &mut Sim, station: u8) {
    sim.set_contention(station, ContentionPolicy::Capture(Driver::Station(1)));
}

fn data_crc(bytes: &[u8]) -> u16 {
    // Independent reflected bit recurrence, including transmitted CRC bytes.
    let mut crc = 0xFFFF_u16;
    for byte in bytes {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Valid capture, UART-clean bad data CRC, and receive errors take different
/// real core paths. Wire intent remains collided for every observer.
#[test]
fn standard_winner_crc_corruption_and_errors_remain_distinct() {
    for host in [false, true] {
        let mut sim = bus(10_000, host, false);
        capture(&mut sim, 10);
        sim.set_contention(
            11,
            ContentionPolicy::Xor {
                driver: Driver::Station(1),
                offset: 10,
                mask: 1,
            },
        );
        sim.set_contention(13, ContentionPolicy::Capture(Driver::Station(2)));
        let first = frame(1, false, &[1, 0, 0xA1]);
        let mut altered = first.octets.clone();
        altered[10] ^= 1;
        assert_eq!(data_crc(&first.octets[8..]), 0xF0B8);
        assert_ne!(data_crc(&altered[8..]), 0xF0B8);
        send(&mut sim, 1, Instant::ZERO, first);
        send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
        sim.run_until(Instant::from_micros(20_000));
        assert_eq!(
            sim.deliveries(10)
                .iter()
                .map(|d| d.data.clone())
                .collect::<Vec<_>>(),
            [vec![1, 0, 0xA1]]
        );
        assert_eq!(
            sim.deliveries(13)
                .iter()
                .map(|d| d.data.clone())
                .collect::<Vec<_>>(),
            [vec![1, 0, 0xB2]]
        );
        for station in [11, 12] {
            assert!(sim.deliveries(station).is_empty());
        }
        assert_eq!(sim.core(11).unwrap().counters().parser_errors, 1);
        assert_eq!(sim.contention_stats(11).altered, 1);
        assert_eq!(sim.contention_stats(11).malformed, 1);
        assert_eq!(sim.contention_stats(10).intact, 1);
        assert_eq!(sim.contention_stats(10).checked, 1);
        assert_eq!(sim.contention_stats(10).matched, 1);
        assert_eq!(sim.contention_stats(10).pending, 0);
        assert_eq!(sim.frames().len(), 2);
        assert!(sim
            .frames()
            .iter()
            .all(|f| f.collided && !f.is_clean() && f.end == Instant::from_micros(13_000)));
        assert_eq!(sim.report().driver_releases.len(), 2);
        assert!(sim
            .report()
            .driver_releases
            .iter()
            .all(|r| r.data_end == Some(Instant::from_micros(13_000))
                && r.at == Instant::from_micros(13_000)));
        sim.assert_no_violations();
    }
}

/// Two equal physical intents produce one perceived frame and one receipt.
#[test]
fn identical_simultaneous_frames_owe_only_one_receipt() {
    for host in [false, true] {
        let mut sim = bus(38_400, host, false);
        capture(&mut sim, 10);
        let first = frame(1, false, &[1, 0, 0x11]);
        send(&mut sim, 1, Instant::ZERO, first.clone());
        send(&mut sim, 2, Instant::ZERO, first);
        sim.run_until(Instant::from_micros(20_000));
        assert_eq!(sim.frames().len(), 2);
        assert!(sim.frames().iter().all(|f| f.collided));
        assert_eq!(sim.deliveries(10).len(), 1);
        assert_eq!(sim.core(10).unwrap().counters().frames_received, 1);
        assert_eq!(sim.contention_stats(10).intact, 1);
        assert_eq!(sim.contention_stats(10).checked, 1);
        sim.assert_no_violations();
    }
}

/// Extended acceptance is a receiver capability, separate from intact
/// capture and the physical collision. Both modes still count one stream.
#[test]
fn extended_winner_is_received_once_or_explicitly_unsupported() {
    for baud in [9_600, 38_400, 115_200] {
        for host in [false, true] {
            let mut sim = bus(baud, host, true);
            // One otherwise identical receiver does not support COBS frames.
            let node = sim.index_of(13);
            sim.power_off(node);
            sim.master_mut(node).unwrap().spec.config.extended_frames = false;
            sim.power_on(node);
            capture(&mut sim, 10);
            capture(&mut sim, 13);
            let mut payload = vec![0x5A; 601];
            payload[..2].copy_from_slice(&[1, 0]);
            let first = frame(1, true, &payload);
            let end = Duration::from_micros(
                (first.octets.len() as u64 * 10_000_000).div_ceil(u64::from(baud)),
            );
            send(&mut sim, 1, Instant::ZERO, first);
            send(&mut sim, 2, Instant::ZERO, frame(2, true, &payload));
            sim.run_until(Instant::ZERO + end + Duration::from_millis(20));
            assert_eq!(
                sim.deliveries(10)
                    .iter()
                    .map(|d| &d.data)
                    .collect::<Vec<_>>(),
                [&payload]
            );
            assert_eq!(sim.core(10).unwrap().counters().frames_received, 1);
            assert_eq!(sim.core(13).unwrap().counters().frames_received, 0);
            assert!(sim.deliveries(13).is_empty());
            for station in [10, 13] {
                assert_eq!(
                    sim.contention_stats(station).checked,
                    1,
                    "baud {baud}, host {host}, station {station}"
                );
                assert_eq!(sim.contention_stats(station).matched, 1);
                assert_eq!(sim.contention_stats(station).pending, 0);
            }
            assert!(sim.frames().iter().all(|f| f.collided));
            sim.assert_no_violations();
        }
    }
}

/// A later unrelated frame, even in the same host read, cannot replace a
/// vanished captured winner merely by increasing frames_received.
#[test]
fn missing_winner_stays_diagnosed_after_an_unrelated_valid_frame() {
    let mut sim = bus(38_400, true, false);
    let node = sim.index_of(10);
    sim.master_mut(node)
        .unwrap()
        .spec
        .adapter
        .host
        .as_mut()
        .unwrap()
        .receive_latency = Duration::from_millis(16);
    capture(&mut sim, 10);
    let first = frame(1, false, &[1, 0, 0xA1]);
    let end = Instant::from_micros((first.octets.len() as u64 * 10_000_000).div_ceil(38_400));
    send(&mut sim, 1, Instant::ZERO, first);
    send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
    send(&mut sim, 3, end, frame(3, false, &[1, 0, 0xC3]));
    sim.run_until(Instant::from_micros(8_000));
    assert_eq!(sim.contention_stats(10).pending, 1);
    // Deliberate adapter-loss mutation. It has no authorized filter
    // disposition, and the later frame must not discharge this obligation.
    sim.master_mut(node)
        .unwrap()
        .host
        .as_mut()
        .unwrap()
        .input
        .retain(|r| r.wire_end > end);
    sim.run_until(Instant::from_micros(25_000));
    assert_eq!(sim.core(10).unwrap().counters().frames_received, 1);
    assert_eq!(
        sim.deliveries(10)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        [vec![1, 0, 0xC3]]
    );
    assert_eq!(sim.contention_stats(10).pending, 0);
    let missed: Vec<_> = sim
        .report()
        .violations
        .iter()
        .filter(|v| v.kind == crate::ViolationKind::ListenerMissedFrame)
        .collect();
    assert_eq!(missed.len(), 1, "{}", sim.trace().tail(20));
    assert!(missed[0]
        .detail
        .contains("lost captured tx0's terminal character"));
}

/// Standalone-valid bytes do not imply parser synchronization: this winner
/// arrives inside an older candidate's declared body at the real core.
#[test]
fn an_older_incomplete_candidate_does_not_create_a_false_receipt_obligation() {
    let mut sim = bus(10_000, false, false);
    capture(&mut sim, 10);
    let mut prefix = frame(3, false, &[0x66; 80]);
    prefix.octets.truncate(8);
    prefix.malformed = true;
    send(&mut sim, 3, Instant::ZERO, prefix);
    send(
        &mut sim,
        1,
        Instant::from_micros(8_000),
        frame(1, false, &[1, 0, 0xA1]),
    );
    send(
        &mut sim,
        2,
        Instant::from_micros(8_000),
        frame(2, false, &[1, 0, 0xB2]),
    );
    sim.run_until(Instant::from_micros(25_000));
    assert!(sim.deliveries(10).is_empty());
    assert_eq!(sim.contention_stats(10).intact, 1);
    assert_eq!(sim.contention_stats(10).unqualified, 1);
    assert_eq!(sim.contention_stats(10).checked, 0);
    sim.assert_no_violations();
}
