//! Local admission policy, not a universal AB.6.3 invalid-frame rule.
//! Real std::Instant timing; these tests require progressing WebSocket writes.

use crate::sc::heartbeat_validation_tests::start_timed;
use crate::sc::*;
use tokio::time::timeout;

// Independent wire vectors: MU, More Options, Header Data (including an
// explicitly present empty field), and a later MU after an ignored option.
const OPTIONS: &[(&[u8], u8)] = &[
    (&[0x42], 0x42),
    (&[0xC2, 0x1F], 0xC2),
    (&[0x62, 0, 0], 0x62),
    (&[0xE2, 0, 0, 0x5F], 0xE2),
    (&[0xE2, 0, 2, 0xAA, 0xBB, 0x1F], 0xE2),
    (&[0x9E, 0x7F, 0, 1, 0xCC], 0x7F),
];

fn wire(options: &[u8], broadcast: bool) -> Vec<u8> {
    let mut wire = vec![1, if broadcast { 0x0E } else { 0x0A }, 0x22, 0x33];
    wire.extend_from_slice(&[0x22; 6]); // nonreserved originating node
    if broadcast {
        wire.extend_from_slice(&[0xFF; 6]);
    }
    wire.extend_from_slice(options);
    wire.extend_from_slice(&[1, 0, 0x30]);
    wire
}

fn nak(marker: u8) -> Vec<u8> {
    let mut nak = vec![0, 4, 0x22, 0x33];
    nak.extend_from_slice(&[0x22; 6]);
    nak.extend_from_slice(&[1, 1, marker, 0, 7, 0, 0x92]);
    nak
}

async fn recv(ws: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(2), ws.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn barrier(ws: &LoopbackWebSocket) {
    // Malformed control is rejected BEFORE activity accounting. FIFO response
    // proves all preceding silent discards were processed without a valid sentinel.
    ws.send(&[0x0A, 0, 0x33, 0x44, 0x42]).await.unwrap();
    assert_eq!(recv(ws).await, [0, 0, 0x33, 0x44, 0x0A, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn mu_liveness_rejected_burst_preserves_pending_probe_and_matching_ack() {
    let (mut transport, mut rx, ws) = start_timed().await;
    let probe = recv(&ws).await;
    assert_eq!(probe[0], 0x0A);
    for &(options, marker) in OPTIONS {
        ws.send(&wire(options, false)).await.unwrap();
        assert_eq!(recv(&ws).await, nak(marker));
        ws.send(&wire(options, true)).await.unwrap();
        barrier(&ws).await;
        assert!(rx.try_recv().is_err(), "MU-rejected NPDU was delivered");
    }
    let unexpected = timeout(Duration::from_millis(350), ws.recv()).await;
    if unexpected.is_ok() {
        transport.stop().await.unwrap();
    }
    assert!(
        unexpected.is_err(),
        "MU rejection cleared the pending heartbeat and reseeded a probe: {unexpected:?}"
    );

    // No valid NPDU masks ACK matching: a new probe proves the ORIGINAL ID was
    // still pending and its ACK alone restored ordinary activity accounting.
    ws.send(&[0x0B, 0, probe[2], probe[3]]).await.unwrap();
    let next = recv(&ws).await;
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Connected
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn mu_liveness_rejected_traffic_keeps_original_idle_timeout_and_reconnects() {
    let (client, ws) = LoopbackWebSocket::pair();
    let (redial_tx, mut redial_rx) = mpsc::unbounded_channel();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(100, 1000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100,
            max_retries: 1,
        })
        .with_connector(move || {
            let (client, hub) = LoopbackWebSocket::pair();
            redial_tx.send(hub).unwrap();
            Box::pin(async { Ok(client) })
        });
    let (rx, ()) = tokio::join!(transport.start(), super::hub_accept(&ws, [0x10; 6]));
    let mut rx = rx.unwrap();
    let mut states = transport.connection_state_changes();
    let started = Instant::now();
    let mut next = 0;
    let mut probes = 0;
    let mut naks = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    // No valid sentinel/ACK during this window. Drain NAKs so the send path
    // progresses; std::Instant is deliberately NOT replaced by Tokio advance.
    let expired = timeout(Duration::from_millis(1700), async {
        loop {
            tokio::select! {
                changed = states.changed() => {
                    changed.unwrap();
                    if *states.borrow_and_update() == ScConnectionState::Disconnected { break; }
                }
                _ = interval.tick() => {
                    ws.send(&wire(OPTIONS[(next / 2) % OPTIONS.len()].0, next % 2 == 1)).await.unwrap();
                    next += 1;
                }
                response = ws.recv() => {
                    let response = response.unwrap();
                    if response[0] == 0x0A {
                        probes += 1;
                        assert!(started.elapsed() < Duration::from_millis(500), "MU traffic postponed the original idle probe");
                    } else {
                        assert!(OPTIONS.iter().any(|&(_, marker)| response == nak(marker)));
                        naks += 1;
                    }
                }
            }
        }
    }).await;
    if expired.is_err() {
        transport.stop().await.unwrap();
    }
    expired.expect("MU-rejected traffic postponed the original receive timeout");
    assert!(started.elapsed() >= Duration::from_millis(1000));
    assert!(next >= 2 * OPTIONS.len());
    assert!(naks >= OPTIONS.len());
    assert_eq!(probes, 1, "MU rejection cleared the pending probe");
    assert!(rx.try_recv().is_err());

    let restored_ws = timeout(Duration::from_secs(2), redial_rx.recv())
        .await
        .unwrap()
        .unwrap();
    timeout(
        Duration::from_secs(2),
        super::hub_accept(&restored_ws, [0x11; 6]),
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(2), async {
        while *states.borrow_and_update() != ScConnectionState::Connected {
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(
        transport.connection().unwrap().lock().await.hub_vmac,
        Some([0x11; 6])
    );
    assert!(redial_rx.try_recv().is_err(), "unexpected extra reconnect");
    restored_ws.send(&wire(&[0x1E], false)).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap()
            .npdu
            .as_ref(),
        &[1, 0, 0x30]
    );
    let probe = recv(&restored_ws).await;
    assert_eq!(probe[0], 0x0A);
    restored_ws
        .send(&[0x0B, 0, probe[2], probe[3]])
        .await
        .unwrap();
    barrier(&restored_ws).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn mu_liveness_valid_npdu_data_options_and_heartbeat_request_restore_activity() {
    let (mut transport, mut rx, ws) = start_timed().await;
    for heartbeat_request in [false, true] {
        let probe = recv(&ws).await;
        assert_eq!(probe[0], 0x0A);
        ws.send(&wire(&[0x62, 0, 0], false)).await.unwrap();
        assert_eq!(recv(&ws).await, nak(0x62));
        // Keep accepted traffic flowing longer than the original 1s timeout.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(1200) {
            if heartbeat_request {
                ws.send(&[0x0A, 0, 0x66, 0x77]).await.unwrap();
                assert_eq!(recv(&ws).await, [0x0B, 0, 0x66, 0x77]);
            } else {
                // MU-clear Destination Option followed by MU Data Option with
                // the SAME type: the two lists must not be conflated.
                let mut accepted = wire(&[0x22, 0, 0, 0x62, 0, 1, 0xAA], false);
                accepted[1] |= 1;
                ws.send(&accepted).await.unwrap();
                let received = timeout(Duration::from_secs(1), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(received.npdu.as_ref(), &[1, 0, 0x30]);
                assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
                assert!(!received.link_layer_group);
                assert_eq!(
                    received.data_attributes,
                    vec![DataAttribute {
                        option_type: 2,
                        must_understand: true,
                        data: vec![0xAA]
                    }]
                );
            }
            assert!(timeout(Duration::from_millis(30), ws.recv()).await.is_err());
            assert_eq!(
                *transport.connection_state_changes().borrow(),
                ScConnectionState::Connected
            );
        }
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn mu_liveness_source_and_control_admission_still_precede_mu() {
    let (mut transport, mut rx, ws) = super::start_transport().await;
    for broadcast in [false, true] {
        for source in [None, Some([0; 6]), Some([0xFF; 6])] {
            let mut invalid = wire(&[0xE2, 0, 0, 0x1F], broadcast);
            if let Some(source) = source {
                invalid[4..10].copy_from_slice(&source);
            } else {
                invalid[1] &= !8;
                invalid.drain(4..10);
            }
            ws.send(&invalid).await.unwrap();
            if source.is_none() && !broadcast {
                assert_eq!(recv(&ws).await, [0, 0, 0x22, 0x33, 1, 1, 0, 0, 7, 0, 0x50]);
            }
            barrier(&ws).await;
        }
    }
    // Illegal heartbeat payload wins over its unsupported MU Destination Option.
    ws.send(&[0x0A, 2, 0x22, 0x33, 0x62, 0, 0, 0x42])
        .await
        .unwrap();
    assert_eq!(recv(&ws).await, [0, 0, 0x22, 0x33, 0x0A, 1, 0, 0, 7, 0, 7]);
    barrier(&ws).await;
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}
