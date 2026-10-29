//! Host equivalents of the scenario families, preserving wire obligations.
mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, QueueRefusal};
use bacnet_mstp_sim::analysis::token_users;
use bacnet_mstp_sim::{Fault, HostSpec, MasterSpec, ReplyPolicy, Sim, SimConfig};
use common::*;

fn master(station: u8, max: u8, baud: u32, mixed: bool, latency: u64) -> MasterSpec {
    let spec = MasterSpec::new(station, max, baud);
    if mixed && !station.is_multiple_of(2) {
        return spec;
    }
    let mut spec = spec.host(HostSpec {
        receive_latency: Duration::from_millis(latency),
        max_chunk: 23,
        random_chunks: true,
        echo_grace: Some(Duration::from_millis(40)),
        ..HostSpec::default()
    });
    spec.adapter.echo = true;
    spec
}

/// Formation, rotation and both immediate and postponed application replies
/// work at 1, 4 and 8 ms latency. The first reply byte is checked on the wire.
#[test]
fn replies_and_rotation_across_host_and_mixed_timing_budgets() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            for latency in [1, 4, 8] {
                for delay in [5, 240] {
                    let mut sim = Sim::new(SimConfig::new(baud));
                    for station in [0, 3] {
                        sim.add_master(master(station, 3, baud, mixed, latency).application(
                            ReplyPolicy::Answer {
                                delay: millis(delay),
                                payload: npdu(false, 0xA9),
                            },
                        ));
                    }
                    sim.enqueue(ms(1_000), 0, 3, &npdu(true, 0x11));
                    sim.enqueue(ms(1_500), 3, 0, &npdu(true, 0x22));
                    sim.run_until(ms(3_000));
                    for station in [0, 3] {
                        assert!(
                            sim.deliveries(station)
                                .iter()
                                .any(|d| d.data == npdu(false, 0xA9)),
                            "baud={baud} mixed={mixed} latency={latency} delay={delay}: {}",
                            sim.dump(80)
                        );
                        assert_eq!(sim.core(station).unwrap().counters().own_address_frames, 0);
                    }
                    for (index, request) in sim.frames().iter().enumerate().filter(|(_, frame)| {
                        frame.frame_type() == Some(FrameType::BACNET_DATA_EXPECTING_REPLY)
                    }) {
                        let reply = &sim.frames()[index + 1];
                        assert!(reply.start - request.end <= millis(250));
                        assert_eq!(
                            reply.frame_type(),
                            Some(if delay == 240 {
                                FrameType::REPLY_POSTPONED
                            } else {
                                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                            })
                        );
                    }
                    assert_eq!(
                        token_users(sim.frames(), ms(2_000))
                            .into_iter()
                            .collect::<Vec<_>>(),
                        [0, 3]
                    );
                    sim.assert_no_violations();
                }
            }
        }
    }
}

/// Maintenance discovers a late joiner and a restarted successor. A bounded
/// slow start remains within the physical response budget in both modes.
#[test]
fn maintenance_join_leave_and_slow_start_recover() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            let mut sim = Sim::new(SimConfig::new(baud));
            for station in [0, 2, 3] {
                let mut spec = master(station, 3, baud, mixed, 4);
                if station == 0 {
                    spec.adapter.start_delay = millis(2);
                }
                if station == 3 {
                    spec = spec.unpowered();
                }
                sim.add_master(spec);
            }
            sim.inject(ms(1_200), Fault::PowerOn { station: 3 });
            sim.inject(ms(6_000), Fault::PowerOff { station: 2 });
            sim.inject(ms(7_000), Fault::PowerOn { station: 2 });
            sim.run_until(ms(12_000));
            assert!(
                sim.frames()
                    .iter()
                    .any(|frame| frame.is(FrameType::POLL_FOR_MASTER, 2, 3))
            );
            assert_eq!(
                token_users(sim.frames(), ms(10_000))
                    .into_iter()
                    .collect::<Vec<_>>(),
                [0, 2, 3],
                "{}",
                sim.dump(80)
            );
            sim.assert_no_violations();
        }
    }
}

/// Loss of every token holder is followed by regeneration and a complete
/// ring, with no old buffered input crossing the reboot boundary.
#[test]
fn losing_all_holders_regenerates_the_ring() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            let mut sim = Sim::new(SimConfig::new(baud));
            for station in [0, 3] {
                sim.add_master(master(station, 3, baud, mixed, 4));
                sim.inject(ms(2_000), Fault::PowerOff { station });
                sim.inject(ms(2_200), Fault::PowerOn { station });
            }
            sim.run_until(ms(4_000));
            assert!(sim.core(0).unwrap().counters().token_regenerations > 0);
            assert_eq!(
                token_users(sim.frames(), ms(3_000))
                    .into_iter()
                    .collect::<Vec<_>>(),
                [0, 3]
            );
            sim.assert_no_violations();
        }
    }
}

/// Full and oversized queues reject locally while accepted standard frames
/// drain and the token continues to rotate beside host readers.
#[test]
fn bounded_queues_and_oversize_refusal_keep_the_ring_moving() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            let mut sim = Sim::new(SimConfig::new(baud));
            sim.add_master(master(0, 3, baud, mixed, 4).queues(2, 2));
            sim.add_master(master(3, 3, baud, mixed, 4));
            sim.enqueue(ms(800), 0, 3, &vec![0x11; 502]);
            for tag in 0..10 {
                sim.enqueue(ms(1_000), 0, 3, &npdu(false, tag));
            }
            sim.run_until(ms(2_500));
            assert_eq!(sim.rejects(0).len(), 9);
            assert_eq!(sim.rejects(0)[0].1, QueueRefusal::Oversize);
            assert!(
                sim.rejects(0)[1..]
                    .iter()
                    .all(|(_, reason)| *reason == QueueRefusal::Full)
            );
            assert_eq!(sim.deliveries(3).len(), 2);
            assert_eq!(
                token_users(sim.frames(), ms(2_000))
                    .into_iter()
                    .collect::<Vec<_>>(),
                [0, 3]
            );
            sim.assert_no_violations();
        }
    }
}

/// The host's 300 ms reply timeout starts at the transmitted frame end,
/// including when echo notification or received chunks arrive later.
#[test]
fn no_answer_uses_the_host_reply_timeout_from_wire_completion() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            let mut sim = Sim::new(SimConfig::new(baud));
            sim.add_master(master(0, 3, baud, mixed, 4));
            sim.add_master(master(3, 3, baud, mixed, 4));
            sim.enqueue(ms(1_000), 0, 7, &npdu(true, 0x73));
            sim.run_until(ms(2_000));
            let index = sim
                .frames()
                .iter()
                .position(|frame| frame.is(FrameType::BACNET_DATA_EXPECTING_REPLY, 0, 7))
                .unwrap();
            let request = &sim.frames()[index];
            let pass = &sim.frames()[index + 1];
            assert!(pass.is(FrameType::TOKEN, 0, 3));
            assert_eq!(pass.start, request.end + millis(300));
            sim.assert_no_violations();
        }
    }
}
