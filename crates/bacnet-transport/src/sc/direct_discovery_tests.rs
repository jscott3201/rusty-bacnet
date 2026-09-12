//! Opt-in direct discovery + NPDU-over-direct with hub fallback (Refs #615 PR3b).
//!
//! Enabled unicast consults the bounded URI cache; on a miss it issues one
//! Address-Resolution request through the hub, caches the ACK URIs by message
//! ID, dials direct, and sends the NPDU over direct with both addresses
//! omitted. Any direct-stage failure falls back to hub delivery. Disabled
//! (default) runs the hub path unchanged. Loopback fixtures only; no network
//! or TLS dials occur here.

use super::direct_discovery::{
    parse_ack_uris, DirectUriCache, DIRECT_URI_CACHE_MAX_ENTRIES, DIRECT_URI_CACHE_TTL,
};
use super::*;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

const TARGET: Vmac = [0x22; 6];
const NPDU: &[u8] = &[0x01, 0x00, 0x30];

async fn hub_recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(2), hub.recv())
        .await
        .expect("hub recv timed out")
        .unwrap()
}

async fn hub_try_recv(hub: &LoopbackWebSocket) -> Option<Vec<u8>> {
    timeout(Duration::from_millis(150), hub.recv())
        .await
        .ok()
        .map(|r| r.unwrap())
}

fn fake_dialer(
    attempted: Arc<Mutex<Vec<String>>>,
    peers: mpsc::UnboundedSender<LoopbackWebSocket>,
) -> impl Fn(
    String,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<LoopbackWebSocket, Error>> + Send>,
> + Send
       + Sync
       + 'static {
    move |uri: String| {
        let attempted = attempted.clone();
        let peers = peers.clone();
        Box::pin(async move {
            attempted.lock().await.push(uri);
            let (client, peer) = LoopbackWebSocket::pair();
            // Answer the direct Connect handshake on the peer side with a
            // valid Accept, then hand the peer to the test for the NPDU
            // check. This keeps discovery-focused tests green while the
            // production path requires the handshake before any NPDU.
            let peers_for_handshake = peers.clone();
            tokio::spawn(async move {
                let Ok(req_bytes) = peer.recv().await else {
                    return;
                };
                let Ok(req) = decode_sc_message(&req_bytes) else {
                    let _ = peers_for_handshake.send(peer);
                    return;
                };
                if req.function == ScFunction::ConnectRequest {
                    let mut payload = Vec::with_capacity(26);
                    payload.extend_from_slice(&[0x33; 6]);
                    payload.extend_from_slice(&[0x44; 16]);
                    payload.extend_from_slice(&1476u16.to_be_bytes());
                    payload.extend_from_slice(&1476u16.to_be_bytes());
                    let accept = ScMessage {
                        function: ScFunction::ConnectAccept,
                        message_id: req.message_id,
                        originating_vmac: None,
                        destination_vmac: None,
                        dest_options: Vec::new(),
                        data_options: Vec::new(),
                        payload: bytes::Bytes::from(payload),
                    };
                    let mut buf = BytesMut::new();
                    encode_sc_message(&mut buf, &accept);
                    let _ = peer.send(&buf).await;
                }
                let _ = peers_for_handshake.send(peer);
            });
            Ok(client)
        })
            as std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<LoopbackWebSocket, Error>> + Send>,
            >
    }
}

async fn start_direct(
    connect_timeout_ms: u64,
) -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
    Arc<Mutex<Vec<String>>>,
    mpsc::UnboundedReceiver<LoopbackWebSocket>,
) {
    let attempted = Arc::new(Mutex::new(Vec::new()));
    let (peer_tx, peer_rx) = mpsc::unbounded_channel();
    let (client, hub) = LoopbackWebSocket::pair();
    let dialer = fake_dialer(attempted.clone(), peer_tx);
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(connect_timeout_ms)
        .with_direct_discovery(true)
        .with_direct_dialer(dialer);
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    (transport, rx, hub, attempted, peer_rx)
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

const SENDER: Vmac = [0x01; 6];

async fn start_responder(
    uris: &[&str],
) -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, TARGET)
        .with_device_uuid([2; 16])
        .with_advertised_uris(uris.to_vec());
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    (transport, rx, hub)
}

async fn relay_one_ar_exchange(sender_hub: &LoopbackWebSocket, responder_hub: &LoopbackWebSocket) {
    let ar_bytes = hub_recv(sender_hub).await;
    let ar = decode_sc_message(&ar_bytes).unwrap();
    assert_eq!(ar.function, ScFunction::AddressResolution);
    assert_eq!(ar.destination_vmac, Some(TARGET));
    assert_eq!(ar.originating_vmac, None);
    assert!(ar.payload.is_empty());
    let relayed = ScMessage {
        function: ar.function,
        message_id: ar.message_id,
        originating_vmac: Some(SENDER),
        destination_vmac: None,
        dest_options: ar.dest_options.clone(),
        data_options: ar.data_options.clone(),
        payload: ar.payload.clone(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &relayed);
    responder_hub.send(&buf).await.unwrap();
    let ack_bytes = hub_recv(responder_hub).await;
    let ack = decode_sc_message(&ack_bytes).unwrap();
    assert_eq!(ack.function, ScFunction::AddressResolutionAck);
    assert_eq!(ack.message_id, ar.message_id);
    assert_eq!(ack.destination_vmac, Some(SENDER));
    assert_eq!(ack.originating_vmac, None);
    let back = ScMessage {
        function: ack.function,
        message_id: ack.message_id,
        originating_vmac: Some(TARGET),
        destination_vmac: None,
        dest_options: ack.dest_options.clone(),
        data_options: ack.data_options.clone(),
        payload: ack.payload.clone(),
    };
    let mut back_buf = BytesMut::new();
    encode_sc_message(&mut back_buf, &back);
    sender_hub.send(&back_buf).await.unwrap();
}

#[test]
fn uri_cache_count_bound_evicts_oldest_first() {
    let mut cache = DirectUriCache::new();
    let now = Instant::now();
    for i in 0..DIRECT_URI_CACHE_MAX_ENTRIES + 5 {
        let mut vmac = [0u8; 6];
        vmac[0] = (i & 0xFF) as u8;
        vmac[1] = ((i >> 8) & 0xFF) as u8;
        cache.insert(vmac, vec![format!("wss://peer{i}.example/sc")], now);
    }
    assert_eq!(cache.len(), DIRECT_URI_CACHE_MAX_ENTRIES);
    let mut oldest = [0u8; 6];
    oldest[0] = 0;
    oldest[1] = 0;
    assert!(cache.get(&oldest, now).is_none());
    let last = DIRECT_URI_CACHE_MAX_ENTRIES + 4;
    let mut newest = [0u8; 6];
    newest[0] = (last & 0xFF) as u8;
    newest[1] = ((last >> 8) & 0xFF) as u8;
    assert!(cache.get(&newest, now).is_some());
}

#[test]
fn uri_cache_ttl_expiry_drops_stale_entries() {
    let mut cache = DirectUriCache::new();
    let inserted = Instant::now();
    cache.insert(TARGET, vec!["wss://peer.example/sc".into()], inserted);
    assert!(cache.get(&TARGET, inserted).is_some());
    let expired = inserted + DIRECT_URI_CACHE_TTL + Duration::from_secs(1);
    assert!(cache.get(&TARGET, expired).is_none());
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn ack_uri_parsing_accepts_empty_and_valid_lists() {
    assert_eq!(parse_ack_uris(&[]), Some(Vec::new()));
    assert_eq!(
        parse_ack_uris(b"wss://one.example/sc"),
        Some(vec!["wss://one.example/sc".to_owned()])
    );
    assert_eq!(
        parse_ack_uris(b"wss://one.example/sc wss://two.example:8443/sc"),
        Some(vec![
            "wss://one.example/sc".to_owned(),
            "wss://two.example:8443/sc".to_owned()
        ])
    );
}

#[test]
fn ack_uri_parsing_rejects_structural_faults() {
    for payload in [
        b" wss://one.example/sc".as_slice(),
        b"wss://one.example/sc ".as_slice(),
        b"wss://one.example/sc  wss://two.example/sc".as_slice(),
        b"ws://one.example/sc".as_slice(),
        b"wss://".as_slice(),
        b"not-a-uri".as_slice(),
    ] {
        assert_eq!(parse_ack_uris(payload), None, "payload {payload:?}");
    }
    assert_eq!(parse_ack_uris(&[0xFF, 0x00]), None);
}

#[test]
fn connection_builders_shape_request_and_direct_npdu() {
    let mut conn = ScConnection::new([1; 6], [1; 16]);
    conn.state = ScConnectionState::Connected;
    let req = conn.build_address_resolution_request(TARGET);
    assert_eq!(req.function, ScFunction::AddressResolution);
    assert_eq!(req.destination_vmac, Some(TARGET));
    assert_eq!(req.originating_vmac, None);
    assert!(req.payload.is_empty());
    assert!(req.data_options.is_empty());
    let direct = conn
        .build_direct_encapsulated_npdu(NPDU, &[])
        .expect("direct build must succeed");
    assert_eq!(direct.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(direct.originating_vmac, None);
    assert_eq!(direct.destination_vmac, None);
    assert_eq!(direct.payload.as_ref(), NPDU);
    assert_eq!(
        direct.message_id,
        req.message_id.wrapping_add(1),
        "builders must consume fresh IDs"
    );
}

#[tokio::test]
async fn miss_triggers_ar_request_then_direct_send_with_hub_silence() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(500).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        assert_eq!(ar.function, ScFunction::AddressResolution);
        assert_eq!(ar.destination_vmac, Some(TARGET));
        assert!(ar.payload.is_empty());
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    send_res.unwrap();
    assert_eq!(
        attempted.lock().await.as_slice(),
        &["wss://peer.example/sc".to_owned()]
    );
    let peer = timeout(Duration::from_secs(2), peers.recv())
        .await
        .unwrap()
        .expect("direct peer missing");
    let direct_bytes = timeout(Duration::from_secs(2), peer.recv())
        .await
        .unwrap()
        .unwrap();
    let direct = decode_sc_message(&direct_bytes).unwrap();
    assert_eq!(direct.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(direct.originating_vmac, None);
    assert_eq!(direct.destination_vmac, None);
    assert_eq!(direct.payload.as_ref(), NPDU);
    assert!(hub_try_recv(&hub).await.is_none());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn default_off_sends_hub_only_with_no_ar_or_direct() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    transport.send_unicast(NPDU, &TARGET).await.unwrap();
    let bytes = hub_recv(&hub).await;
    let msg = decode_sc_message(&bytes).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(TARGET));
    assert_eq!(msg.payload.as_ref(), NPDU);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn ack_timeout_falls_back_to_hub_without_dial() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(60)
        .with_direct_discovery(true)
        .with_direct_dialer(
            |_uri: String| async move { panic!("no ACK means no dial must occur") },
        );
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        assert_eq!(
            decode_sc_message(&ar_bytes).unwrap().function,
            ScFunction::AddressResolution
        );
    });
    send_res.unwrap();
    let npdu_bytes = hub_recv(&hub).await;
    let npdu_msg = decode_sc_message(&npdu_bytes).unwrap();
    assert_eq!(npdu_msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(npdu_msg.destination_vmac, Some(TARGET));
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn empty_ack_falls_back_to_hub_without_dial() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(500).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"")).await.unwrap();
    });
    send_res.unwrap();
    assert!(attempted.lock().await.is_empty());
    assert!(timeout(Duration::from_millis(100), peers.recv())
        .await
        .is_err());
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn ar_nak_falls_back_to_hub_without_dial() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(500).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        let nak = ScMessage {
            function: ScFunction::Result,
            message_id: ar.message_id,
            originating_vmac: Some(TARGET),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::from(vec![
                ScFunction::AddressResolution.to_raw(),
                0x01,
                0x00,
                0x00,
                0x07,
                0x00,
                0x96,
            ]),
        };
        let mut nak_buf = BytesMut::new();
        encode_sc_message(&mut nak_buf, &nak);
        hub.send(&nak_buf).await.unwrap();
    });
    send_res.unwrap();
    assert!(attempted.lock().await.is_empty());
    assert!(timeout(Duration::from_millis(100), peers.recv())
        .await
        .is_err());
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn mismatched_ack_id_is_ignored_then_hub_fallback() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(80).await;
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(
            ar.message_id.wrapping_add(1),
            b"wss://peer.example/sc",
        ))
        .await
        .unwrap();
    });
    send_res.unwrap();
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    assert!(attempted.lock().await.is_empty());
    assert!(timeout(Duration::from_millis(50), peers.recv())
        .await
        .is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_dial_failure_falls_back_to_hub() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(300)
        .with_direct_discovery(true)
        .with_direct_dialer(|uri: String| async move {
            assert_eq!(uri, "wss://peer.example/sc");
            Err::<LoopbackWebSocket, Error>(Error::Encoding("dial refused".into()))
        });
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    send_res.unwrap();
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn direct_send_failure_falls_back_to_hub() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(300)
        .with_direct_discovery(true)
        .with_direct_dialer(|_uri: String| async move {
            let (direct_client, direct_peer) = LoopbackWebSocket::pair();
            drop(direct_peer);
            Ok(direct_client)
        });
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    send_res.unwrap();
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn enabled_without_dialer_falls_back_to_hub() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(300)
        .with_direct_discovery(true);
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    let (send_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    send_res.unwrap();
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn broadcast_never_uses_direct_even_when_enabled() {
    let (mut transport, _rx, hub, attempted, mut peers) = start_direct(500).await;
    transport.send_unicast(NPDU, &BROADCAST_VMAC).await.unwrap();
    let bytes = hub_recv(&hub).await;
    let msg = decode_sc_message(&bytes).unwrap();
    assert_eq!(msg.destination_vmac, Some(BROADCAST_VMAC));
    assert!(attempted.lock().await.is_empty());
    assert!(timeout(Duration::from_millis(100), peers.recv())
        .await
        .is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn interop_production_answer_drives_direct_without_timeout() {
    let (mut sender, _rx, sender_hub, attempted, mut peers) = start_direct(2000).await;
    let (mut responder, mut responder_rx, responder_hub) =
        start_responder(&["wss://peer.example/sc"]).await;
    let start = Instant::now();
    let (send_res, ()) = tokio::join!(
        sender.send_unicast(NPDU, &TARGET),
        relay_one_ar_exchange(&sender_hub, &responder_hub)
    );
    send_res.unwrap();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(1500));
    assert_eq!(
        attempted.lock().await.as_slice(),
        &["wss://peer.example/sc".to_owned()]
    );
    let peer = timeout(Duration::from_secs(2), peers.recv())
        .await
        .unwrap()
        .expect("direct peer missing");
    let direct = decode_sc_message(
        &timeout(Duration::from_secs(2), peer.recv())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(direct.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(direct.originating_vmac, None);
    assert_eq!(direct.destination_vmac, None);
    assert_eq!(direct.payload.as_ref(), NPDU);
    assert!(hub_try_recv(&sender_hub).await.is_none());
    assert!(responder_rx.try_recv().is_err());
    sender.stop().await.unwrap();
    responder.stop().await.unwrap();
}

#[tokio::test]
async fn interop_repeated_send_uses_cache_without_stall() {
    let (mut sender, _rx, sender_hub, attempted, mut peers) = start_direct(2000).await;
    let (mut responder, mut responder_rx, responder_hub) =
        start_responder(&["wss://peer.example/sc"]).await;
    let (first_res, ()) = tokio::join!(
        sender.send_unicast(NPDU, &TARGET),
        relay_one_ar_exchange(&sender_hub, &responder_hub)
    );
    first_res.unwrap();
    let first_peer = timeout(Duration::from_secs(2), peers.recv())
        .await
        .unwrap()
        .expect("first direct peer missing");
    timeout(Duration::from_secs(2), first_peer.recv())
        .await
        .unwrap()
        .unwrap();
    let start = Instant::now();
    sender.send_unicast(NPDU, &TARGET).await.unwrap();
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(1000));
    assert!(hub_try_recv(&sender_hub).await.is_none());
    let second_peer = timeout(Duration::from_secs(2), peers.recv())
        .await
        .unwrap()
        .expect("cached direct peer missing");
    let second = decode_sc_message(
        &timeout(Duration::from_secs(2), second_peer.recv())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(second.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(second.payload.as_ref(), NPDU);
    assert_eq!(attempted.lock().await.len(), 2);
    assert!(responder_rx.try_recv().is_err());
    sender.stop().await.unwrap();
    responder.stop().await.unwrap();
}
