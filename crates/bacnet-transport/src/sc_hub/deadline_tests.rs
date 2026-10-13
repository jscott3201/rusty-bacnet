use super::deadline_test_support::*;
use super::*;
use crate::port_ownership::connect_loopback;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

// The tests below run on the paused clock, which only `advance` moves:
// `poll_io` keeps it from auto-advancing while loopback I/O is in flight, so
// a runner stall can neither expire a hub deadline early nor outlast a wait
// for one (#1042). Tokio rounds timer deadlines up to the next millisecond,
// never down, so each boundary is checked 1 ms before the budget and 1 ms
// after it.

#[tokio::test(start_paused = true)]
async fn hub_default_deadline_releases_silent_and_partial_tls() {
    let tls = TestTls::new();
    let budget = ScHubHandshakeTimeouts::default().tls();
    let mut hub = ScHub::start("127.0.0.1:0", tls.hub_config, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let address = hub.local_addr().unwrap();
    let mut silent = poll_io(connect_loopback(address)).await;
    let mut partial = poll_io(connect_loopback(address)).await;
    poll_io(partial.write_all(&[0x16, 0x03, 0x03, 0x00, 0x10, 0x01]))
        .await
        .unwrap();
    // The TLS budget starts at admission, not at TCP connect completion.
    until(|| hub.active.load(Ordering::Acquire) == 2).await;
    tokio::time::advance(budget - Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(
        hub.active.load(Ordering::Acquire),
        2,
        "TLS deadline expired early"
    );
    tokio::time::advance(Duration::from_millis(2)).await;
    let mut a = [0; 1];
    let mut b = [0; 1];
    let (a, b) = poll_io(async { tokio::join!(silent.read(&mut a), partial.read(&mut b)) }).await;
    assert!(
        matches!(a, Ok(0) | Err(_)),
        "silent TLS remained open: {a:?}"
    );
    assert!(
        matches!(b, Ok(0) | Err(_)),
        "partial TLS remained open: {b:?}"
    );
    poll_io(hub.stop()).await;
    assert_eq!(
        hub.status().await.outcomes,
        ScHubOutcomeCounts {
            tls_timeouts: 2,
            ..ScHubOutcomeCounts::default()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn hub_http_upgrade_has_independent_deadline_after_tls_success() {
    let tls = TestTls::new();
    let budgets = ScHubHandshakeTimeouts::new(
        Duration::from_secs(2),
        Duration::from_millis(150),
        Duration::from_secs(5),
    )
    .unwrap();
    let upgrade = budgets.websocket_upgrade();
    let mut hub = ScHub::start_with_uuid_and_timeouts(
        "127.0.0.1:0",
        tls.hub_config.clone(),
        [0x10; 6],
        [0x10; 16],
        budgets,
    )
    .await
    .unwrap();
    for (armed, partial) in [(1, false), (2, true)] {
        let tcp = poll_io(connect_loopback(hub.local_addr().unwrap())).await;
        until(|| hub.active.load(Ordering::Acquire) == 1).await;
        // More than the entire HTTP budget passes before TLS succeeds.
        tokio::time::advance(upgrade + Duration::from_millis(100)).await;
        let mut stream = poll_io(tls.connect_tls(tcp)).await;
        // TLS 1.3 completes on the hub after it does here, so wait for the
        // hub to arm the HTTP budget before moving the clock.
        until(|| {
            hub.admission
                .upgrade_deadlines_armed
                .load(Ordering::Acquire)
                == armed
        })
        .await;
        if partial {
            poll_io(stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n"))
                .await
                .unwrap();
        }
        tokio::time::advance(upgrade - Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(
            hub.active.load(Ordering::Acquire),
            1,
            "HTTP budget started before TLS success"
        );
        tokio::time::advance(Duration::from_millis(2)).await;
        let mut byte = [0; 1];
        let released = poll_io(stream.read(&mut byte)).await;
        assert!(
            matches!(released, Ok(0) | Err(_)),
            "HTTP upgrade did not release stalled peer: {released:?}"
        );
        until(|| hub.active.load(Ordering::Acquire) == 0).await;
    }
    poll_io(hub.stop()).await;
    assert_eq!(
        hub.status().await.outcomes,
        ScHubOutcomeCounts {
            websocket_timeouts: 2,
            ..ScHubOutcomeCounts::default()
        }
    );
}

#[tokio::test(start_paused = true)]
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
        tls.hub_config.clone(),
        [0x10; 6],
        [0x10; 16],
        budgets,
    )
    .await
    .unwrap();
    // The hub arms the Connect wait in the poll that writes its 101 response,
    // so the upgrade completing here orders it before the advance.
    let mut ws = poll_io(tls.websocket(hub.local_addr().unwrap())).await;
    tokio::time::advance(budgets.connect_request() - Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(
        hub.active.load(Ordering::Acquire),
        1,
        "Connect wait expired early"
    );
    tokio::time::advance(Duration::from_millis(2)).await;
    let result = poll_io(ws.next()).await;
    poll_io(hub.stop()).await;
    assert!(
        matches!(result, Some(Ok(Message::Close(_)))),
        "Connect wait must expire with Close, got {result:?}"
    );
    assert_eq!(
        hub.status().await.outcomes,
        ScHubOutcomeCounts {
            connect_timeouts: 1,
            ..ScHubOutcomeCounts::default()
        }
    );
}
