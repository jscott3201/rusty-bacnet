//! Reads captured from actual bus-driven local returns cross the public port.

use super::*;
use crate::sim::host::delayed_echo_tests::request;
use crate::{ScriptSend, ViolationKind};

#[derive(Debug, Clone, Copy)]
enum BusCase {
    Grace,
    Residual,
    Probe,
    Expired,
    Missing,
    Peers,
}

async fn paired_bus(case: BusCase) {
    let (latency, delay, horizon) = match case {
        BusCase::Grace => (16, 57, 106),
        BusCase::Residual => (1, 80, 116),
        BusCase::Probe => (16, 61, 109),
        BusCase::Expired => (16, 62, 110),
        BusCase::Missing => (16, 61, 105),
        BusCase::Peers => (16, 48, 100),
    };
    let mut sim = request(latency, delay, 5);
    if matches!(case, BusCase::Missing) {
        sim.master_mut(0).unwrap().spec.adapter.echo = false;
    }
    let modeled_start = sim.now();
    sim.master_mut(0)
        .unwrap()
        .host
        .as_mut()
        .unwrap()
        .read_capture = Some(Vec::new());
    if matches!(case, BusCase::Peers) {
        for (at, data) in [(48, [1, 0, 0xA1]), (65, [1, 0, 0xA2])] {
            sim.schedule(
                ms(at),
                crate::event::Class::Scheduled,
                1u8,
                crate::sim::Event::ScriptSend {
                    node: 1,
                    send: ScriptSend::frame(
                        CoreDuration::ZERO,
                        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                        0,
                        1,
                        &data,
                    ),
                },
            );
        }
    }
    sim.run_until(ms(horizon));
    let reads = sim
        .master_mut(0)
        .unwrap()
        .host
        .as_mut()
        .unwrap()
        .read_capture
        .take()
        .unwrap();
    let modeled_frame = sim
        .frames()
        .iter()
        .find(|frame| frame.source() == Some(0))
        .unwrap();
    let modeled_counters = sim.core(0).unwrap().counters();
    let host = sim.host_stats(0).unwrap();
    let expected = (
        u64::from(host.echo_missing),
        u64::from(host.echo_mismatches),
        u64::from(host.echo_fragments_dropped),
        u64::from(modeled_counters.own_address_frames),
    );
    let payloads: Vec<_> = sim
        .deliveries(0)
        .iter()
        .map(|delivery| delivery.data.clone())
        .collect();
    assert_eq!(sim.report.host_transmits[0].completion, ms(100));
    assert_eq!(
        sim.report.host_transmits[0].notification,
        ms(if matches!(case, BusCase::Peers) {
            100
        } else {
            105
        })
    );

    let (input, rx) = mpsc::unbounded_channel();
    let (tx, mut writes) = mpsc::unbounded_channel();
    let mut transport = MstpTransport::new(
        FixtureSerial {
            input: Mutex::new(rx),
            remaining: StdMutex::new(Vec::new()),
            writes: tx,
            echo: Echo::Full { grace: GRACE },
            drain_duration: Duration::from_millis(if latency == 16 { 69 } else { 78 }),
        },
        MstpConfig {
            this_station: 0,
            max_master: 1,
            baud_rate: 10_000,
            transmit_latency_pad: Duration::ZERO,
            ..MstpConfig::default()
        },
    );
    let mut npdus = transport.start().await.unwrap();
    transport.send_unicast(&[1, 4, 0xC1], &[1]).await.unwrap();
    settle_task().await;
    input.send(encode(FrameType::TOKEN, 0, 1, &[])).unwrap();
    let (actual_start, actual_frame) =
        tokio::time::timeout(Duration::from_millis(100), writes.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        actual_frame, modeled_frame.octets,
        "{case:?}: same physical frame"
    );
    for (read_at, characters) in &reads {
        let offset = read_at.as_nanos() - modeled_start.as_nanos();
        tokio::time::sleep_until(actual_start + Duration::from_nanos(offset)).await;
        let bytes = characters
            .iter()
            .map(|received| match received.character {
                Character::Clean(byte) => byte,
                Character::Garbled => panic!("clean bus fixture"),
            })
            .collect();
        input.send(bytes).unwrap();
        settle_task().await;
    }
    let end_offset = (ms(horizon) - modeled_start).as_nanos();
    tokio::time::sleep_until(actual_start + Duration::from_nanos(end_offset)).await;
    settle_task().await;
    let counters = transport.diagnostics().snapshot();
    let actual = (
        counters.echo_missing,
        counters.echo_mismatches,
        counters.echo_fragments_dropped,
        counters.mac.own_address_frames,
    );
    let mut actual_payloads = Vec::new();
    while let Ok(npdu) = npdus.try_recv() {
        actual_payloads.push(npdu.npdu.to_vec());
    }
    transport.stop().await.unwrap();
    assert_eq!(actual, expected, "{case:?}: bus-derived read counters");
    assert_eq!(
        actual_payloads, payloads,
        "{case:?}: bus-derived payload order"
    );
    if matches!(case, BusCase::Expired) {
        assert_eq!(actual.3, 1);
        assert!(
            sim.report
                .violations
                .iter()
                .any(|v| v.kind == ViolationKind::CoreCounter)
        );
    } else {
        assert_eq!(actual.3, 0);
        sim.assert_no_violations();
    }
}

#[tokio::test(start_paused = true)]
async fn bus_generated_delayed_echo_reads_match_the_public_transport() {
    for case in [
        BusCase::Grace,
        BusCase::Residual,
        BusCase::Probe,
        BusCase::Expired,
        BusCase::Missing,
        BusCase::Peers,
    ] {
        paired_bus(case).await;
    }
}
