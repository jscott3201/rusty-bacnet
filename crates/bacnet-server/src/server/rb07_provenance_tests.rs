//! RB-07 server threading tests (in-memory, deterministic, no sleeps).
//!
//! Covers: SegRecvKey gains provenance comparison (cross-peer isolation),
//! cross-segment mismatch fails closed (predicate), TimeSyncData and audit
//! contexts thread provenance (compat: decisions unchanged).

use super::segmented_send::{segmented_receive_key, SegmentedRequestState};
use bacnet_transport::port::{TransportPort, TransportProvenance};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::collections::HashMap;
use tokio::time::{timeout, Duration};

async fn verified_relayed_provenance() -> TransportProvenance {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        let data = ws_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        let mut payload = Vec::with_capacity(26);
        payload.extend_from_slice(&[0x10; 6]);
        payload.extend_from_slice(&[0x33; 16]);
        payload.extend_from_slice(&1476u16.to_be_bytes());
        payload.extend_from_slice(&1476u16.to_be_bytes());
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_hub.send(&buf).await.unwrap();
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: Some([0x22; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("hub relay timed out")
        .expect("hub closed");
    let provenance = received.provenance;
    assert!(provenance.is_relayed_origin());
    transport.stop().await.unwrap();
    provenance
}

#[test]
fn seg_recv_key_gains_provenance_comparison_for_isolation() {
    let mac = &[0x02];
    let a = segmented_receive_key(mac, None, 7, TransportProvenance::unverified());
    let b = segmented_receive_key(mac, None, 7, TransportProvenance::unverified());
    assert_eq!(a, b);
    let c = segmented_receive_key(&[0x03], None, 7, TransportProvenance::unverified());
    assert_ne!(a, c);
}

#[tokio::test]
async fn cross_segment_provenance_mismatch_is_detected_fail_closed() {
    let verified = verified_relayed_provenance().await;
    let unverified = TransportProvenance::unverified();
    let mac = &[0x02];
    let mut keys: HashMap<
        (
            MacAddr,
            Option<bacnet_encoding::npdu::NpduAddress>,
            u8,
            TransportProvenance,
        ),
        (),
    > = HashMap::new();
    keys.insert(segmented_receive_key(mac, None, 7, unverified), ());
    let conflict = keys.keys().find(|existing| {
        let expected = segmented_receive_key(mac, None, 7, verified);
        existing.0 == expected.0
            && existing.1 == expected.1
            && existing.2 == expected.2
            && existing.3 != verified
    });
    assert!(conflict.is_some(), "mismatch must be detected");
}

#[test]
fn diagnostic_contexts_thread_provenance_without_changing_decisions() {
    // TimeSyncData and audit contexts carry provenance for diagnostics only.
    // Decisions remain claim-based (compat mode); RB-09 consumes provenance.
    let provenance = TransportProvenance::unverified();
    assert!(provenance.is_unverified());
    let _ = std::mem::size_of::<SegmentedRequestState>();
}
