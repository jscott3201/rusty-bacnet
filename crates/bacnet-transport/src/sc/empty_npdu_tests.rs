//! Independently encoded AB.2.5 / AB.3.1.5 zero-payload admission vectors.
use super::data_attribute_tests::start_transport;
use super::*;
use tokio::time::timeout;

fn wire(payload: &[u8]) -> Vec<u8> {
    let mut wire = vec![1, 8, 0x22, 0x33];
    wire.extend_from_slice(&[0x22; 6]);
    wire.extend_from_slice(payload);
    wire
}

#[tokio::test]
async fn empty_npdu_node_does_not_deliver_before_positive_sentinel() {
    let (mut transport, mut rx, hub) = start_transport().await;
    hub.send(&wire(&[])).await.unwrap();
    hub.send(&wire(&[1, 0, 0x30])).await.unwrap();
    let first = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    transport.stop().await.unwrap();
    assert_eq!(
        first.npdu.as_ref(),
        &[1, 0, 0x30],
        "empty NPDU was delivered"
    );
}

fn routed(source: Option<Vmac>, destination: Option<Vmac>, options: u8, body: &[u8]) -> Vec<u8> {
    let flags = u8::from(source.is_some()) * 8 + u8::from(destination.is_some()) * 4 + options;
    let mut wire = vec![1, flags, 0x22, 0x33];
    if let Some(source) = source {
        wire.extend_from_slice(&source);
    }
    if let Some(destination) = destination {
        wire.extend_from_slice(&destination);
    }
    wire.extend_from_slice(body);
    wire
}

async fn recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap()
}

fn nak(destination: Option<Vmac>, marker: u8, code: u8) -> Vec<u8> {
    let mut wire = vec![0, u8::from(destination.is_some()) * 4, 0x22, 0x33];
    if let Some(destination) = destination {
        wire.extend_from_slice(&destination);
    }
    wire.extend_from_slice(&[1, 1, marker, 0, 7, 0, code]);
    wire
}

#[tokio::test]
async fn empty_npdu_node_wire_source_mu_precedence_and_option_boundaries() {
    let (mut transport, mut rx, hub) = start_transport().await;
    for source in [None, Some([0; 6]), Some(BROADCAST_VMAC), Some([0x22; 6])] {
        for destination in [None, Some(BROADCAST_VMAC), Some([1; 6]), Some([0x44; 6])] {
            // HeaderData (including zero length), MoreOptions, and Data Options
            // occupy bytes but do not supply a payload. The MU marker is raw E2.
            for (flags, options, mu) in [
                (0, vec![], false),
                (2, vec![0xA2, 0, 1, 0xAA, 0x1F], false),
                (1, vec![0xFF, 0, 1, 0xBB, 0x1E], false),
                (3, vec![0xA2, 0, 0, 0x1F, 0x3E, 0, 1, 0xBB], false),
                (3, vec![0xE2, 0, 0, 0x1F, 0x3E, 0, 1, 0xBB], true),
            ] {
                let incoming = routed(source, destination, flags, &options);
                assert!(decode_sc_message(&incoming).unwrap().payload.is_empty());
                hub.send(&incoming).await.unwrap();
                let expected = if destination == Some(BROADCAST_VMAC) {
                    None
                } else if source.is_none() {
                    Some(nak(None, 0, 0x50))
                } else if source == Some([0; 6]) || source == Some(BROADCAST_VMAC) {
                    None
                } else if mu {
                    Some(nak(source, 0xE2, 0x92))
                } else if destination.is_none() {
                    Some(nak(source, 0, 0x95))
                } else {
                    None
                };
                if let Some(expected) = expected {
                    assert_eq!(recv(&hub).await, expected);
                }
                // Ordered non-activity barrier proves silence without racing the
                // receive loop; it also catches an extra or misaddressed NAK.
                hub.send(&[0x0A, 0, 0x44, 0x55, 0x42]).await.unwrap();
                assert_eq!(recv(&hub).await, [0, 0, 0x44, 0x55, 0x0A, 1, 0, 0, 7, 0, 7]);
                assert!(rx.try_recv().is_err());
            }
        }
    }
    // One byte is compatible at SC admission, not a claim of a valid upper NPDU.
    for payload in [&[0x42][..], &[1, 0, 0x10, 8][..]] {
        for destination in [None, Some(BROADCAST_VMAC)] {
            let mut body = vec![0xA2, 0, 0, 0x1F, 0x7E, 0, 1, 0xBB];
            body.extend_from_slice(payload);
            hub.send(&routed(Some([0x22; 6]), destination, 3, &body))
                .await
                .unwrap();
            let received = timeout(Duration::from_secs(1), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received.npdu.as_ref(), payload);
            assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
            assert_eq!(received.link_layer_group, destination.is_some());
            assert_eq!(
                received.data_attributes,
                vec![DataAttribute {
                    option_type: 30,
                    must_understand: true,
                    data: vec![0xBB],
                }]
            );
        }
    }
    transport.stop().await.unwrap();
}

fn unchanged(conn: &ScConnection, before: &ScConnection) {
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
    // Compare the queued response's wire image without requiring a new API trait.
    let encode = |msg: &Option<ScMessage>| {
        msg.as_ref().map(|msg| {
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, msg);
            buf
        })
    };
    assert_eq!(
        encode(&conn.disconnect_ack_to_send),
        encode(&before.disconnect_ack_to_send)
    );
}

#[test]
fn empty_npdu_direct_connection_is_pure_in_all_states_and_codec_stays_permissive() {
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
        for destination in [None, Some(BROADCAST_VMAC), Some([1; 6])] {
            let raw = routed(Some([0x22; 6]), destination, 0, &[]);
            let msg = decode_sc_message(&raw).unwrap();
            let mut encoded = BytesMut::new();
            encode_sc_message(&mut encoded, &msg);
            assert_eq!(encoded.as_ref(), raw);
            assert!(conn.handle_received(&msg).is_none());
            unchanged(&conn, &before);
        }
        let positive = decode_sc_message(&wire(&[0x42])).unwrap();
        assert_eq!(
            conn.handle_received(&positive).is_some(),
            state == ScConnectionState::Connected
        );
    }
}

#[tokio::test]
async fn empty_npdu_silent_and_other_empty_functions_never_consult_expired_budget() {
    use super::rejection::{reject, RejectionBudget};
    let (client, hub) = LoopbackWebSocket::pair();
    let budget = RejectionBudget::new(Instant::now() - Duration::from_secs(1), 1);
    for destination in [Some(BROADCAST_VMAC), Some([1; 6]), Some([0x44; 6])] {
        let raw = routed(Some([0x22; 6]), destination, 0, &[]);
        assert_eq!(
            reject(&decode_sc_message(&raw).unwrap(), &raw, &client, budget).await,
            Ok(true)
        );
    }
    for function in [2, 5, 8, 9, 10, 11] {
        let raw = [function, 0, 0x22, 0x33];
        let msg = decode_sc_message(&raw).unwrap();
        assert!(!crate::sc_frame::missing_npdu_payload(&msg));
        assert_eq!(reject(&msg, &raw, &client, budget).await, Ok(false));
    }
    assert!(timeout(Duration::from_millis(20), hub.recv())
        .await
        .is_err());
}

#[tokio::test]
async fn empty_npdu_burst_does_not_refresh_real_clock_or_clear_pending_probe() {
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
    let mut states = transport.connection_state_changes();
    let mut interval = tokio::time::interval(Duration::from_millis(15));
    let mut sent = 0;
    let mut extra_probes = 0;
    let expired = timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                changed = states.changed() => {
                    changed.unwrap();
                    if *states.borrow_and_update() == ScConnectionState::Disconnected { break; }
                }
                _ = interval.tick() => {
                    let destination = [None, Some(BROADCAST_VMAC), Some([1; 6])][sent % 3];
                    hub.send(&routed(Some([0x22; 6]), destination, 0, &[])).await.unwrap();
                    sent += 1;
                }
                response = hub.recv() => {
                    let response = response.unwrap();
                    if response[0] == 0x0A { extra_probes += 1; }
                    else { assert_eq!(response, nak(Some([0x22; 6]), 0, 0x95)); }
                }
            }
        }
    })
    .await;
    transport.stop().await.unwrap();
    expired.expect("empty burst refreshed activity");
    assert_eq!(
        extra_probes, 0,
        "empty traffic cleared the pending heartbeat"
    );
    assert!(sent > 3);
    assert!(rx.try_recv().is_err());
}
