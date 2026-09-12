//! Independent raw-wire node admission vectors for Advertisement (0x04) and
//! Advertisement-Solicitation (0x05); no hub fallback involved.
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

fn nak(raw: u8, id: u16, dest: Option<Vmac>, code: u16, marker: u8) -> Vec<u8> {
    let [hi, lo] = code.to_be_bytes();
    wire(0, id, None, dest, 0, &[raw, 1, marker, 0, 7, hi, lo])
}

fn valid_advertisement() -> Vec<u8> {
    vec![1, 1, 0x05, 0xC4, 0x05, 0xC4]
}

async fn recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn barrier(hub: &LoopbackWebSocket) {
    // An invalid known control is an ordered, non-activity barrier, not a valid
    // sentinel that could conceal unintended activity from advertisement traffic.
    hub.send(&[0x0A, 0, 0x44, 0x55, 0x42]).await.unwrap();
    assert_eq!(recv(hub).await, [0, 0, 0x44, 0x55, 0x0A, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn advertisement_valid_shapes_are_consumed_silently_without_npdu() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    for status in [0, 1, 2] {
        for direct in [0, 1] {
            let mut payload = vec![status, direct, 0, 0, 0, 0];
            payload[2..4].copy_from_slice(&0u16.to_be_bytes());
            payload[4..6].copy_from_slice(&65535u16.to_be_bytes());
            for source in [None, Some([0x22; 6])] {
                hub.send(&wire(4, 0x2233, source, None, 0, &payload))
                    .await
                    .unwrap();
                barrier(&hub).await;
                assert!(rx.try_recv().is_err());
            }
        }
    }
    for source in [None, Some([0x22; 6])] {
        hub.send(&wire(5, 0x2234, source, None, 0, &[]))
            .await
            .unwrap();
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    // Non-MU destination options on valid shapes stay silent here.
    let mut optioned = vec![0x1F];
    optioned.extend_from_slice(&valid_advertisement());
    hub.send(&wire(4, 0x2235, None, None, 2, &optioned))
        .await
        .unwrap();
    barrier(&hub).await;
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn advertisement_negative_matrix_nak_vs_silence() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    // (function, body, expected NAK code, MU marker)
    let mut faults: Vec<(u8, Vec<u8>, u16, u8)> = Vec::new();
    faults.push((4, vec![], 149, 0));
    for len in [1, 3, 5] {
        faults.push((4, vec![1; len], 147, 0));
    }
    for len in [7, 8] {
        faults.push((4, vec![1; len], 7, 0));
    }
    let mut bad_status = valid_advertisement();
    bad_status[0] = 3;
    faults.push((4, bad_status, 80, 0));
    let mut bad_direct = valid_advertisement();
    bad_direct[1] = 2;
    faults.push((4, bad_direct, 80, 0));
    faults.push((4, [valid_advertisement(), vec![0xFF]].concat(), 7, 0));
    faults.push((5, vec![0x00], 7, 0));
    faults.push((5, valid_advertisement(), 7, 0));
    for (function, body, code, _marker) in faults {
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(function, 0x2233, source, None, 0, &body);
            hub.send(&bytes).await.unwrap();
            assert_eq!(
                recv(&hub).await,
                nak(function, 0x2233, source, code, 0),
                "function {function} source {source:02x?} body {body:02x?}"
            );
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Data Options are out of range even around otherwise valid payloads.
    for function in [4, 5] {
        let payload = if function == 4 {
            valid_advertisement()
        } else {
            vec![]
        };
        let mut body = vec![0x01];
        body.extend_from_slice(&payload);
        for source in [None, Some([0x22; 6])] {
            hub.send(&wire(function, 0x2233, source, None, 1, &body))
                .await
                .unwrap();
            assert_eq!(
                recv(&hub).await,
                nak(function, 0x2233, source, 80, 0),
                "function {function} data options source {source:02x?}"
            );
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Must-Understand destination option faults carry the wire marker.
    for function in [4, 5] {
        let payload = if function == 4 {
            valid_advertisement()
        } else {
            vec![]
        };
        let mut body = vec![0x42];
        body.extend_from_slice(&payload);
        for source in [None, Some([0x22; 6])] {
            hub.send(&wire(function, 0x2234, source, None, 2, &body))
                .await
                .unwrap();
            assert_eq!(
                recv(&hub).await,
                nak(function, 0x2234, source, 146, 0x42),
                "function {function} MU source {source:02x?}"
            );
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Addressed, broadcast, and reserved-origin envelopes stay silent for
    // both valid and forbidden shapes without spending a rejection budget.
    let silent_bodies: Vec<(u8, Vec<u8>)> = vec![
        (4, valid_advertisement()),
        (4, vec![]),
        (4, vec![9, 9, 0, 1, 0, 1]),
        (5, vec![]),
        (5, vec![0x42]),
    ];
    for (function, body) in silent_bodies {
        for (source, dest) in [
            (None, Some([0x44; 6])),
            (Some([0x22; 6]), Some([0x44; 6])),
            (None, Some(BROADCAST_VMAC)),
            (Some([0; 6]), None),
            (Some(BROADCAST_VMAC), None),
            (Some([0; 6]), Some([0x44; 6])),
        ] {
            hub.send(&wire(function, 0x2235, source, dest, 0, &body))
                .await
                .unwrap();
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn advertisement_silence_and_known_codes_never_consult_expired_budget() {
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
    // Forbidden local shapes must consult the budget (expiry surfaces).
    let nak_cases: Vec<(u8, Vec<u8>)> = vec![
        (4, vec![]),
        (4, vec![1, 1, 0]),
        (4, vec![1; 7]),
        (4, vec![3, 1, 0, 5, 0, 5]),
        (5, vec![0x00]),
    ];
    for (function, body) in nak_cases {
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(function, 0, source, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Err(RejectionExpired),
                "function {function} source {source:02x?}"
            );
        }
    }
    // MU faults also consult the budget when the marker is recoverable.
    for function in [4, 5] {
        let payload = if function == 4 {
            valid_advertisement()
        } else {
            vec![]
        };
        let mut body = vec![0x42];
        body.extend_from_slice(&payload);
        let bytes = wire(function, 0, None, None, 2, &body);
        let msg = decode_sc_message(&bytes).unwrap();
        assert_eq!(
            reject(&msg, &bytes, &NoIo, expired).await,
            Err(RejectionExpired),
            "function {function} MU"
        );
    }
    // Silent envelopes and valid shapes never consult the budget.
    for (function, body) in [(4, valid_advertisement()), (5, vec![])] {
        for (source, dest) in [
            (None, Some([0x44; 6])),
            (None, Some(BROADCAST_VMAC)),
            (Some([0; 6]), None),
            (Some(BROADCAST_VMAC), None),
        ] {
            let bytes = wire(function, 0, source, dest, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Ok(true),
                "silent function {function}"
            );
        }
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(function, 0, source, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Ok(false),
                "valid function {function}"
            );
        }
    }
}

#[test]
fn advertisement_direct_connection_is_pure_in_all_states_and_codec_is_permissive() {
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
        for function in [4, 5] {
            for dest in [None, Some(BROADCAST_VMAC), Some([1; 6])] {
                for body in [&[][..], &valid_advertisement()[..], &[9, 9, 0, 1, 0, 1][..]] {
                    let bytes = wire(function, u16::MAX, Some([0x22; 6]), dest, 0, body);
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
}

#[tokio::test]
async fn advertisement_malformed_generic_wire_is_still_silent() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    for bytes in [
        vec![0x04],
        vec![0x04, 0x80, 0, 1],
        vec![0x04, 8, 0, 1],
        vec![0x04, 4, 0, 1],
        vec![0x04, 2, 0, 1, 0],
        vec![0x05, 1, 0, 1, 0x7E, 0, 1],
    ] {
        assert!(decode_sc_message(&bytes).is_err());
        hub.send(&bytes).await.unwrap();
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn advertisement_result_for_keeps_existing_parse_and_fatal_policy() {
    for raw in [4, 5] {
        for (body, fatal) in [
            (vec![raw, 0], false),
            (vec![raw, 1, 0, 0, 7, 0, 150], true),
            (vec![raw, 1], true),
        ] {
            let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
            let bytes = wire(0, 0x2233, None, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(msg.function, ScFunction::Result);
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
async fn advertisement_valid_traffic_keeps_heartbeat_and_npdu_interop() {
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
    // Valid advertisements and solicitations are consumed without replies.
    for _ in 0..4 {
        hub.send(&wire(4, 0, None, None, 0, &valid_advertisement()))
            .await
            .unwrap();
        hub.send(&wire(5, 1, Some([0x22; 6]), None, 0, &[]))
            .await
            .unwrap();
    }
    // No rejection reply may precede the next liveness probe; accepted
    // advertisement traffic keeps the heartbeat budget alive like NPDUs.
    let next = timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    assert!(rx.try_recv().is_err());
    // Positive controls only AFTER the advertisement window.
    hub.send(&[0x0B, 0, next[2], next[3]]).await.unwrap();
    hub.send(&wire(1, 0, Some([0x22; 6]), None, 0, &[1, 0, 0x30]))
        .await
        .unwrap();
    let npdu = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    transport.stop().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), &[1, 0, 0x30]);
}
