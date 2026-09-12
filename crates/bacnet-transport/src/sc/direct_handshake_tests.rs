//! Direct-connection Connect handshake over loopback (Refs #615).
//!
//! After the direct TLS plus WebSocket upgrade the dial-out path must run a
//! Connect-Request into Connect-Accept exchange with the existing hub
//! validation before any NPDU travels over the direct socket. Any handshake
//! failure falls back to hub delivery without changing the fallback
//! contract. Loopback fixtures only; no network or TLS dials occur here.
//!
//! Source grounding paraphrases the local Standard 135-2020 Annex AB as
//! located through the package navigation map (Annex AB, printed
//! pp1377-1410): a direct connection is established with the direct
//! subprotocol upgrade followed by the Connect identity and length exchange
//! before NPDU traffic, and the initiating peer waits for the matching
//! Accept under its existing connect wait. Only the request plus accept
//! exchange is required; no inbound accept path or additional exchanges are
//! added here.

use super::direct_discovery::parse_ack_uris;
use super::*;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

const TARGET: Vmac = [0x22; 6];
const SENDER: Vmac = [0x01; 6];
const NPDU: &[u8] = &[0x01, 0x00, 0x30];
const SENDER_UUID: [u8; 16] = [1; 16];

async fn hub_recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(2), hub.recv())
        .await
        .expect("hub recv timed out")
        .unwrap()
}

fn ack_for(id: u16, payload: &'static [u8]) -> Vec<u8> {
    let ack = ScMessage {
        function: ScFunction::AddressResolutionAck,
        message_id: id,
        originating_vmac: Some(TARGET),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: bytes::Bytes::from_static(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &ack);
    buf.to_vec()
}

fn direct_accept(id: u16, vmac: Vmac, uuid: [u8; 16], bvlc: u16, apdu: u16) -> Vec<u8> {
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&vmac);
    payload.extend_from_slice(&uuid);
    payload.extend_from_slice(&bvlc.to_be_bytes());
    payload.extend_from_slice(&apdu.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: bytes::Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    buf.to_vec()
}

fn assert_direct_request(req: &ScMessage) {
    assert_eq!(req.function, ScFunction::ConnectRequest);
    assert_eq!(req.originating_vmac, None);
    assert_eq!(req.destination_vmac, None);
    assert!(req.data_options.is_empty());
    assert_eq!(req.payload.len(), 26);
    assert_eq!(&req.payload[0..6], &SENDER);
    assert_eq!(&req.payload[6..22], &SENDER_UUID);
    assert_eq!(
        u16::from_be_bytes([req.payload[22], req.payload[23]]),
        crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH
    );
    assert_eq!(
        u16::from_be_bytes([req.payload[24], req.payload[25]]),
        1476u16
    );
}

async fn start_sender(
    connect_timeout_ms: u64,
) -> (
    ScTransport<LoopbackWebSocket>,
    LoopbackWebSocket,
    Arc<Mutex<Vec<String>>>,
    mpsc::UnboundedReceiver<LoopbackWebSocket>,
) {
    let attempted = Arc::new(Mutex::new(Vec::new()));
    let (peer_tx, peer_rx) = mpsc::unbounded_channel();
    let attempted_clone = attempted.clone();
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, SENDER)
        .with_device_uuid(SENDER_UUID)
        .with_connect_timeout_ms(connect_timeout_ms)
        .with_direct_discovery(true)
        .with_direct_dialer(move |uri: String| {
            let attempted = attempted_clone.clone();
            let peer_tx = peer_tx.clone();
            async move {
                attempted.lock().await.push(uri);
                let (client, peer) = LoopbackWebSocket::pair();
                let _ = peer_tx.send(peer);
                Ok(client)
            }
        });
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    // Exercise the shared ACK parser so the import stays live if helpers shift.
    assert_eq!(parse_ack_uris(b"wss://peer.example/sc").unwrap().len(), 1);
    (transport, hub, attempted, peer_rx)
}

#[tokio::test]
async fn direct_handshake_valid_accept_enables_direct_send() {
    let (mut transport, hub, attempted, mut peers) = start_sender(800).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        assert_eq!(ar.function, ScFunction::AddressResolution);
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
        let peer = timeout(Duration::from_secs(2), peers.recv())
            .await
            .expect("direct peer missing")
            .expect("peer channel closed");
        let req_bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .expect("direct request timed out")
            .unwrap();
        let req = decode_sc_message(&req_bytes).unwrap();
        assert_direct_request(&req);
        peer.send(&direct_accept(
            req.message_id,
            [0x33; 6],
            [0x44; 16],
            1476,
            1476,
        ))
        .await
        .unwrap();
        let npdu_bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .expect("direct NPDU timed out")
            .unwrap();
        let direct = decode_sc_message(&npdu_bytes).unwrap();
        assert_eq!(direct.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(direct.originating_vmac, None);
        assert_eq!(direct.destination_vmac, None);
        assert_eq!(direct.payload.as_ref(), NPDU);
    });
    send_res.unwrap();
    assert_eq!(
        attempted.lock().await.as_slice(),
        &["wss://peer.example/sc".to_owned()]
    );
    assert!(
        timeout(Duration::from_millis(100), hub.recv())
            .await
            .is_err(),
        "valid handshake must keep hub silent"
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_handshake_bad_identity_falls_back_to_hub() {
    let (mut transport, hub, attempted, mut peers) = start_sender(400).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
        let peer = timeout(Duration::from_secs(2), peers.recv())
            .await
            .expect("direct peer missing")
            .expect("peer channel closed");
        let req_bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .expect("direct request timed out")
            .unwrap();
        let req = decode_sc_message(&req_bytes).unwrap();
        assert_direct_request(&req);
        // Reserved all-zero VMAC identity is rejected by the shared
        // Connect-Accept validation, so no NPDU may follow on direct.
        // Invalid Accepts are silently discarded within the existing wait;
        // either a wait timeout or a dropped socket both mean no direct NPDU.
        peer.send(&direct_accept(
            req.message_id,
            [0; 6],
            [0x44; 16],
            1476,
            1476,
        ))
        .await
        .unwrap();
        if let Ok(Ok(bytes)) = timeout(Duration::from_millis(300), peer.recv()).await {
            panic!("bad identity must not enable direct NPDU: {bytes:?}");
        }
    });
    send_res.unwrap();
    assert_eq!(attempted.lock().await.len(), 1);
    let npdu_bytes = hub_recv(&hub).await;
    let hub_msg = decode_sc_message(&npdu_bytes).unwrap();
    assert_eq!(hub_msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(hub_msg.destination_vmac, Some(TARGET));
    assert_eq!(hub_msg.payload.as_ref(), NPDU);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_handshake_zero_limits_falls_back_to_hub() {
    let (mut transport, hub, attempted, mut peers) = start_sender(400).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
        let peer = timeout(Duration::from_secs(2), peers.recv())
            .await
            .expect("direct peer missing")
            .expect("peer channel closed");
        let req_bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .expect("direct request timed out")
            .unwrap();
        let req = decode_sc_message(&req_bytes).unwrap();
        assert_direct_request(&req);
        // Zero Max-BVLC-Length is rejected by the shared validation.
        // Either a wait timeout or a dropped socket both mean no direct NPDU.
        peer.send(&direct_accept(
            req.message_id,
            [0x33; 6],
            [0x44; 16],
            0,
            1476,
        ))
        .await
        .unwrap();
        if let Ok(Ok(bytes)) = timeout(Duration::from_millis(300), peer.recv()).await {
            panic!("zero limits must not enable direct NPDU: {bytes:?}");
        }
    });
    send_res.unwrap();
    assert_eq!(attempted.lock().await.len(), 1);
    let npdu_bytes = hub_recv(&hub).await;
    let hub_msg = decode_sc_message(&npdu_bytes).unwrap();
    assert_eq!(hub_msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(hub_msg.destination_vmac, Some(TARGET));
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_handshake_timeout_falls_back_to_hub() {
    let (mut transport, hub, attempted, mut peers) = start_sender(120).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
        let peer = timeout(Duration::from_secs(2), peers.recv())
            .await
            .expect("direct peer missing")
            .expect("peer channel closed");
        // Observe the handshake request but never answer, so the existing
        // connect wait expires and the send falls back to the hub. Either a
        // wait timeout or a dropped socket both mean no direct NPDU.
        let req_bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .expect("direct request timed out")
            .unwrap();
        assert_direct_request(&decode_sc_message(&req_bytes).unwrap());
        if let Ok(Ok(bytes)) = timeout(Duration::from_millis(400), peer.recv()).await {
            panic!("handshake timeout must not enable direct NPDU: {bytes:?}");
        }
    });
    send_res.unwrap();
    assert_eq!(attempted.lock().await.len(), 1);
    let npdu_bytes = hub_recv(&hub).await;
    let hub_msg = decode_sc_message(&npdu_bytes).unwrap();
    assert_eq!(hub_msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(hub_msg.destination_vmac, Some(TARGET));
    assert_eq!(hub_msg.payload.as_ref(), NPDU);
    transport.stop().await.unwrap();
}
