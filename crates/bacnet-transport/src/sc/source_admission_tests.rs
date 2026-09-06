use super::*;
use crate::sc_frame::ScOption;
use tokio::time::timeout;

use super::data_attribute_tests::{hub_accept, start_transport};

fn npdu(source: Option<Vmac>, destination: Option<Vmac>, must_understand: bool) -> ScMessage {
    ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: source,
        destination_vmac: destination,
        dest_options: if must_understand {
            vec![ScOption {
                option_type: 2,
                must_understand: true,
                data: Vec::new(),
            }]
        } else {
            Vec::new()
        },
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    }
}

async fn send(ws: &LoopbackWebSocket, msg: &ScMessage) {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, msg);
    ws.send(&buf).await.unwrap();
}

async fn recv(ws: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(1), ws.recv())
        .await
        .expect("timed out waiting for SC response")
        .unwrap()
}

async fn assert_valid_sentinel(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    ws: &LoopbackWebSocket,
    destination: Option<Vmac>,
) {
    let mut sentinel = npdu(Some([0x22; 6]), destination, false);
    sentinel.payload = Bytes::from_static(&[0x01, 0x00, 0x99]);
    sentinel.data_options = vec![ScOption {
        option_type: 31,
        must_understand: true,
        data: vec![0xAA],
    }];
    send(ws, &sentinel).await;
    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for valid sentinel")
        .expect("SC receive channel closed");
    assert_eq!(
        received.npdu, sentinel.payload,
        "invalid NPDU was delivered"
    );
    assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
    assert_eq!(
        received.link_layer_group,
        destination == Some(BROADCAST_VMAC)
    );
    assert_eq!(
        received.data_attributes,
        vec![DataAttribute {
            option_type: 31,
            must_understand: true,
            data: vec![0xAA],
        }]
    );
    assert!(rx.try_recv().is_err());
}

async fn assert_missing_source_unicast_rejected(must_understand: bool) {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    send(&ws_hub, &npdu(None, None, must_understand)).await;

    // Connection-local Result, echoed ID, no VMACs/options, marker zero,
    // COMMUNICATION / PARAMETER_OUT_OF_RANGE, and empty error detail.
    assert_eq!(
        recv(&ws_hub).await,
        [0x00, 0x00, 0x22, 0x33, 0x01, 0x01, 0x00, 0x00, 0x07, 0x00, 0x50]
    );
    assert_valid_sentinel(&mut rx, &ws_hub, None).await;
    assert!(timeout(Duration::from_millis(25), ws_hub.recv())
        .await
        .is_err());
    assert_eq!(
        transport.connection().unwrap().lock().await.state,
        ScConnectionState::Connected
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn hub_npdu_missing_source_precedes_destination_option_nak() {
    assert_missing_source_unicast_rejected(true).await;
}

#[tokio::test]
async fn hub_npdu_missing_source_unicast_returns_connection_local_nak() {
    assert_missing_source_unicast_rejected(false).await;
}

#[tokio::test]
async fn hub_npdu_missing_source_broadcast_is_silent_with_or_without_options() {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    for must_understand in [false, true] {
        send(&ws_hub, &npdu(None, Some(BROADCAST_VMAC), must_understand)).await;
        assert_valid_sentinel(&mut rx, &ws_hub, Some(BROADCAST_VMAC)).await;
        assert!(timeout(Duration::from_millis(25), ws_hub.recv())
            .await
            .is_err());
    }
    assert_eq!(
        transport.connection().unwrap().lock().await.state,
        ScConnectionState::Connected
    );
    transport.stop().await.unwrap();
}

async fn assert_silent_discard(source: Option<Vmac>, must_understand: bool) {
    let (mut transport, mut rx, ws_hub) = start_transport().await;
    for destination in [None, Some(BROADCAST_VMAC)] {
        send(&ws_hub, &npdu(source, destination, must_understand)).await;
        assert_valid_sentinel(&mut rx, &ws_hub, destination).await;
        assert!(
            timeout(Duration::from_millis(25), ws_hub.recv())
                .await
                .is_err(),
            "invalid source {source:?} caused a response"
        );
        assert_eq!(
            transport.connection().unwrap().lock().await.state,
            ScConnectionState::Connected
        );
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn hub_npdu_reserved_sources_are_discarded() {
    for source in [[0; 6], BROADCAST_VMAC] {
        assert_silent_discard(Some(source), false).await;
    }
}

#[tokio::test]
async fn hub_npdu_reserved_sources_suppress_destination_option_nak() {
    for source in [[0; 6], BROADCAST_VMAC] {
        assert_silent_discard(Some(source), true).await;
    }
}

#[test]
fn hub_connection_never_returns_missing_or_reserved_npdu_source() {
    let mut conn = ScConnection::new([0x01; 6], [0; 16]);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some([0x10; 6]);
    let before = conn.clone();
    for source in [None, Some([0; 6]), Some(BROADCAST_VMAC)] {
        for destination in [None, Some(BROADCAST_VMAC)] {
            assert!(conn
                .handle_received(&npdu(source, destination, false))
                .is_none());
            assert_eq!(conn.state, before.state);
            assert_eq!(conn.hub_vmac, before.hub_vmac);
            assert_eq!(conn.next_message_id, before.next_message_id);
            assert_eq!(conn.connect_retry_allowed, before.connect_retry_allowed);
            assert!(conn.disconnect_ack_to_send.is_none());
        }
    }
    let valid = npdu(Some([0x22; 6]), None, false);
    assert_eq!(
        conn.handle_received(&valid),
        Some((valid.payload, [0x22; 6]))
    );
}

#[test]
fn generic_npdu_codec_preserves_omitted_source_for_direct_connections() {
    let msg = npdu(None, None, false);
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    let decoded = decode_sc_message(&buf).unwrap();
    assert_eq!(decoded.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(decoded.originating_vmac, None);
    assert_eq!(decoded.destination_vmac, None);
    assert_eq!(decoded.payload, msg.payload);
}

async fn start_with_heartbeat_timing() -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport =
        ScTransport::new(ws_client, [0x01; 6]).with_test_heartbeat_timing_ms(100, 1000);
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&ws_hub, [0x10; 6]));
    (transport, rx.unwrap(), ws_hub)
}

fn invalid_source_messages() -> Vec<ScMessage> {
    let mut messages = Vec::new();
    for source in [None, Some([0; 6]), Some(BROADCAST_VMAC)] {
        for destination in [None, Some(BROADCAST_VMAC)] {
            for must_understand in [false, true] {
                messages.push(npdu(source, destination, must_understand));
            }
        }
    }
    messages
}

#[tokio::test]
async fn hub_npdu_invalid_sources_do_not_refresh_liveness() {
    let (mut transport, mut rx, ws_hub) = start_with_heartbeat_timing().await;
    assert_valid_sentinel(&mut rx, &ws_hub, None).await;
    let mut states = transport.connection_state_changes();
    let messages = invalid_source_messages();
    let mut next = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(20));

    // Keep malformed traffic flowing through the real std::Instant timeout.
    // A state notification proves timeout; a generous bound catches refreshes.
    timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                changed = states.changed() => {
                    changed.unwrap();
                    if *states.borrow_and_update() == ScConnectionState::Disconnected {
                        break;
                    }
                }
                _ = interval.tick() => {
                    send(&ws_hub, &messages[next % messages.len()]).await;
                    next += 1;
                }
                response = ws_hub.recv() => {
                    let response = decode_sc_message(&response.unwrap()).unwrap();
                    assert!(matches!(response.function, ScFunction::HeartbeatRequest | ScFunction::Result));
                }
            }
        }
    })
    .await
    .expect("invalid NPDUs kept the hub connection alive");
    assert!(rx.try_recv().is_err(), "invalid NPDU was delivered");
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn hub_npdu_invalid_sources_do_not_clear_pending_heartbeat() {
    let (mut transport, mut rx, ws_hub) = start_with_heartbeat_timing().await;
    assert_valid_sentinel(&mut rx, &ws_hub, None).await;
    let heartbeat = decode_sc_message(&recv(&ws_hub).await).unwrap();
    assert_eq!(heartbeat.function, ScFunction::HeartbeatRequest);

    for msg in invalid_source_messages() {
        send(&ws_hub, &msg).await;
        if msg.originating_vmac.is_none() && msg.destination_vmac.is_none() {
            assert_eq!(
                decode_sc_message(&recv(&ws_hub).await).unwrap().function,
                ScFunction::Result
            );
        }
    }
    // This NAK is a processing barrier after the silent discards. It cannot
    // legitimately refresh activity or clear the outstanding heartbeat.
    send(&ws_hub, &npdu(None, None, false)).await;
    assert_eq!(
        decode_sc_message(&recv(&ws_hub).await).unwrap().function,
        ScFunction::Result
    );
    assert!(
        timeout(Duration::from_millis(350), ws_hub.recv())
            .await
            .is_err(),
        "invalid NPDU cleared the pending heartbeat and caused another request"
    );
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Connected
    );

    let mut ack = heartbeat;
    ack.function = ScFunction::HeartbeatAck;
    send(&ws_hub, &ack).await;
    assert_valid_sentinel(&mut rx, &ws_hub, None).await;
    transport.stop().await.unwrap();
}
