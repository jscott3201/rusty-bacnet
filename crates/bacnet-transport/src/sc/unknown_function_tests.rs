//! Independent raw-wire node admission vectors; no native hub fallback involved.
use super::*;
use tokio::time::timeout;

fn wire(
    raw: u8,
    id: u16,
    source: Option<Vmac>,
    dest: Option<Vmac>,
    flags: u8,
    body: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![
        raw,
        flags | (u8::from(source.is_some()) * 8) | (u8::from(dest.is_some()) * 4),
    ];
    bytes.extend_from_slice(&id.to_be_bytes());
    if let Some(source) = source {
        bytes.extend_from_slice(&source);
    }
    if let Some(dest) = dest {
        bytes.extend_from_slice(&dest);
    }
    bytes.extend_from_slice(body);
    bytes
}

fn nak(raw: u8, id: u16, dest: Option<Vmac>) -> Vec<u8> {
    wire(0, id, None, dest, 0, &[raw, 1, 0, 0, 7, 0, 0x8F])
}

async fn recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn barrier(hub: &LoopbackWebSocket) {
    // An invalid known control is an ordered, non-activity barrier, not a valid
    // sentinel that could conceal unintended activity from unknown traffic.
    hub.send(&[0x0A, 0, 0x44, 0x55, 0x42]).await.unwrap();
    assert_eq!(recv(hub).await, [0, 0, 0x44, 0x55, 0x0A, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn unknown_function_node_returns_connection_local_nak() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    hub.send(&[0x42, 0, 0x22, 0x33]).await.unwrap();
    let response = timeout(Duration::from_millis(150), hub.recv()).await;
    // Stop joins the receive task even on the expected pre-fix timeout.
    transport.stop().await.unwrap();
    assert_eq!(
        response.expect("missing unknown-function NAK").unwrap(),
        [0, 0, 0x22, 0x33, 0x42, 1, 0, 0, 7, 0, 0x8F]
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn unknown_function_all_wire_codes_and_address_option_payload_matrix() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    for raw in 0x0D..=0xFF {
        let bytes = wire(raw, 0x2233, None, None, 0, &[]);
        assert_eq!(
            decode_sc_message(&bytes).unwrap().function,
            ScFunction::Unknown(raw)
        );
        hub.send(&bytes).await.unwrap();
        assert_eq!(recv(&hub).await, nak(raw, 0x2233, None));
    }
    for raw in [0x0D, 0x42, 0xFF] {
        for id in [0, u16::MAX] {
            for source in [None, Some([0x22; 6]), Some([0; 6]), Some(BROADCAST_VMAC)] {
                for dest in [
                    None,
                    Some(BROADCAST_VMAC),
                    Some([1; 6]),
                    Some([0x44; 6]),
                    Some([0; 6]),
                ] {
                    for (flags, options) in [
                        (0, vec![]),
                        (2, vec![0x42]),                   // MU Destination Option
                        (1, vec![0xFF, 0, 1, 0xBB, 0x1E]), // MU Data + MoreOptions
                        (3, vec![0xE2, 0, 0, 0x1F, 0x7E, 0, 0]), // empty HeaderData
                        (3, vec![0xE2, 0, 1, 0xAA, 0x1F, 0x7E, 0, 1, 0xBB]),
                    ] {
                        for payload in [&[][..], &[1, 0, 0x30][..]] {
                            let mut body = options.clone();
                            body.extend_from_slice(payload);
                            let bytes = wire(raw, id, source, dest, flags, &body);
                            let decoded = decode_sc_message(&bytes).unwrap();
                            assert_eq!(decoded.payload.as_ref(), payload);
                            hub.send(&bytes).await.unwrap();
                            if dest.is_none()
                                && source != Some([0; 6])
                                && source != Some(BROADCAST_VMAC)
                            {
                                assert_eq!(recv(&hub).await, nak(raw, id, source));
                            }
                            barrier(&hub).await; // also rules out duplicate NAKs
                            assert!(rx.try_recv().is_err());
                        }
                    }
                }
            }
        }
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn unknown_function_silence_and_known_codes_never_consult_expired_budget() {
    use super::rejection::{reject, RejectionBudget, RejectionExpired};
    struct NoIo;
    impl WebSocketPort for NoIo {
        async fn send(&self, _: &[u8]) -> Result<(), Error> {
            panic!("unexpected send")
        }
        async fn recv(&self) -> Result<Vec<u8>, Error> {
            panic!("unexpected receive")
        }
    }
    let expired = RejectionBudget::new(Instant::now() - Duration::from_secs(1), 1);
    for raw in [0x0D, 0x42, 0xFF] {
        for source in [None, Some([0x22; 6]), Some([0; 6]), Some(BROADCAST_VMAC)] {
            for dest in [
                None,
                Some(BROADCAST_VMAC),
                Some([1; 6]),
                Some([0x44; 6]),
                Some([0; 6]),
            ] {
                let bytes = wire(raw, 0, source, dest, 3, &[0xE2, 0, 0, 0x1F, 0x7E, 0, 0]);
                let msg = decode_sc_message(&bytes).unwrap();
                let expected =
                    if dest.is_none() && source != Some([0; 6]) && source != Some(BROADCAST_VMAC) {
                        Err(RejectionExpired)
                    } else {
                        Ok(true)
                    };
                assert_eq!(reject(&msg, &bytes, &NoIo, expired).await, expected);
            }
        }
    }
    for raw in 0..=0x0C {
        let bytes = if raw == 1 {
            wire(raw, 0, Some([0x22; 6]), None, 0, &[0x42])
        } else {
            wire(raw, 0, None, None, 0, &[])
        };
        let msg = decode_sc_message(&bytes).unwrap();
        assert!(!matches!(msg.function, ScFunction::Unknown(_)));
        assert_eq!(
            super::unknown_function::reject(&msg, &NoIo, expired).await,
            Ok(false)
        );
        // Advertisement (0x04) and Solicitation (0x05) own a later gate that
        // consults the budget for forbidden local shapes; an empty
        // Advertisement is such a shape while an empty Solicitation is valid.
        let expected = match raw {
            4 => Err(RejectionExpired),
            _ => Ok(false),
        };
        assert_eq!(reject(&msg, &bytes, &NoIo, expired).await, expected);
    }
}

#[test]
fn unknown_function_direct_connection_is_pure_in_all_states_and_codec_is_permissive() {
    for state in [
        ScConnectionState::Disconnected,
        ScConnectionState::Connecting,
        ScConnectionState::Connected,
        ScConnectionState::Disconnecting,
    ] {
        let mut conn = ScConnection::new([1; 6], [1; 16]);
        conn.build_connect_request();
        conn.state = state;
        conn.hub_vmac = Some([0x10; 6]);
        conn.hub_device_uuid = Some([0x33; 16]);
        conn.disconnect_ack_to_send = Some(conn.build_heartbeat_ack(0x4455));
        let before = conn.clone();
        for raw in 0x0D..=0xFF {
            for dest in [None, Some(BROADCAST_VMAC), Some([1; 6])] {
                let bytes = wire(
                    raw,
                    u16::MAX,
                    Some([0x22; 6]),
                    dest,
                    3,
                    &[0x42, 0x7E, 0, 1, 0xBB, 0x42],
                );
                let msg = decode_sc_message(&bytes).unwrap();
                let mut encoded = BytesMut::new();
                encode_sc_message(&mut encoded, &msg);
                assert_eq!(encoded.as_ref(), bytes);
                assert!(conn.handle_received(&msg).is_none());
                assert_eq!(conn.state, before.state);
                assert_eq!(conn.local_vmac, before.local_vmac);
                assert_eq!(conn.device_uuid, before.device_uuid);
                assert_eq!(conn.hub_vmac, before.hub_vmac);
                assert_eq!(conn.hub_device_uuid, before.hub_device_uuid);
                assert_eq!(conn.max_bvlc_length, before.max_bvlc_length);
                assert_eq!(conn.max_apdu_length, before.max_apdu_length);
                assert_eq!(conn.hub_max_bvlc_length, before.hub_max_bvlc_length);
                assert_eq!(conn.hub_max_apdu_length, before.hub_max_apdu_length);
                assert_eq!(conn.next_message_id, before.next_message_id);
                assert_eq!(
                    conn.pending_connect_message_id,
                    before.pending_connect_message_id
                );
                assert_eq!(conn.connect_retry_allowed, before.connect_retry_allowed);
                assert_eq!(conn.disconnect_ack_to_send, before.disconnect_ack_to_send);
            }
        }
    }
}

#[tokio::test]
async fn unknown_function_malformed_generic_wire_is_still_silent() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    for bytes in [
        vec![0x42],
        vec![0x42, 0x80, 0, 1],
        vec![0x42, 8, 0, 1],
        vec![0x42, 4, 0, 1],
        vec![0x42, 2, 0, 1, 0],
        vec![0x42, 1, 0, 1, 0x7E, 0, 1],
    ] {
        assert!(decode_sc_message(&bytes).is_err());
        hub.send(&bytes).await.unwrap();
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn unknown_function_result_for_unknown_keeps_existing_parse_and_fatal_policy() {
    for raw in [0x0D, 0x42, 0xFF] {
        for (body, fatal) in [
            (vec![raw, 0], false),
            (vec![raw, 1, 0, 0, 7, 0, 0x8F], true),
            (vec![raw, 1], true),
        ] {
            let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
            let bytes = wire(0, 0x2233, None, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(msg.function, ScFunction::Result);
            if body.len() == 2 && body[1] == 1 {
                assert!(decode_sc_bvlc_result(&msg).is_err());
            } else {
                use crate::sc_frame::ScBvlcResult;
                let result_for = match decode_sc_bvlc_result(&msg).unwrap() {
                    ScBvlcResult::Ack { result_for } | ScBvlcResult::Nak { result_for, .. } => {
                        result_for
                    }
                };
                assert_eq!(result_for, ScFunction::Unknown(raw));
            }
            hub.send(&bytes).await.unwrap();
            let mut states = transport.connection_state_changes();
            let finished = if fatal {
                timeout(Duration::from_secs(1), async {
                    while *states.borrow_and_update() != ScConnectionState::Disconnected {
                        states.changed().await.unwrap();
                    }
                })
                .await
            } else {
                barrier(&hub).await;
                assert_eq!(*states.borrow(), ScConnectionState::Connected);
                Ok(())
            };
            let extra = timeout(Duration::from_millis(20), hub.recv()).await;
            transport.stop().await.unwrap();
            finished.unwrap();
            assert!(extra.is_err(), "response on Result");
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn unknown_function_during_handshake_is_ignored_without_nak() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6]).with_device_uuid([1; 16]);
    let peer = async {
        let request = recv(&hub).await;
        assert_eq!(request[0], 6);
        for raw in [0x0D, 0x42, 0xFF] {
            hub.send(&wire(raw, 0x2233, None, None, 2, &[0x42]))
                .await
                .unwrap();
        }
        let silent = timeout(Duration::from_millis(30), hub.recv())
            .await
            .is_err();
        let mut accept = vec![7, 0, request[2], request[3]];
        accept.extend_from_slice(&[0x10; 6]);
        accept.extend_from_slice(&[0x33; 16]);
        accept.extend_from_slice(&[5, 0xC4, 5, 0xC4]);
        hub.send(&accept).await.unwrap();
        silent
    };
    let result = timeout(Duration::from_secs(2), async {
        tokio::join!(transport.start(), peer)
    })
    .await;
    transport.stop().await.unwrap();
    let (rx, silent) = result.unwrap();
    assert!(rx.is_ok());
    assert!(silent, "handshake unexpectedly rejected unknown function");
}

#[tokio::test]
async fn unknown_function_burst_before_and_after_probe_cannot_refresh_real_clock() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(120, 700);
    let (rx, ()) = tokio::join!(
        transport.start(),
        super::data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    let mut rx = rx.unwrap();
    let started = Instant::now();
    let mut states = transport.connection_state_changes();
    let mut interval = tokio::time::interval(Duration::from_millis(15));
    let mut sent = 0;
    let mut before_probe = 0;
    let mut probes = 0;
    let expired = timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                changed = states.changed() => {
                    changed.unwrap();
                    if *states.borrow_and_update() == ScConnectionState::Disconnected { break; }
                }
                _ = interval.tick() => {
                    let dest = [None, Some(BROADCAST_VMAC), Some([1; 6])][sent % 3];
                    hub.send(&wire(0x42, 0x2233, None, dest, 2, &[0x42])).await.unwrap();
                    sent += 1;
                    if probes == 0 { before_probe += 1; }
                }
                response = hub.recv() => {
                    let response = response.unwrap();
                    if response[0] == 0x0A { probes += 1; }
                    else { assert_eq!(response, nak(0x42, 0x2233, None)); }
                }
            }
        }
    })
    .await;
    let elapsed = started.elapsed();
    transport.stop().await.unwrap();
    expired.expect("unknown traffic refreshed accepted activity");
    assert_eq!(probes, 1, "unknown traffic cleared the outstanding probe");
    assert!(before_probe >= 2 && sent > before_probe + 2);
    assert!(elapsed >= Duration::from_millis(680));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn unknown_function_original_ack_and_valid_npdu_resume_activity() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(80, 700);
    let (rx, ()) = tokio::join!(
        transport.start(),
        super::data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    let mut rx = rx.unwrap();
    let probe = recv(&hub).await;
    assert_eq!(probe[0], 0x0A);
    for _ in 0..4 {
        hub.send(&wire(0xFF, 0, None, None, 0, &[])).await.unwrap();
        assert_eq!(recv(&hub).await, nak(0xFF, 0, None));
    }
    assert!(timeout(Duration::from_millis(100), hub.recv())
        .await
        .is_err());
    assert!(rx.try_recv().is_err());
    // Positive controls only AFTER the unknown accounting window.
    hub.send(&[0x0B, 0, probe[2], probe[3]]).await.unwrap();
    let next = recv(&hub).await;
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    hub.send(&wire(1, 0, Some([0x22; 6]), None, 0, &[1, 0, 0x30]))
        .await
        .unwrap();
    let npdu = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let after_npdu = recv(&hub).await;
    transport.stop().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), &[1, 0, 0x30]);
    assert_eq!(after_npdu[0], 0x0A);
    assert_ne!(&after_npdu[2..4], &next[2..4]);
}
