//! Real TLS and the production TlsWebSocket write mutex, NOT OS backpressure.

use super::*;
use crate::port::TransportPort;
use crate::sc::{ScConnectionState, ScReconnectConfig, ScTransport};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};

struct HubSubprotocol;
impl Callback for HubSubprotocol {
    fn on_request(self, _: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        response.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
        );
        Ok(response)
    }
}

#[derive(Default)]
struct Counts {
    sends: AtomicUsize,
    receives: AtomicUsize,
    nak_entered: AtomicUsize,
    nak_dropped: AtomicUsize,
}
struct ObservedTls {
    ws: Arc<TlsWebSocket>,
    counts: Arc<Counts>,
}
struct DropNak<'a>(&'a AtomicUsize);
impl Drop for DropNak<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl WebSocketPort for ObservedTls {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.counts.sends.fetch_add(1, Ordering::SeqCst);
        let _nak = if data[0] == 0 {
            self.counts.nak_entered.fetch_add(1, Ordering::SeqCst);
            Some(DropNak(&self.counts.nak_dropped))
        } else {
            None
        };
        self.ws.send(data).await
    }
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.counts.receives.fetch_add(1, Ordering::SeqCst);
        self.ws.recv().await
    }
}

#[tokio::test]
async fn rejection_deadline_tls_production_write_lock_is_cancelled_without_later_flush() {
    let mut mu = vec![1, 10, 0x22, 0x33];
    mu.extend_from_slice(&[0x22; 6]);
    mu.extend_from_slice(&[0xE2, 0, 0, 0x1F, 1, 0, 0x30]);
    let mut empty = vec![1, 8, 0x22, 0x33];
    empty.extend_from_slice(&[0x22; 6]);
    for wire in [
        vec![0x0A, 0, 0x22, 0x33, 0x42],
        vec![1, 0, 0x22, 0x33, 1, 0, 0x30],
        mu,
        empty,
        vec![0x42, 3, 0x22, 0x33, 0xE2, 0, 0, 0x1F, 0x7E, 0, 0],
    ] {
        tokio::time::timeout(Duration::from_secs(6), exercise(wire))
            .await
            .unwrap();
    }
}

async fn exercise(wire: Vec<u8>) {
    let (node, server) = test_tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let peer = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
        assert_eq!(
            tls.get_ref().1.protocol_version(),
            Some(ProtocolVersion::TLSv1_3)
        );
        tokio_tungstenite::accept_hdr_async(tls, HubSubprotocol)
            .await
            .unwrap()
    };
    let (ws, mut peer) = tokio::join!(TlsWebSocket::connect(&url, node), peer);
    let ws = Arc::new(ws.unwrap());
    let counts = Arc::new(Counts::default());
    let mut transport = ScTransport::new(ObservedTls { ws: ws.clone(), counts: counts.clone() }, [1; 6])
        .with_device_uuid([1; 16])
        // Public production timing, no clock replacement or private override.
        .with_heartbeat_interval_ms(3000).with_heartbeat_timeout_ms(3400)
        .with_reconnect(ScReconnectConfig { initial_delay_ms: 20, max_delay_ms: 20, max_retries: 1 });
    let accept = async {
        let request = peer.next().await.unwrap().unwrap().into_data();
        assert_eq!(&request[..4], &[6, 0, 0, 1]);
        let mut accept = vec![7, 0, 0, 1];
        accept.extend_from_slice(&[0x10; 6]);
        accept.extend_from_slice(&[0x33; 16]);
        accept.extend_from_slice(&[5, 0xC4, 5, 0xC4]);
        peer.send(Message::Binary(accept.into())).await.unwrap();
    };
    let (rx, ()) = tokio::join!(transport.start(), accept);
    let mut rx = rx.unwrap();
    let started = std::time::Instant::now();
    let mut states = transport.connection_state_changes();
    let write = ws.write.lock().await; // Gate the real production write lock.
    peer.send(Message::Binary(wire.into())).await.unwrap();
    while counts.nak_entered.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(*states.borrow(), ScConnectionState::Connected);
    while *states.borrow_and_update() != ScConnectionState::Disconnected {
        states.changed().await.unwrap();
    }
    assert!(started.elapsed() >= Duration::from_millis(3300));
    assert_eq!(counts.nak_dropped.load(Ordering::SeqCst), 1);
    assert!(rx.try_recv().is_err());
    let before = (
        counts.sends.load(Ordering::SeqCst),
        counts.receives.load(Ordering::SeqCst),
    );
    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
    // Let the configured reconnect decision run, then stop while the lock is
    // STILL held. Neither operation may attempt old-socket I/O.
    tokio::time::sleep(Duration::from_millis(60)).await;
    tokio::time::timeout(Duration::from_millis(250), transport.stop())
        .await
        .unwrap()
        .unwrap();
    drop(write);
    assert!(tokio::time::timeout(Duration::from_millis(50), peer.next())
        .await
        .is_err());
    assert_eq!(
        (
            counts.sends.load(Ordering::SeqCst),
            counts.receives.load(Ordering::SeqCst)
        ),
        before
    );
    // ws deliberately retains the driver; only logical retirement was asserted.
}
