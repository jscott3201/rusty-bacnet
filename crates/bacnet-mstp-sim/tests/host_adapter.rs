//! Host reads retain delivery time without changing physical timing oracles.
mod common;

use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::{Fault, HostSpec, MasterSpec, ScriptSend, Sim, SimConfig, ViolationKind};
use common::*;

/// Hosts, UART masters, standard frames and COBS frames share one wire.
#[test]
fn host_and_mixed_rings_deliver_without_echo_leaks() {
    for baud in [9_600, 38_400, 115_200] {
        for mixed in [false, true] {
            let mut sim = Sim::new(SimConfig::new(baud));
            for station in [0, 1] {
                let mut spec = MasterSpec::new(station, 1, baud).extended_frames();
                if !mixed || station == 0 {
                    spec = spec.host(HostSpec {
                        max_chunk: 7,
                        random_chunks: true,
                        echo_grace: Some(Duration::from_millis(40)),
                        ..HostSpec::default()
                    });
                    spec.adapter.echo = true;
                }
                sim.add_master(spec);
            }
            let extended = vec![0x11; 700];
            sim.enqueue(ms(800), 0, 1, &npdu(false, 0x42));
            sim.enqueue(ms(1_000), 1, 0, &extended);
            sim.run_until(ms(3_000));
            sim.assert_no_violations();
            assert!(sim
                .deliveries(1)
                .iter()
                .any(|d| d.data == npdu(false, 0x42)));
            assert!(sim.deliveries(0).iter().any(|d| d.data == extended));
            assert!(sim.host_stats(0).unwrap().reads > 0);
            assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
        }
    }
}

/// A 16 ms host delay cannot meet the wire's 15 ms usage obligation.
#[test]
fn excessive_host_latency_remains_a_physical_timing_failure() {
    let mut config = SimConfig::new(38_400);
    config.panic_on_violation = false;
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(1, 1, 38_400).host(HostSpec {
        receive_latency: Duration::from_millis(16),
        ..HostSpec::default()
    }));
    let poll = ScriptSend::frame(
        Duration::ZERO,
        bacnet_mstp_codec::FrameType::POLL_FOR_MASTER,
        1,
        0,
        &[],
    );
    sim.inject(
        ms(20),
        Fault::Noise {
            octets: poll.octets,
        },
    );
    sim.run_until(ms(100));
    assert!(
        sim.report()
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::UsageDelay),
        "{}",
        sim.dump(120)
    );
}
