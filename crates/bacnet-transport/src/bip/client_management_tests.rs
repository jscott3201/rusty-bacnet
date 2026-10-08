use super::*;

async fn peer() -> (UdpSocket, [u8; 6]) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let target = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    (peer, target)
}

#[tokio::test]
async fn bvlc_cancelled_request_releases_slot_for_the_next_request() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let mut packet = [0; 64];
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        tokio::select! {
            result = &mut request => panic!("request finished before a response: {result:?}"),
            received = peer.recv_from(&mut packet) => { received.unwrap(); }
        }
        // Dropping a sent request must release the one outstanding slot.
    }
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let (_, source) = tokio::select! {
            result = &mut request => panic!("cancelled request stranded the slot: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 0x03, 0x00, 0x04], source)
            .await
            .unwrap();
        assert!(request.await.unwrap().is_empty());
    }
    let counters = transport.bvlc_client_snapshot().read_bdt;
    assert_eq!(
        (counters.sent, counters.acknowledgements, counters.timeouts),
        (2, 1, 0)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_ttl_one_renews_before_the_advertised_ttl() {
    let (peer, target) = peer().await;
    let (_, port) = decode_bip_mac(&target).unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: port,
        renewal_interval: None,
        ttl: 1,
    });
    let _rx = transport.start().await.unwrap();
    let mut packet = [0; 64];
    let (len, source) = peer.recv_from(&mut packet).await.unwrap();
    assert_eq!(&packet[..len], &[0x81, 0x05, 0, 6, 0, 1]);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    let (len, _) = tokio::time::timeout(Duration::from_millis(900), peer.recv_from(&mut packet))
        .await
        .expect("TTL1 must schedule a renewal before one second")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 0x05, 0, 6, 0, 1]);
    transport.stop().await.unwrap();
}

fn message(function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    BvllMessage {
        function,
        payload: bytes::Bytes::copy_from_slice(payload),
        originating_ip: None,
        originating_port: None,
    }
}

#[tokio::test]
async fn bvlc_early_response_does_not_count_an_unsent_request() {
    let management = Arc::new(client_management::ManagementClient::default());
    management.start();
    let target = ([127, 0, 0, 1], 47808);
    let (guard, _rx) = management
        .begin(
            target,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            BvlcResponseKind::Result,
            Some(60),
        )
        .unwrap();
    // The receive worker can run after admission, before the sender returns
    // from its UDP send. A local send failure/cancellation must not leave a
    // falsely counted receipt from that interval.
    management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0]), target);
    let snapshot = management.snapshot();
    assert_eq!(snapshot.register_foreign_device.sent, 0);
    assert_eq!(
        snapshot.foreign_registration.accepted, 0,
        "an early response must wait for confirmed local send completion"
    );
    drop(guard);
    assert_eq!(management.snapshot().register_foreign_device.results, 0);
}

#[tokio::test]
async fn bvlc_snapshot_counts_each_public_management_helper() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    for (request_function, response) in [
        (1, vec![0x81, 0, 0, 6, 0, 0]),
        (2, vec![0x81, 3, 0, 4]),
        (5, vec![0x81, 0, 0, 6, 0, 0]),
        (6, vec![0x81, 7, 0, 4]),
        (8, vec![0x81, 0, 0, 6, 0, 0]),
    ] {
        let request = async {
            match request_function {
                1 => transport.write_bdt(&target, &[]).await.map(|_| ()),
                2 => transport.read_bdt(&target).await.map(|_| ()),
                5 => transport
                    .register_foreign_device_bvlc(&target, 55)
                    .await
                    .map(|_| ()),
                6 => transport.read_fdt(&target).await.map(|_| ()),
                8 => transport
                    .delete_fdt_entry(&target, [10, 0, 0, 2], 47808)
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            }
        };
        tokio::pin!(request);
        let mut packet = [0; 128];
        let (len, source) = tokio::select! {
            result = &mut request => panic!("request finished before peer response: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..2], &[0x81, request_function]);
        if request_function == 5 {
            assert_eq!(&packet[4..len], &[0, 55]);
        }
        peer.send_to(&response, source).await.unwrap();
        request.await.unwrap();
    }
    let snapshot = transport.bvlc_client_snapshot();
    for counters in [snapshot.read_bdt, snapshot.read_fdt] {
        assert_eq!(
            (counters.sent, counters.acknowledgements, counters.results),
            (1, 1, 0)
        );
    }
    for counters in [
        snapshot.write_bdt,
        snapshot.delete_fdt_entry,
        snapshot.register_foreign_device,
    ] {
        assert_eq!(
            (counters.sent, counters.acknowledgements, counters.results),
            (1, 0, 1)
        );
        assert_eq!(
            counters.last_result,
            Some(BvlcResultCode::SUCCESSFUL_COMPLETION)
        );
    }
    assert_eq!(snapshot.foreign_registration.attempts, 1);
    assert_eq!(snapshot.foreign_registration.accepted, 1);
    assert_eq!(snapshot.foreign_registration.last_ttl, Some(55));
    assert_eq!(snapshot.foreign_registration.next_attempt_in, None);
    assert_eq!(
        transport.management_counters(),
        ManagementCounters::default()
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_operation_specific_results_and_malformed_frames_do_not_steal_requests() {
    let management = Arc::new(client_management::ManagementClient::default());
    management.start();
    let target = ([127, 0, 0, 1], 47808);
    let (guard, mut rx) = management
        .begin(
            target,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            BvlcResponseKind::Result,
            Some(60),
        )
        .unwrap();
    guard.sent(Duration::from_secs(3));
    for (source, code) in [
        (([127, 0, 0, 2], 47808), 0), // wrong IP
        (([127, 0, 0, 1], 47809), 0), // wrong port
        (target, 0x60),               // DBTN-NAK is unrelated to registration
        (target, 0x20),               // Read-BDT-NAK is also unrelated
        (target, 0x1234),             // unknown codes stay observable but unclassified
    ] {
        assert!(!management.complete(
            &message(BvlcFunction::BVLC_RESULT, &u16::to_be_bytes(code)),
            source
        ));
        assert!(rx.try_recv().is_err());
    }
    for payload in [&[][..], &[0][..], &[0, 0, 0][..]] {
        assert!(!management.complete(&message(BvlcFunction::BVLC_RESULT, payload), target));
        assert!(rx.try_recv().is_err());
    }
    assert!(!management.complete(
        &message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, &[]),
        target
    ));
    let before = management.snapshot();
    assert_eq!(before.register_foreign_device.results, 0);
    assert_eq!(before.foreign_registration.accepted, 0);
    assert_eq!(before.foreign_registration.refused, 0);
    assert_eq!(before.unmatched_responses, 6);
    assert_eq!(before.last_unmatched_result.unwrap().to_raw(), 0x1234);
    assert_eq!(before.register_foreign_device.malformed_responses, 3);
    assert!(management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0x30]), target));
    assert_eq!(
        decode_bvlc_result_code(&rx.await.unwrap()).unwrap(),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(management.snapshot().foreign_registration.refused, 1);

    // Each own NAK is a matched result; another operation's NAK is not.
    for (function, expected, own_nak) in [
        (
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            BvlcResponseKind::Result,
            0x10,
        ),
        (
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            BvlcResponseKind::ReadBroadcastDistributionTableAck,
            0x20,
        ),
        (
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            BvlcResponseKind::ReadForeignDeviceTableAck,
            0x40,
        ),
        (
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            BvlcResponseKind::Result,
            0x50,
        ),
    ] {
        let (guard, rx) = management.begin(target, function, expected, None).unwrap();
        guard.sent(Duration::from_secs(3));
        assert!(!management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0x30]), target));
        if expected != BvlcResponseKind::Result {
            assert!(!management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0]), target));
        }
        assert!(management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, own_nak]), target));
        rx.await.unwrap();
        let mut snapshot = management.snapshot();
        assert_eq!(snapshot.counters(function).results, 1);
        assert_eq!(
            snapshot.counters(function).last_result.unwrap().to_raw(),
            u16::from(own_nak)
        );
    }
}

#[tokio::test]
async fn bvlc_malformed_ack_returns_decode_error_without_counting_an_ack() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (_, source) = tokio::select! {
            result = &mut request => panic!("early response: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 3, 0, 5, 0], source).await.unwrap();
        assert!(matches!(request.await, Err(Error::Decoding { .. })));
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.read_bdt.sent, 1);
    assert_eq!(snapshot.read_bdt.acknowledgements, 0);
    assert_eq!(snapshot.read_bdt.malformed_responses, 1);
    assert_eq!(snapshot.malformed_responses, 1);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_busy_timeout_and_late_result_are_distinct_observations() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.register_foreign_device_bvlc(&target, 40);
        tokio::pin!(request);
        let mut packet = [0; 64];
        tokio::select! {
            result = &mut request => panic!("early response: {result:?}"),
            received = peer.recv_from(&mut packet) => { received.unwrap(); }
        }
        assert!(transport
            .read_fdt(&target)
            .await
            .unwrap_err()
            .to_string()
            .contains("already in flight"));
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(matches!(request.await, Err(Error::Timeout(_))));
        tokio::time::resume();
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!((snapshot.read_fdt.sent, snapshot.read_fdt.busy), (0, 1));
    assert_eq!(
        (
            snapshot.register_foreign_device.sent,
            snapshot.register_foreign_device.timeouts
        ),
        (1, 1)
    );
    assert_eq!(
        snapshot.foreign_registration.last_outcome,
        Some(ForeignRegistrationOutcome::TimedOut)
    );
    // A late success with no pending request is observable, not acceptance.
    assert!(!transport.client_management.complete(
        &message(BvlcFunction::BVLC_RESULT, &[0, 0]),
        decode_bip_mac(&target).unwrap()
    ));
    assert_eq!(
        transport
            .bvlc_client_snapshot()
            .foreign_registration
            .accepted,
        0
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_encode_and_send_errors_release_slot_without_fabricated_timeouts() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let entries = vec![
        BdtEntry {
            ip: [10, 0, 0, 1],
            port: 47808,
            broadcast_mask: [255; 4]
        };
        6554
    ];
    assert!(matches!(
        transport.write_bdt(&target, &entries).await,
        Err(Error::Encoding(_))
    ));
    let invalid_port = encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), 0);
    assert!(matches!(
        transport.read_bdt(&invalid_port).await,
        Err(Error::Transport(_))
    ));
    {
        let request = transport.read_bdt(&target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (_, source) = tokio::select! {
            result = &mut request => panic!("failed request stranded the slot: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        request.await.unwrap();
    }
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(
        (
            snapshot.write_bdt.sent,
            snapshot.write_bdt.local_errors,
            snapshot.write_bdt.timeouts
        ),
        (0, 1, 0)
    );
    assert_eq!(
        (
            snapshot.read_bdt.sent,
            snapshot.read_bdt.local_errors,
            snapshot.read_bdt.timeouts
        ),
        (1, 1, 0)
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_stale_guard_cannot_clear_new_request_after_response_or_restart() {
    let management = Arc::new(client_management::ManagementClient::default());
    management.start();
    let target = ([127, 0, 0, 1], 47808);
    for restart in [false, true] {
        let (old, old_rx) = management
            .begin(
                target,
                BvlcFunction::REGISTER_FOREIGN_DEVICE,
                BvlcResponseKind::Result,
                Some(60),
            )
            .unwrap();
        old.sent(Duration::from_secs(3));
        if restart {
            management.stop();
            assert!(old_rx.await.is_err());
            management.start();
        } else {
            assert!(management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0]), target));
            old_rx.await.unwrap();
        }
        let (new, new_rx) = management
            .begin(
                target,
                BvlcFunction::REGISTER_FOREIGN_DEVICE,
                BvlcResponseKind::Result,
                Some(90),
            )
            .unwrap();
        new.sent(Duration::from_secs(3));
        drop(old);
        assert!(management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0]), target));
        new_rx.await.unwrap();
        assert_eq!(
            management.snapshot().foreign_registration.last_ttl,
            Some(90)
        );
        assert_eq!(
            management.snapshot().foreign_registration.last_outcome,
            Some(ForeignRegistrationOutcome::Accepted)
        );
    }
}

#[tokio::test]
async fn bvlc_early_response_is_delivered_after_successful_local_send() {
    let management = Arc::new(client_management::ManagementClient::default());
    management.start();
    let target = ([127, 0, 0, 1], 47808);
    let (guard, mut rx) = management
        .begin(
            target,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            BvlcResponseKind::Result,
            Some(60),
        )
        .unwrap();
    assert!(management.complete(&message(BvlcFunction::BVLC_RESULT, &[0, 0]), target));
    assert_eq!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    assert_eq!(management.snapshot().register_foreign_device.results, 0);
    guard.sent(Duration::from_secs(3));
    assert_eq!(
        decode_bvlc_result_code(&rx.await.unwrap()).unwrap(),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let snapshot = management.snapshot();
    assert_eq!(
        (
            snapshot.register_foreign_device.sent,
            snapshot.register_foreign_device.results
        ),
        (1, 1)
    );
    assert_eq!(snapshot.foreign_registration.accepted, 1);
}

#[tokio::test]
async fn bvlc_manual_zero_ttl_is_a_one_shot_wire_request() {
    let (peer, target) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    {
        let request = transport.register_foreign_device_bvlc(&target, 0);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (len, source) = tokio::select! {
            result = &mut request => panic!("zero TTL was blocked locally: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 0]);
        peer.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
        assert_eq!(
            request.await.unwrap(),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    assert_eq!(
        transport
            .bvlc_client_snapshot()
            .foreign_registration
            .last_ttl,
        Some(0)
    );
    assert_eq!(
        transport
            .bvlc_client_snapshot()
            .foreign_registration
            .next_attempt_in,
        None
    );
    transport.stop().await.unwrap();
}
