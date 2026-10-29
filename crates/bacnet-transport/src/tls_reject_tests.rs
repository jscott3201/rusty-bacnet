//! The post-alert drain: FIN first, then bounded by bytes, linger and deadline.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

use super::{ALERT_LINGER, DRAIN_LIMIT, close_after_alert};
use crate::port_ownership::connect_loopback;

/// A connected pair: (the refusing side, the peer).
async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (peer, accepted) = tokio::join!(
        connect_loopback(listener.local_addr().unwrap()),
        listener.accept()
    );
    (accepted.unwrap().0, peer)
}

#[tokio::test(start_paused = true)]
async fn a_silent_open_peer_is_cut_off_after_the_linger() {
    let (server, mut peer) = pair().await;
    let started = Instant::now();
    assert_eq!(
        close_after_alert(server, started + Duration::from_secs(10)).await,
        0
    );
    assert_eq!(started.elapsed(), ALERT_LINGER);
    // The peer saw FIN, not a reset.
    assert_eq!(peer.read(&mut [0; 16]).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_earlier_handshake_deadline_wins_over_the_linger() {
    let (server, _peer) = pair().await;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(200);
    close_after_alert(server, deadline).await;
    assert_eq!(started.elapsed(), Duration::from_millis(200));
}

// Real time: paused time would auto-advance toward the linger while the
// runtime waits for socket readiness.
#[tokio::test]
async fn a_peer_that_closes_ends_the_drain_at_once() {
    let (server, mut peer) = pair().await;
    let request = b"GET /hub HTTP/1.1\r\nHost: localhost\r\n\r\n";
    peer.write_all(request).await.unwrap();
    peer.shutdown().await.unwrap();
    let started = Instant::now();
    assert_eq!(
        close_after_alert(server, started + Duration::from_secs(10)).await,
        request.len()
    );
    // Ended by the peer's FIN, not by the linger.
    assert!(started.elapsed() < ALERT_LINGER);
}

#[tokio::test]
async fn a_peer_that_keeps_writing_is_cut_off_at_the_byte_cap() {
    let (server, mut peer) = pair().await;
    let writer = tokio::spawn(async move {
        let chunk = [0x16u8; 4096];
        let mut written = 0usize;
        while peer.write_all(&chunk).await.is_ok() {
            written += chunk.len();
            // Far past the cap means the drain never stopped.
            assert!(written < 64 * DRAIN_LIMIT, "drain kept reading");
        }
    });
    let started = Instant::now();
    let discarded = close_after_alert(server, started + Duration::from_secs(10)).await;
    assert!(
        (DRAIN_LIMIT..DRAIN_LIMIT + 8 * 1024).contains(&discarded),
        "discarded {discarded}"
    );
    assert!(started.elapsed() < ALERT_LINGER);
    // With the socket gone, the writer fails instead of running on.
    tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .unwrap()
        .unwrap();
}
