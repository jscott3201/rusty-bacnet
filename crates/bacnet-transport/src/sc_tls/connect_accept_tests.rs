use super::*;
use crate::port::TransportPort;
use crate::sc::{ScConnection, ScConnectionState, ScTransport};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const CONNECT_WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
enum Outcome {
    Recover,
    Expire,
    InspectPending,
}

#[derive(Default)]
struct Progress {
    sends: AtomicUsize,
    receives: AtomicUsize,
}

struct ObservedTls {
    inner: TlsWebSocket,
    progress: Arc<Progress>,
}

impl WebSocketPort for ObservedTls {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.progress.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.send(data).await
    }
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        // Re-entering recv proves the preceding TLS message was read and the
        // handshake chose to wait again. The initial receive is counted too.
        self.progress.receives.fetch_add(1, Ordering::SeqCst);
        self.inner.recv().await
    }
}

// Real TLS needs reactor progress while the test controls virtual time.
// Keeping this task runnable prevents Tokio from skipping to the deadline;
// the wall-clock bound still fails a broken I/O or progress barrier.
async fn poll_io<F: std::future::Future>(future: F) -> F::Output {
    tokio::pin!(future);
    let wall_start = std::time::Instant::now();
    loop {
        if let std::task::Poll::Ready(value) = futures_util::poll!(&mut future) {
            return value;
        }
        assert!(
            wall_start.elapsed() < Duration::from_secs(5),
            "TLS progress stalled"
        );
        tokio::task::yield_now().await;
    }
}

fn assert_no_peer(conn: &ScConnection) {
    assert_eq!(conn.hub_vmac, None);
    assert_eq!(conn.hub_device_uuid, None);
    assert_eq!(
        (conn.hub_max_bvlc_length, conn.hub_max_npdu_length),
        (1476, 1476)
    );
}

// Tungstenite's handshake callback requires its unboxed HTTP error response.
#[allow(clippy::result_large_err)]
async fn check_invalid_accept(outcome: Outcome, field: std::ops::Range<usize>) {
    let (node, server) = test_tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let peer =
        async {
            let (tcp, _) = listener.accept().await.unwrap();
            let tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
            assert_eq!(
                tls.get_ref().1.protocol_version(),
                Some(ProtocolVersion::TLSv1_3)
            );
            tokio_tungstenite::accept_hdr_async(
            tls,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
                );
                Ok(response)
            },
        ).await.unwrap()
        };
    let (ws, mut peer) = tokio::join!(TlsWebSocket::connect(&url, node), peer);
    // TLS and WebSocket setup use the real clock. Only the BACnet connect wait
    // is paused: runner wall-clock stalls cannot consume its 500 ms budget.
    tokio::time::pause();
    let began = tokio::time::Instant::now();
    let progress = Arc::new(Progress::default());
    let mut transport = ScTransport::new(
        ObservedTls {
            inner: ws.unwrap(),
            progress: progress.clone(),
        },
        [0x02; 6],
    )
    .with_device_uuid([0x12; 16])
    .with_connect_timeout_ms(500);
    let mut state = transport.connection_state_changes();
    // Independent AB.2.11 fixture, not the product encoder. Nonzero UUID
    // bits are opaque; the valid peer advertises limits 8192 and 4096.
    let mut valid = vec![
        7, 0, 0, 1, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0x20, 0, 0x10, 0,
    ];
    valid[10..26].fill(0xff);
    let mut invalid = valid.clone();
    invalid[field].fill(0);
    let started = {
        let start = transport.start();
        tokio::pin!(start);
        let invalid_frames = async {
            let request = peer.next().await.unwrap().unwrap().into_data();
            assert_eq!(&request[..4], &[6, 0, 0, 1]);
            assert_eq!(&request[4..10], &[0x02; 6]);
            assert_eq!(&request[10..26], &[0x12; 16]);
            assert_eq!(*state.borrow_and_update(), ScConnectionState::Connecting);
            for received in 2..=4 {
                peer.send(Message::Binary(invalid.clone().into()))
                    .await
                    .unwrap();
                // The next receive starts only after this invalid frame was
                // consumed by the real TLS driver and rejected by the handshake.
                while progress.receives.load(Ordering::SeqCst) < received {
                    tokio::task::yield_now().await;
                }
                assert_eq!(
                    progress.sends.load(Ordering::SeqCst),
                    1,
                    "only Connect-Request was sent"
                );
                assert!(
                    matches!(futures_util::poll!(peer.next()), std::task::Poll::Pending),
                    "invalid Accept elicited a wire response or close"
                );
                assert!(
                    !state.has_changed().unwrap(),
                    "invalid Accept published state"
                );
                tokio::time::advance(Duration::from_millis(100)).await;
            }
            tokio::time::advance(Duration::from_millis(199)).await;
            assert_eq!(began.elapsed(), CONNECT_WAIT - Duration::from_millis(1));
            assert_eq!(*state.borrow(), ScConnectionState::Connecting);
            assert!(!state.has_changed().unwrap());
        };
        poll_io(async {
            tokio::select! {
                result = &mut start => panic!("connect completed before its deadline: {result:?}"),
                () = invalid_frames => {}
            }
        })
        .await;
        match outcome {
            Outcome::InspectPending => None,
            Outcome::Recover => {
                poll_io(peer.send(Message::Binary(valid.into())))
                    .await
                    .unwrap();
                Some(poll_io(&mut start).await)
            }
            Outcome::Expire => {
                // Cross the deadline by one timer-wheel tick (AB.6.2). There
                // must be no timeout before it and no fresh budget per frame.
                tokio::time::advance(Duration::from_millis(2)).await;
                let started = poll_io(&mut start).await;
                let end = poll_io(peer.next()).await;
                assert!(!matches!(end, Some(Ok(Message::Binary(_)))));
                Some(started)
            }
        }
    };
    let conn = transport.connection().unwrap().lock().await;
    assert_eq!(conn.local_vmac, [0x02; 6]);
    assert_eq!(conn.device_uuid, [0x12; 16]);
    match outcome {
        Outcome::Recover => {
            let _rx = started.unwrap().unwrap();
            assert_eq!(conn.state, ScConnectionState::Connected);
            assert_eq!(*state.borrow(), ScConnectionState::Connected);
            assert_eq!(conn.hub_vmac, Some([0x22; 6]));
            assert_eq!(conn.hub_device_uuid, Some([0xff; 16]));
            assert_eq!(
                (conn.hub_max_bvlc_length, conn.hub_max_npdu_length),
                (8192, 4096)
            );
        }
        Outcome::Expire => {
            assert!(matches!(started.unwrap(), Err(Error::Timeout(d)) if d == CONNECT_WAIT));
            assert_eq!(conn.state, ScConnectionState::Disconnected);
            assert_eq!(*state.borrow(), ScConnectionState::Disconnected);
            assert_no_peer(&conn);
        }
        Outcome::InspectPending => {
            // Dropping the pending start releases its mutable owner borrow,
            // allowing a snapshot before any timeout or valid Accept. This
            // cancellation is test observation, not a successful connection.
            assert_eq!(began.elapsed(), CONNECT_WAIT - Duration::from_millis(1));
            assert_eq!(conn.state, ScConnectionState::Connecting);
            assert_no_peer(&conn);
        }
    }
    drop(conn);
    tokio::time::resume();
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn nil_accept_tls_is_silent_until_later_valid_accept() {
    tokio::time::timeout(
        Duration::from_secs(5),
        check_invalid_accept(Outcome::Recover, 10..26),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn nil_accept_tls_expires_without_peer_identity_or_limits() {
    tokio::time::timeout(
        Duration::from_secs(5),
        check_invalid_accept(Outcome::Expire, 10..26),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn zero_limits_accept_tls_is_silent_until_later_valid_accept() {
    for field in [26..28, 28..30, 26..30] {
        tokio::time::timeout(
            Duration::from_secs(5),
            check_invalid_accept(Outcome::Recover, field),
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn zero_limits_accept_tls_expires_without_peer_identity_or_limits() {
    for field in [26..28, 28..30, 26..30] {
        tokio::time::timeout(
            Duration::from_secs(5),
            check_invalid_accept(Outcome::Expire, field),
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn invalid_accept_tls_keeps_peer_identity_and_limits_uncommitted_before_expiry() {
    for field in [10..26, 26..28, 28..30, 26..30] {
        tokio::time::timeout(
            Duration::from_secs(5),
            check_invalid_accept(Outcome::InspectPending, field),
        )
        .await
        .unwrap();
    }
}
