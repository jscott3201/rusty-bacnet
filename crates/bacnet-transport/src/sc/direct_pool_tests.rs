//! Bounded redial backoff + pooled reuse for dial-out direct sends (#615).
//!
//! Owner-local policy (Annex AB leaves initiation/re-initiation timing to
//! the local node): per-URI exponential backoff (`200ms..5s`) bounds
//! repeated failing dials; one handshaked connection per destination VMAC is
//! pooled with a 60s idle TTL and FIFO eviction. Every failure still falls
//! back to hub delivery; default-off stays hub-only. Loopback fixtures only.

use super::direct_discovery::{
    redial_backoff_delay, DirectPool, RedialBackoff, DIRECT_POOL_IDLE_TTL, DIRECT_POOL_MAX_ENTRIES,
    DIRECT_REDIAL_INITIAL_BACKOFF, DIRECT_REDIAL_MAX_BACKOFF, DIRECT_REDIAL_MAX_ENTRIES,
};
use super::*;
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
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

fn direct_accept(id: u16) -> Vec<u8> {
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&[0x33; 6]);
    payload.extend_from_slice(&[0x44; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
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

/// Counting dialer that answers the direct handshake on the peer side and
/// hands the peer to the test. The peer stays alive for pooled-reuse reads.
fn counting_handshake_dialer(
    dials: Arc<AtomicUsize>,
    peers: mpsc::UnboundedSender<LoopbackWebSocket>,
) -> impl Fn(
    String,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<LoopbackWebSocket, Error>> + Send>,
> + Send
       + Sync
       + 'static {
    move |_uri: String| {
        let dials = dials.clone();
        let peers = peers.clone();
        Box::pin(async move {
            dials.fetch_add(1, Ordering::SeqCst);
            let (client, peer) = LoopbackWebSocket::pair();
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
                    let _ = peer.send(&direct_accept(req.message_id)).await;
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

async fn start_pooled(
    connect_timeout_ms: u64,
    dials: Arc<AtomicUsize>,
    peer_tx: mpsc::UnboundedSender<LoopbackWebSocket>,
) -> (ScTransport<LoopbackWebSocket>, LoopbackWebSocket) {
    let (client, hub) = LoopbackWebSocket::pair();
    let dialer = counting_handshake_dialer(dials, peer_tx);
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(connect_timeout_ms)
        .with_direct_discovery(true)
        .with_direct_dialer(dialer);
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    (transport, hub)
}

#[test]
fn redial_backoff_progression_and_cap() {
    assert_eq!(redial_backoff_delay(0), DIRECT_REDIAL_INITIAL_BACKOFF);
    assert_eq!(redial_backoff_delay(1), Duration::from_millis(200));
    assert_eq!(redial_backoff_delay(2), Duration::from_millis(400));
    assert_eq!(redial_backoff_delay(3), Duration::from_millis(800));
    assert_eq!(redial_backoff_delay(4), Duration::from_millis(1600));
    assert_eq!(redial_backoff_delay(5), Duration::from_millis(3200));
    assert_eq!(redial_backoff_delay(6), DIRECT_REDIAL_MAX_BACKOFF);
    assert_eq!(redial_backoff_delay(7), DIRECT_REDIAL_MAX_BACKOFF);
    assert_eq!(redial_backoff_delay(u32::MAX), DIRECT_REDIAL_MAX_BACKOFF);
    assert!(DIRECT_REDIAL_INITIAL_BACKOFF <= DIRECT_REDIAL_MAX_BACKOFF);
}

#[test]
fn redial_backoff_table_bounds_and_expiry_without_sleep() {
    let mut table = RedialBackoff::new();
    let now = Instant::now();
    let uri = "wss://peer.example/sc".to_owned();
    assert!(!table.is_backed_off(&uri, now));
    table.record_failure(uri.clone(), now);
    assert!(table.is_backed_off(&uri, now));
    assert_eq!(table.len(), 1);
    let after_window = now + DIRECT_REDIAL_MAX_BACKOFF + Duration::from_secs(1);
    assert!(!table.is_backed_off(&uri, after_window));
    // Second failure advances the window (still backed off at the first
    // deadline) and success clears the entry.
    table.record_failure(uri.clone(), after_window);
    assert!(table.is_backed_off(&uri, after_window));
    table.record_success(&uri);
    assert!(!table.is_backed_off(&uri, after_window));
    assert_eq!(table.len(), 0);
    // FIFO bound: oldest tracked URI evicts first while over cap.
    for i in 0..DIRECT_REDIAL_MAX_ENTRIES + 5 {
        table.record_failure(format!("wss://peer{i}.example/sc"), now);
    }
    assert_eq!(table.len(), DIRECT_REDIAL_MAX_ENTRIES);
    assert!(!table.is_backed_off("wss://peer0.example/sc", now));
    assert!(table.is_backed_off(
        &format!("wss://peer{}.example/sc", DIRECT_REDIAL_MAX_ENTRIES + 4),
        now
    ));
}

#[test]
fn direct_pool_hit_miss_eviction_idle_expiry_without_sleep() {
    let mut pool: DirectPool<LoopbackWebSocket> = DirectPool::new();
    let now = Instant::now();
    let vmac = [0x22; 6];
    assert!(pool.get(&vmac, now).is_none());
    let (client, _peer) = LoopbackWebSocket::pair();
    pool.insert_test_entry(
        vmac,
        Arc::new(client),
        "wss://peer.example/sc".to_owned(),
        now,
    );
    assert_eq!(pool.len(), 1);
    assert!(pool.get(&vmac, now).is_some());
    let expired = now + DIRECT_POOL_IDLE_TTL + Duration::from_secs(1);
    assert!(pool.get(&vmac, expired).is_none());
    assert_eq!(pool.len(), 0);
    for i in 0..DIRECT_POOL_MAX_ENTRIES + 3 {
        let mut entry_vmac = [0u8; 6];
        entry_vmac[0] = (i & 0xFF) as u8;
        entry_vmac[1] = ((i >> 8) & 0xFF) as u8;
        let (c, _p) = LoopbackWebSocket::pair();
        pool.insert_test_entry(
            entry_vmac,
            Arc::new(c),
            format!("wss://peer{i}.example/sc"),
            now,
        );
    }
    assert_eq!(pool.len(), DIRECT_POOL_MAX_ENTRIES);
    assert!(pool.get(&[0, 0, 0, 0, 0, 0], now).is_none());
}

#[tokio::test]
async fn pooled_reuse_across_sends_dials_once() {
    let dials = Arc::new(AtomicUsize::new(0));
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
    let (mut transport, hub) = start_pooled(800, dials.clone(), peer_tx).await;
    let (first_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        assert_eq!(ar.function, ScFunction::AddressResolution);
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    first_res.unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    let peer = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .unwrap()
        .expect("direct peer missing");
    let first_bytes = timeout(Duration::from_secs(2), peer.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&first_bytes).unwrap().function,
        ScFunction::EncapsulatedNpdu
    );
    // Second send hits the URI cache and the pooled connection: no new AR,
    // no new dial, hub stays silent, same peer receives the second NPDU.
    transport.send_unicast(NPDU, &TARGET).await.unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    assert!(hub_try_recv(&hub).await.is_none());
    let second_bytes = timeout(Duration::from_secs(2), peer.recv())
        .await
        .unwrap()
        .unwrap();
    let second = decode_sc_message(&second_bytes).unwrap();
    assert_eq!(second.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(second.payload.as_ref(), NPDU);
    assert_eq!(
        transport
            .direct_shared_test_pool_len()
            .await
            .expect("direct must stay enabled"),
        1
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn failing_uri_records_backoff_and_falls_back_to_hub() {
    let dials = Arc::new(AtomicUsize::new(0));
    let dials_clone = dials.clone();
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(300)
        .with_direct_discovery(true)
        .with_direct_dialer(move |_uri: String| {
            let dials = dials_clone.clone();
            async move {
                dials.fetch_add(1, Ordering::SeqCst);
                Err::<LoopbackWebSocket, Error>(Error::Encoding("dial refused".into()))
            }
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
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    let npdu_bytes = hub_recv(&hub).await;
    assert_eq!(
        decode_sc_message(&npdu_bytes).unwrap().destination_vmac,
        Some(TARGET)
    );
    assert!(
        transport
            .direct_shared_test_is_backed_off("wss://peer.example/sc")
            .await
    );
    assert_eq!(transport.direct_shared_test_pool_len().await, Some(0));
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn default_off_still_hub_only_with_no_direct_state() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    assert!(transport.direct_shared_test_pool_len().await.is_none());
    transport.send_unicast(NPDU, &TARGET).await.unwrap();
    let bytes = hub_recv(&hub).await;
    let msg = decode_sc_message(&bytes).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some(TARGET));
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn concurrent_sends_share_pool_without_deadlock() {
    let dials = Arc::new(AtomicUsize::new(0));
    let (peer_tx, mut peer_rx) = mpsc::unbounded_channel();
    let (transport, hub) = start_pooled(800, dials.clone(), peer_tx).await;
    let (prime_res, ()) = tokio::join!(transport.send_unicast(NPDU, &TARGET), async {
        let ar_bytes = hub_recv(&hub).await;
        let ar = decode_sc_message(&ar_bytes).unwrap();
        hub.send(&ack_for(ar.message_id, b"wss://peer.example/sc"))
            .await
            .unwrap();
    });
    prime_res.unwrap();
    let peer = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .unwrap()
        .expect("direct peer missing");
    let _ = timeout(Duration::from_secs(2), peer.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    let (r1, r2, r3, r4) = tokio::join!(
        timeout(
            Duration::from_secs(5),
            transport.send_unicast(NPDU, &TARGET)
        ),
        timeout(
            Duration::from_secs(5),
            transport.send_unicast(NPDU, &TARGET)
        ),
        timeout(
            Duration::from_secs(5),
            transport.send_unicast(NPDU, &TARGET)
        ),
        timeout(
            Duration::from_secs(5),
            transport.send_unicast(NPDU, &TARGET)
        ),
    );
    r1.expect("concurrent send timed out").unwrap();
    r2.expect("concurrent send timed out").unwrap();
    r3.expect("concurrent send timed out").unwrap();
    r4.expect("concurrent send timed out").unwrap();
    // All four reused the pool: the prime completed first, so no redial.
    // Concurrent sends may duplicate a dial only in the pre-insert race
    // window; here the pool was already warm.
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    for _ in 0..4 {
        let bytes = timeout(Duration::from_secs(2), peer.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decode_sc_message(&bytes).unwrap().function,
            ScFunction::EncapsulatedNpdu
        );
    }
    assert!(hub_try_recv(&hub).await.is_none());
    let mut transport = transport;
    transport.stop().await.unwrap();
}
