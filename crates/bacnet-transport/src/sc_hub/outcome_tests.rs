//! Observable outcomes over real TLS, with deterministic I/O failure seams.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{barrier, raw, send, stopped};
use super::*;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Error;

#[tokio::test]
async fn ordered_accept_capacity_counts_only_selected_total_limit() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start_with_limits(
        &tls,
        ScHubHandshakeTimeouts::default(),
        ScHubAdmissionLimits::new(1, 1).unwrap(),
    )
    .await;
    let mut established = tls.websocket(hub.address).await;
    established
        .send(request([0x42; 6], [0x42; 16]))
        .await
        .unwrap();
    assert!(
        matches!(poll_io(established.next()).await, Some(Ok(Message::Binary(data))) if data[0] == 7)
    );
    let _idle = TcpStream::connect(hub.address).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 2).await;
    let mut refused = TcpStream::connect(hub.address).await.unwrap();
    assert!(matches!(
        poll_io(refused.read(&mut [0; 1])).await,
        Ok(0) | Err(_)
    ));
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts {
            total_active_accept_drops: 1,
            ..ScHubOutcomeCounts::default()
        }
    );
    stopped(&mut hub).await;
    assert_eq!(
        hub.hub.status().await.outcomes.tls_timeouts,
        0,
        "stop cancellation is not a timeout"
    );
}

#[tokio::test]
async fn eligible_npdu_and_opaque_count_missing_and_limits_but_not_self_or_malformed() {
    for function in [0x01, 0x0D] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut source = ControlledPeer::open(&tls, &hub).await;
        source.connect(0x42).await;
        let mut target = ControlledPeer::open(&tls, &hub).await;
        target.connect(0x43).await;
        // Independently control the peer-advertised receive limit fixture.
        {
            let mut map = hub.clients.lock().await;
            let client = map.get_mut(&[0x43; 6]).unwrap();
            client.max_npdu = 1;
            client.max_bvlc = 10;
        }
        for destination in [[0x44; 6], [0x43; 6], [0x42; 6]] {
            send(
                &mut source,
                raw(function, 7, None, Some(destination), 0, &[1, 0]),
            )
            .await;
        }
        if function == 0x01 {
            // NPDU self-addressing retains its existing echo behavior; it is
            // outside the diagnostic unicast categories.
            assert_eq!(
                super::unknown_transit_tests::recv(&mut source).await,
                raw(function, 7, Some([0x42; 6]), None, 0, &[1, 0])
            );
        }
        // An origin is invalid on ingress and must not become a missing-target event.
        send(
            &mut source,
            raw(function, 8, Some([0x45; 6]), Some([0x44; 6]), 0, &[1, 0]),
        )
        .await;
        barrier(&mut source).await;
        assert_eq!(
            hub.hub.status().await.outcomes,
            ScHubOutcomeCounts {
                unicast_no_target: 1,
                unicast_target_limit: 1,
                ..ScHubOutcomeCounts::default()
            }
        );
        stopped(&mut hub).await;
    }
}

struct Failure;
impl relay_send::RelayIo for Failure {
    async fn send(&self, _: &mut WsSink, _: Message) -> Result<(), Error> {
        Err(Error::Io(std::io::ErrorKind::WouldBlock.into()))
    }
}

#[tokio::test]
async fn unicast_error_counts_but_cancelled_and_retired_attempts_do_not() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    let target =
        HubRelaySink::capture([0x42; 6], hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    let frame = Message::Binary(Bytes::new());
    let budget = Duration::from_secs(5);
    relay_send::unicast(
        [0x43; 6],
        &target,
        &hub.clients,
        frame.clone(),
        budget,
        &Failure,
    )
    .await;
    let held = target.sink.lock().await;
    {
        let mut cancelled = Box::pin(relay_send::unicast(
            [0x43; 6],
            &target,
            &hub.clients,
            frame.clone(),
            budget,
            &Failure,
        ));
        assert!(futures_util::poll!(&mut cancelled).is_pending());
    }
    drop(held);
    target.closed.store(true, Ordering::Release);
    relay_send::unicast([0x43; 6], &target, &hub.clients, frame, budget, &Failure).await;
    assert_eq!(
        hub.hub.status().await.outcomes,
        ScHubOutcomeCounts {
            unicast_send_error: 1,
            ..ScHubOutcomeCounts::default()
        }
    );
    stopped(&mut hub).await;
}
