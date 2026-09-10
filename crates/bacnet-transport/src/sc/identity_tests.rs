use super::*;
use std::sync::atomic::AtomicUsize;

// Deliberately sparse/non-RFC-shaped TEST identity and non-Random-48 VMAC.
// The startup policy rejects only whole-zero UUIDs and the two reserved VMACs.
const UUID: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
const VMAC: Vmac = [0xf1, 0, 0, 0, 0, 3];
const UUID_ERROR: &str = "SC device UUID is all-zero";
const VMAC_ERROR: &str = "SC VMAC is zero or broadcast";

#[derive(Debug, Default)]
struct Counts {
    sends: AtomicUsize,
    receives: AtomicUsize,
    drops: AtomicUsize,
    primary_dials: AtomicUsize,
    failover_dials: AtomicUsize,
}

struct CountedSocket {
    inner: LoopbackWebSocket,
    counts: Arc<Counts>,
}

impl WebSocketPort for CountedSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.counts.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.send(data).await
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.counts.receives.fetch_add(1, Ordering::SeqCst);
        self.inner.recv().await
    }
}

impl Drop for CountedSocket {
    fn drop(&mut self) {
        self.counts.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn fixture(vmac: Vmac) -> (ScTransport<CountedSocket>, LoopbackWebSocket, Arc<Counts>) {
    let (client, hub) = LoopbackWebSocket::pair();
    let (failover, _) = LoopbackWebSocket::pair();
    let counts = Arc::new(Counts::default());
    let transport = ScTransport::new(
        CountedSocket {
            inner: client,
            counts: counts.clone(),
        },
        vmac,
    )
    .with_connect_timeout_ms(20)
    .with_failover(CountedSocket {
        inner: failover,
        counts: counts.clone(),
    })
    .with_connector({
        let counts = counts.clone();
        move || {
            counts.primary_dials.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Encoding("unexpected primary dial".into())) }
        }
    })
    .with_failover_connector({
        let counts = counts.clone();
        move || {
            counts.failover_dials.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Encoding("unexpected failover dial".into())) }
        }
    });
    (transport, hub, counts)
}

fn assert_untouched(
    transport: &ScTransport<CountedSocket>,
    counts: &Counts,
    states: &watch::Receiver<ScConnectionState>,
    vmac: Vmac,
    uuid: [u8; 16],
) {
    for counter in [
        &counts.sends,
        &counts.receives,
        &counts.drops,
        &counts.primary_dials,
        &counts.failover_dials,
    ] {
        assert_eq!(counter.load(Ordering::SeqCst), 0, "{counts:?}");
    }
    assert!(!states.has_changed().unwrap());
    assert_eq!(*states.borrow(), ScConnectionState::Disconnected);
    assert!(transport.connection().is_none());
    assert!(transport.ws.is_some());
    assert!(transport.failover_ws.is_some());
    assert!(transport.primary_connector.is_some());
    assert!(transport.failover_connector.is_some());
    assert!(transport.ws_shared.is_none());
    assert!(transport.recv_task.is_none());
    assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
    assert_eq!(transport.max_apdu_length(), DEFAULT_MAX_APDU_LENGTH);
    assert_eq!(transport.local_mac(), vmac);
    assert_eq!(transport.device_uuid, uuid);
}

async fn reject_identity(vmac: Vmac, uuid: Option<[u8; 16]>, expected: &str) {
    let (mut transport, hub, counts) = fixture(vmac);
    if let Some(uuid) = uuid {
        transport = transport.with_device_uuid(uuid);
    }
    let states = transport.connection_state_changes();
    for _ in 0..2 {
        let error = transport.start().await.unwrap_err();
        assert!(
            matches!(&error, Error::Encoding(message) if message == expected),
            "expected {expected}, got {error:?}; counts={counts:?}; state={:?}; connection={}",
            *states.borrow(),
            transport.connection().is_some()
        );
        tokio::task::yield_now().await;
        assert_untouched(&transport, &counts, &states, vmac, uuid.unwrap_or([0; 16]));
    }
    if expected == UUID_ERROR {
        // Repair via the EXISTING public consuming setter, on the SAME owned W.
        transport = transport.with_device_uuid(UUID);
        let (started, ()) = tokio::join!(transport.start(), accept_identity(&hub, vmac, UUID));
        let mut rx = started.unwrap();
        assert_eq!(*states.borrow(), ScConnectionState::Connected);
        assert_eq!(counts.sends.load(Ordering::SeqCst), 1);
        assert_eq!(counts.drops.load(Ordering::SeqCst), 0);
        assert_eq!(counts.primary_dials.load(Ordering::SeqCst), 0);
        assert_eq!(counts.failover_dials.load(Ordering::SeqCst), 0);
        assert_eq!(
            transport.connection().unwrap().lock().await.device_uuid,
            UUID
        );
        transport
            .send_unicast(&[1, 2, 3], &[0x33; 6])
            .await
            .unwrap();
        let wire = recv(&hub).await;
        assert_eq!(&wire[..2], &[1, 4]);
        assert_eq!(&wire[4..], &[0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 1, 2, 3]);
        hub.send(&[
            1, 8, 0x12, 0x34, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 4, 5, 6,
        ])
        .await
        .unwrap();
        let npdu = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(npdu.npdu.as_ref(), &[4, 5, 6]);
        assert_eq!(npdu.source_mac.as_slice(), &[0x33; 6]);
        transport.stop().await.unwrap();
        assert_eq!(*states.borrow(), ScConnectionState::Disconnected);
        assert!(rx.recv().await.is_none());
        assert!(transport.recv_task.is_none());
        assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
        assert_eq!(counts.drops.load(Ordering::SeqCst), 2);
    }
    // No public VMAC repair is promised; dropping an unstarted rejected transport
    // releases both sockets. For the repaired case stop already joined/dropped them.
    drop(transport);
    assert_eq!(counts.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn missing_uuid_rejected_without_io_and_repaired_on_same_socket() {
    reject_identity(VMAC, None, UUID_ERROR).await;
}

#[tokio::test]
async fn explicit_zero_uuid_rejected_without_io_and_repaired_on_same_socket() {
    reject_identity(VMAC, Some([0; 16]), UUID_ERROR).await;
}

#[tokio::test]
async fn zero_vmac_rejected_without_io_or_socket_consumption() {
    reject_identity([0; 6], Some(UUID), VMAC_ERROR).await;
}

#[tokio::test]
async fn broadcast_vmac_rejected_without_io_or_socket_consumption() {
    reject_identity([0xff; 6], Some(UUID), VMAC_ERROR).await;
}

#[tokio::test]
async fn reconnect_then_heartbeat_then_identity_error_precedence() {
    let (transport, _hub, counts) = fixture([0; 6]);
    let mut transport = transport
        .with_heartbeat_interval_ms(0)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 0,
            max_delay_ms: 1,
            max_retries: 1,
        });
    let states = transport.connection_state_changes();
    let error = transport.start().await.unwrap_err();
    assert!(matches!(error, Error::OutOfRange(message) if message.contains("reconnect")));
    assert_untouched(&transport, &counts, &states, [0; 6], [0; 16]);
    transport = transport.with_reconnect(ScReconnectConfig::default());
    let error = transport.start().await.unwrap_err();
    assert!(matches!(error, Error::OutOfRange(message) if message.contains("heartbeat interval")));
    assert_untouched(&transport, &counts, &states, [0; 6], [0; 16]);
    transport = transport.with_heartbeat_interval_ms(30_000);
    let error = transport.start().await.unwrap_err();
    assert!(matches!(error, Error::Encoding(message) if message == UUID_ERROR));
    assert_untouched(&transport, &counts, &states, [0; 6], [0; 16]);
}

async fn recv(ws: &impl WebSocketPort) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(1), ws.recv())
        .await
        .unwrap()
        .unwrap()
}

// Independent AB.2.10 wire oracle: four-byte header, VMAC, UUID, two budgets.
// No production encoder/decoder is used to inspect the identity bytes.
pub(in crate::sc) fn assert_request_uuid(wire: &[u8], uuid: [u8; 16]) {
    assert_eq!(wire.len(), 30);
    assert_eq!(&wire[..2], &[6, 0]);
    assert_eq!(&wire[10..26], &uuid);
}

async fn accept_identity(ws: &impl WebSocketPort, vmac: Vmac, uuid: [u8; 16]) {
    let wire = recv(ws).await;
    assert_request_uuid(&wire, uuid);
    assert_eq!(&wire[4..10], &vmac);
    assert_eq!(&wire[26..], &[0x16, 0x49, 0x05, 0xc4]); // local budgets: 5705/1476
    let mut accept = vec![7, 0, wire[2], wire[3]];
    accept.extend_from_slice(&[0x10; 6]);
    accept.extend_from_slice(&[0x48; 16]);
    accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
    ws.send(&accept).await.unwrap();
}
