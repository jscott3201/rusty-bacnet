use super::*;

const PERIOD: Duration = Duration::from_millis(400);

async fn peer_and_transport() -> (UdpSocket, BipTransport) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: peer.local_addr().unwrap().port(),
        ttl: 2,
        renewal_interval: Some(PERIOD),
    });
    (peer, transport)
}

async fn registration(peer: &UdpSocket) -> std::net::SocketAddr {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut packet))
        .await
        .expect("scheduled registration did not arrive")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 2]);
    source
}

async fn observed(
    transport: &BipTransport,
    predicate: impl Fn(BvlcClientSnapshot) -> bool,
) -> BvlcClientSnapshot {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = transport.bvlc_client_snapshot();
            if predicate(snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("management observation did not arrive")
}

#[tokio::test]
async fn foreign_mode_validation_precedes_interface_or_socket_io() {
    for (ttl, interval) in [
        (0, None),
        (0, Some(Duration::from_millis(1))),
        (1, Some(Duration::ZERO)),
        (1, Some(Duration::from_secs(1))),
        (1, Some(Duration::from_secs(2))),
    ] {
        // A nonlocal interface would yield a transport error if validation ran late.
        let mut transport = BipTransport::new(Ipv4Addr::new(192, 0, 2, 1), 0, Ipv4Addr::BROADCAST);
        transport.register_as_foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: 47808,
            ttl,
            renewal_interval: interval,
        });
        let error = transport.start().await.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(error.to_string().contains("renewal interval"));
        assert!(transport.socket.is_none());
        assert!(!transport.bvlc_client_snapshot().running);
        assert_eq!(
            transport
                .bvlc_client_snapshot()
                .foreign_registration
                .attempts,
            0
        );
    }
}

#[tokio::test]
async fn foreign_attempt_results_renewal_and_restart_are_observable() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    assert!(transport.bvlc_client_snapshot().running);
    let source = registration(&peer).await;
    let pending = transport.bvlc_client_snapshot();
    assert_eq!(
        (
            pending.foreign_registration.attempts,
            pending.foreign_registration.accepted
        ),
        (1, 0)
    );
    assert_eq!(pending.foreign_registration.last_ttl, Some(2));
    assert_eq!(
        pending.foreign_registration.last_outcome,
        Some(ForeignRegistrationOutcome::Pending)
    );
    assert!(pending.foreign_registration.next_attempt_in.unwrap() <= PERIOD);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0x30], source)
        .await
        .unwrap();
    let refused = observed(&transport, |s| s.foreign_registration.refused == 1).await;
    assert_eq!(
        refused.foreign_registration.last_result,
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    );
    let renewed = tokio::time::Instant::now();
    assert_eq!(registration(&peer).await, source);
    assert!(renewed.elapsed() < Duration::from_secs(1));
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    let accepted = observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    assert_eq!(
        (
            accepted.register_foreign_device.sent,
            accepted.register_foreign_device.results
        ),
        (2, 2)
    );
    assert_eq!(
        accepted.foreign_registration.last_result,
        Some(BvlcResultCode::SUCCESSFUL_COMPLETION)
    );
    let worker = transport.registration_task.as_ref().unwrap().abort_handle();
    transport.stop().await.unwrap();
    assert!(worker.is_finished());
    let stopped = transport.bvlc_client_snapshot();
    assert!(!stopped.running);
    assert_eq!(stopped.foreign_registration.last_outcome, None);
    assert_eq!(stopped.foreign_registration.last_ttl, None);
    assert_eq!(stopped.foreign_registration.last_result, None);
    assert_eq!(stopped.register_foreign_device.last_result, None);
    assert_eq!(stopped.foreign_registration.next_attempt_in, None);
    assert_eq!(stopped.foreign_registration.accepted, 1);
    let _rx = transport.start().await.unwrap();
    let restarted = transport.bvlc_client_snapshot();
    assert_eq!(restarted.foreign_registration.last_outcome, None);
    assert_eq!(registration(&peer).await, source);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 2).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_silent_attempt_times_out_then_renews_without_claiming_acceptance() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    let source = registration(&peer).await;
    // Automatic response waiting is bounded by PERIOD, below the manual 3 s.
    let started = tokio::time::Instant::now();
    assert_eq!(registration(&peer).await, source);
    assert!(started.elapsed() < Duration::from_secs(1));
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.register_foreign_device.timeouts, 1);
    assert_eq!(snapshot.foreign_registration.accepted, 0);
    assert_eq!(snapshot.foreign_registration.refused, 0);
    assert_eq!(snapshot.foreign_registration.last_result, None);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_busy_worker_retries_after_manual_request_finishes() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    let target = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    {
        let manual = transport.read_bdt(&target);
        tokio::pin!(manual);
        // Own the slot before the newly spawned automatic worker can run.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(manual.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let mut packet = [0; 64];
        let (len, source) = tokio::select! {
            result = &mut manual => panic!("manual request finished early: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..len], &[0x81, 2, 0, 4]);
        let busy = observed(&transport, |s| s.register_foreign_device.busy == 1).await;
        assert_eq!(busy.register_foreign_device.sent, 0);
        assert_eq!(busy.foreign_registration.attempts, 0);
        assert!(busy.foreign_registration.next_attempt_in.unwrap() <= PERIOD);
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        manual.await.unwrap();
    }
    let source = registration(&peer).await;
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.foreign_registration.accepted == 1).await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_stop_during_pending_request_resets_status_and_drop_aborts_workers() {
    let (peer, mut transport) = peer_and_transport().await;
    let _rx = transport.start().await.unwrap();
    registration(&peer).await;
    let management = transport.client_management.clone();
    transport.stop().await.unwrap();
    assert_eq!(
        management.snapshot().foreign_registration.last_outcome,
        None
    );
    assert_eq!(management.snapshot().register_foreign_device.timeouts, 0);
    let _rx = transport.start().await.unwrap();
    registration(&peer).await;
    let worker = transport.registration_task.as_ref().unwrap().abort_handle();
    drop(transport);
    assert_eq!(
        management.snapshot().foreign_registration.next_attempt_in,
        None
    );
    assert_eq!(
        management.snapshot().foreign_registration.last_outcome,
        None
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while !worker.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
