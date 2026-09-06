use super::deadline_test_support::*;
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn hub_timeout_configuration_checks_phase_boundaries() {
    let defaults = ScHubHandshakeTimeouts::default();
    assert_eq!(defaults.tls(), Duration::from_secs(10));
    assert_eq!(defaults.websocket_upgrade(), Duration::from_secs(10));
    assert_eq!(defaults.connect_request(), Duration::from_secs(10));
    for (tls, ws, connect) in [
        (
            Duration::from_nanos(1),
            Duration::from_nanos(1),
            Duration::from_secs(5),
        ),
        (
            Duration::from_secs(300),
            Duration::from_secs(300),
            Duration::from_secs(300),
        ),
    ] {
        let config = ScHubHandshakeTimeouts::new(tls, ws, connect).unwrap();
        assert_eq!(
            (
                config.tls(),
                config.websocket_upgrade(),
                config.connect_request()
            ),
            (tls, ws, connect)
        );
    }
    for phase in 0..3 {
        let minimum = if phase == 2 {
            Duration::from_secs(5)
        } else {
            Duration::from_nanos(1)
        };
        for invalid in [
            Duration::ZERO,
            minimum - Duration::from_nanos(1),
            Duration::from_secs(300) + Duration::from_nanos(1),
            Duration::MAX,
        ] {
            let mut values = [Duration::from_secs(10); 3];
            values[phase] = invalid;
            assert!(
                matches!(
                    ScHubHandshakeTimeouts::new(values[0], values[1], values[2]),
                    Err(bacnet_types::error::Error::Encoding(_))
                ),
                "phase {phase}: {invalid:?}"
            );
        }
    }
}

#[tokio::test]
async fn hub_default_deadline_releases_silent_and_partial_tls() {
    let tls = TestTls::new();
    let mut hub = ScHub::start("127.0.0.1:0", tls.acceptor, [0x10; 6])
        .await
        .unwrap();
    let mut silent = TcpStream::connect(hub.local_addr().unwrap()).await.unwrap();
    let mut partial = TcpStream::connect(hub.local_addr().unwrap()).await.unwrap();
    partial
        .write_all(&[0x16, 0x03, 0x03, 0x00, 0x10, 0x01])
        .await
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(12), async {
        let mut a = [0; 1];
        let mut b = [0; 1];
        let (a, b) = tokio::join!(silent.read(&mut a), partial.read(&mut b));
        assert!(
            matches!(a, Ok(0) | Err(_)),
            "silent TLS remained open: {a:?}"
        );
        assert!(
            matches!(b, Ok(0) | Err(_)),
            "partial TLS remained open: {b:?}"
        );
    })
    .await;
    hub.stop().await;
    assert!(
        closed.is_ok(),
        "default TLS deadline did not release stalled connections"
    );
}

#[tokio::test]
async fn hub_http_upgrade_has_independent_deadline_after_tls_success() {
    let tls = TestTls::new();
    let budgets = ScHubHandshakeTimeouts::new(
        Duration::from_secs(2),
        Duration::from_millis(150),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut hub = ScHub::start_with_uuid_and_timeouts(
        "127.0.0.1:0",
        tls.acceptor.clone(),
        [0x10; 6],
        [0x10; 16],
        budgets,
    )
    .await
    .unwrap();
    for partial in [false, true] {
        let tcp = TcpStream::connect(hub.local_addr().unwrap()).await.unwrap();
        // More than the entire HTTP budget passes before TLS succeeds.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut stream = tls.connect_tls(tcp).await;
        if partial {
            stream
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
                .await
                .unwrap();
        }
        let mut byte = [0; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(50), stream.read(&mut byte))
                .await
                .is_err(),
            "HTTP budget started before TLS success"
        );
        let released =
            tokio::time::timeout(Duration::from_millis(300), stream.read(&mut byte)).await;
        assert!(
            matches!(released, Ok(Ok(0) | Err(_))),
            "HTTP upgrade did not release stalled peer: {released:?}"
        );
    }
    hub.stop().await;
}

#[tokio::test]
async fn hub_upgraded_idle_peer_gets_close_at_connect_deadline() {
    let tls = TestTls::new();
    let budgets = ScHubHandshakeTimeouts::new(
        Duration::from_secs(2),
        Duration::from_secs(1),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut hub = ScHub::start_with_uuid_and_timeouts(
        "127.0.0.1:0",
        tls.acceptor.clone(),
        [0x10; 6],
        [0x10; 16],
        budgets,
    )
    .await
    .unwrap();
    let mut ws = tls.websocket(hub.local_addr().unwrap()).await;
    let result = tokio::time::timeout(Duration::from_secs(6), ws.next()).await;
    hub.stop().await;
    assert!(
        matches!(result, Ok(Some(Ok(Message::Close(_))))),
        "Connect wait must expire with Close, got {result:?}"
    );
}
