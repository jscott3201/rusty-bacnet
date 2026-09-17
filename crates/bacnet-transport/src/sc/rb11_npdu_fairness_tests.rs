//! RB-11 hub-path fairness over real loopback producers (Refs #518).
//!
//! Deterministic local producers via [`LoopbackWebSocket`]: a single hub-side
//! sender preserves wire order, and a Heartbeat-Request/ACK round trip is the
//! barrier proving the transport processed every prior wire frame — the NPDU
//! channel (not the wire channel) is what fills. Asserts progress, bounds
//! and cleanup, never throughput.

use super::*;

const BURSTY: Vmac = [0xA0; 6];
const HUB_VMAC: Vmac = [0x10; 6];

fn quota() -> usize {
    ScNpduAdmissionPolicy::default().per_origin_limit
}

async fn hub_accept(ws_hub: &LoopbackWebSocket) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);
    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&HUB_VMAC);
    accept_payload.extend_from_slice(&[0x33; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn start_transport(
    client_vmac: Vmac,
) -> (
    ScTransport<LoopbackWebSocket>,
    LoopbackWebSocket,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, client_vmac).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        hub_accept(&ws_hub).await;
        ws_hub
    });
    let rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    (transport, ws_hub, rx)
}

async fn send_npdu(
    ws_hub: &LoopbackWebSocket,
    origin: Vmac,
    dest: Option<Vmac>,
    payload: Vec<u8>,
    id: u16,
) {
    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: id,
        originating_vmac: Some(origin),
        destination_vmac: dest,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();
}

/// Barrier: the transport processes wire frames FIFO, so the Heartbeat-ACK
/// with this ID proves every prior frame was admitted or dropped already.
async fn barrier(ws_hub: &LoopbackWebSocket, id: u16) {
    let hb = ScMessage {
        function: ScFunction::HeartbeatRequest,
        message_id: id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &hb);
    ws_hub.send(&buf).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let bytes = ws_hub.recv().await.unwrap();
            let msg = decode_sc_message(&bytes).unwrap();
            if msg.function == ScFunction::HeartbeatAck && msg.message_id == id {
                break;
            }
        }
    })
    .await
    .expect("heartbeat barrier timed out");
}

fn payload(origin: u8, seq: u8) -> Vec<u8> {
    vec![0x01, 0x00, origin, seq]
}

#[tokio::test]
async fn bursty_source_capped_while_legitimate_origins_progress() {
    assert_eq!(quota(), 4);
    let (mut transport, ws_hub, mut rx) = start_transport([0x01; 6]).await;
    // One bursty origin floods first: only the quota survives.
    for seq in 0..64u16 {
        send_npdu(&ws_hub, BURSTY, None, payload(0xA0, seq as u8), seq).await;
    }
    // Fifteen legitimate origins fill the rest of the queue.
    for origin in 1..16u8 {
        for seq in 0..4u8 {
            send_npdu(
                &ws_hub,
                [origin; 6],
                None,
                payload(origin, seq),
                u16::from(origin) * 16 + u16::from(seq),
            )
            .await;
        }
    }
    // Below quota but globally full: aggregate drop.
    send_npdu(&ws_hub, [0x40; 6], None, payload(0x40, 0), 0x7000).await;
    // Over quota AND globally full: fairness takes precedence over Full.
    send_npdu(&ws_hub, BURSTY, None, payload(0xA0, 0xFE), 0x7001).await;
    barrier(&ws_hub, 0x7002).await;
    // Measured tradeoff: the burst is capped at 4/64 (one quota) while
    // legitimate origins hold the other 60; the extra burst costs fairness
    // drops, the fresh origin costs exactly one Full drop.
    assert_eq!(
        transport.npdu_drop_counts(),
        ScNpduDropCounts {
            fairness_drops: 61,
            full_drops: 1,
            closed_drops: 0,
        }
    );
    // Draining proves progress per origin: burst first, then each origin.
    let mut seen: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for _ in 0..64 {
        let received = rx.try_recv().expect("64 queued NPDUs must drain");
        assert!(received.provenance.is_relayed_origin());
        assert!(received.reply_tx.is_none());
        *seen.entry(received.npdu[2]).or_default() += 1;
    }
    assert_eq!(seen.get(&0xA0), Some(&4));
    for origin in 1..16u8 {
        assert_eq!(seen.get(&origin), Some(&4), "origin {origin:#x} starved");
    }
    assert!(rx.try_recv().is_err());
    // Quota released by draining: the previously capped burst progresses.
    send_npdu(&ws_hub, BURSTY, None, payload(0xA0, 9), 0x7100).await;
    barrier(&ws_hub, 0x7101).await;
    assert_eq!(rx.try_recv().unwrap().npdu.as_ref(), &[0x01, 0x00, 0xA0, 9]);
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 61);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn slow_consumer_keeps_control_progress_while_queue_full() {
    let (mut transport, ws_hub, mut rx) = start_transport([0x01; 6]).await;
    for seq in 0..70u16 {
        send_npdu(&ws_hub, BURSTY, None, payload(0xA0, seq as u8), seq).await;
    }
    barrier(&ws_hub, 0x7200).await;
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 66);
    // Heartbeat still answered with a full app queue (no new await blocks it).
    barrier(&ws_hub, 0x7201).await;
    // Disconnect still processed with a full app queue.
    let disc = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 0x7300,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &disc);
    ws_hub.send(&buf).await.unwrap();
    let ack_bytes = tokio::time::timeout(Duration::from_secs(5), ws_hub.recv())
        .await
        .expect("disconnect ack timed out")
        .unwrap();
    let ack = decode_sc_message(&ack_bytes).unwrap();
    assert_eq!(ack.function, ScFunction::DisconnectAck);
    assert_eq!(ack.message_id, 0x7300);
    assert_eq!(
        transport.connection().unwrap().lock().await.state,
        ScConnectionState::Disconnected
    );
    // Control frames never touched the quota.
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 66);
    assert_eq!(transport.npdu_drop_counts().full_drops, 0);
    // Queued NPDUs remain drainable.
    for _ in 0..4 {
        rx.try_recv().unwrap();
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_burst_progresses_when_consumer_drains() {
    let (mut transport, ws_hub, mut rx) = start_transport([0x01; 6]).await;
    // A finite multi-NPDU sequence larger than one quota completes as long
    // as the consumer drains: no SC reassembly cache, no deadlock.
    for chunk in 0..3u8 {
        for seq in 0..4u8 {
            let id = u16::from(chunk) * 4 + u16::from(seq);
            send_npdu(&ws_hub, BURSTY, None, payload(0xA0, id as u8), id).await;
        }
        barrier(&ws_hub, 0x7400 + u16::from(chunk)).await;
        for seq in 0..4u8 {
            let id = u16::from(chunk) * 4 + u16::from(seq);
            let received = rx.try_recv().expect("drained quota must admit more");
            assert_eq!(received.npdu.as_ref(), &[0x01, 0x00, 0xA0, id as u8]);
        }
    }
    assert_eq!(transport.npdu_drop_counts(), ScNpduDropCounts::default());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn unicast_and_control_share_quota_without_charging_control() {
    let (mut transport, ws_hub, mut rx) = start_transport([0x01; 6]).await;
    let origin: Vmac = [0x22; 6];
    // Unicast- and broadcast-shaped NPDUs share one per-origin quota.
    send_npdu(&ws_hub, origin, None, payload(0x22, 0), 0x7500).await;
    send_npdu(&ws_hub, origin, None, payload(0x22, 1), 0x7501).await;
    send_npdu(
        &ws_hub,
        origin,
        Some(BROADCAST_VMAC),
        payload(0x22, 2),
        0x7502,
    )
    .await;
    // Control frames interleaved: heartbeat plus a well-formed solicitation
    // (answered, never queued) must not consume quota.
    barrier(&ws_hub, 0x7503).await;
    let solicitation = ScMessage {
        function: ScFunction::AdvertisementSolicitation,
        message_id: 0x7504,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &solicitation);
    ws_hub.send(&buf).await.unwrap();
    send_npdu(
        &ws_hub,
        origin,
        Some(BROADCAST_VMAC),
        payload(0x22, 3),
        0x7505,
    )
    .await;
    barrier(&ws_hub, 0x7506).await;
    assert_eq!(transport.npdu_drop_counts(), ScNpduDropCounts::default());
    // Fifth NPDU of either shape now exceeds the shared quota.
    send_npdu(&ws_hub, origin, None, payload(0x22, 4), 0x7507).await;
    send_npdu(
        &ws_hub,
        origin,
        Some(BROADCAST_VMAC),
        payload(0x22, 5),
        0x7508,
    )
    .await;
    barrier(&ws_hub, 0x7509).await;
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 2);
    assert_eq!(transport.npdu_drop_counts().full_drops, 0);
    let first = rx.try_recv().unwrap();
    assert!(!first.link_layer_group);
    rx.try_recv().unwrap();
    let third = rx.try_recv().unwrap();
    assert!(third.link_layer_group);
    rx.try_recv().unwrap();
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn close_and_cancellation_preserve_closed_precedence() {
    let (mut transport, ws_hub, mut rx) = start_transport([0x01; 6]).await;
    // A cancelled receive on the empty queue changes nothing.
    assert!(tokio::time::timeout(Duration::from_millis(20), rx.recv())
        .await
        .is_err());
    assert_eq!(transport.npdu_drop_counts(), ScNpduDropCounts::default());
    for seq in 0..4u16 {
        send_npdu(&ws_hub, BURSTY, None, payload(0xA0, seq as u8), seq).await;
    }
    barrier(&ws_hub, 0x7600).await;
    rx.close();
    // Closed takes precedence over fairness for the capped source.
    send_npdu(&ws_hub, BURSTY, None, payload(0xA0, 9), 0x7601).await;
    barrier(&ws_hub, 0x7602).await;
    assert_eq!(
        transport.npdu_drop_counts(),
        ScNpduDropCounts {
            closed_drops: 1,
            ..Default::default()
        }
    );
    // Draining still works after close; dropping then counts Closed again.
    for _ in 0..4 {
        rx.recv().await.expect("close preserves queued items");
    }
    drop(rx);
    send_npdu(&ws_hub, BURSTY, None, payload(0xA0, 10), 0x7603).await;
    barrier(&ws_hub, 0x7604).await;
    assert_eq!(transport.npdu_drop_counts().closed_drops, 2);
    assert_eq!(transport.npdu_drop_counts().fairness_drops, 0);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_per_origin_limit_rejected_at_start() {
    for limit in [0, 65, usize::MAX] {
        let (ws_client, _ws_hub) = LoopbackWebSocket::pair();
        let mut transport = ScTransport::new(ws_client, [0x01; 6])
            .with_device_uuid([1; 16])
            .with_npdu_per_origin_limit(limit);
        let err = transport.start().await.unwrap_err();
        assert!(
            err.to_string().contains("per-origin limit"),
            "limit {limit}: {err}"
        );
    }
}
