use super::*;
use tokio::time::{timeout, Duration};

#[test]
fn bip_max_apdu_length() {
    let transport = BipTransport::new(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        std::net::Ipv4Addr::LOCALHOST,
    );
    assert_eq!(transport.max_apdu_length(), 1476);
}

#[tokio::test]
async fn start_stop() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    assert!(!transport.local_mac().iter().all(|b| *b == 0));
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn unicast_loopback() {
    // Two transports on localhost with ephemeral ports
    let mut transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let _rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

    // A sends unicast to B
    transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap();

    // B should receive it
    let received = timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("Timed out waiting for packet")
        .expect("Channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

    transport_a.stop().await.unwrap();
    transport_b.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_register_foreign_device() {
    // Start a BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Start a foreign device that registers with the BBMD
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 60,
    });
    let _fd_rx = fd_transport.start().await.unwrap();

    // Give a moment for the registration to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the BBMD has the foreign device in its FDT
    {
        let bbmd_state = bbmd_transport.bbmd_state().unwrap();
        let mut state = bbmd_state.lock().await;
        let fdt = state.fdt();
        assert_eq!(fdt.len(), 1);
        assert_eq!(fdt[0].ttl, 60);
    }

    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_bdt_from_bbmd() {
    // Start a BBMD with a known BDT
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let initial_bdt = vec![BdtEntry {
        ip: [10, 0, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    }];
    bbmd_transport.enable_bbmd(initial_bdt.clone());
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    // Start a second transport (client) to query the BBMD
    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    // Read the BDT — includes the configured entry plus the auto-inserted self entry
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert_eq!(bdt.len(), 2);
    assert!(bdt
        .iter()
        .any(|e| e.ip == [10, 0, 0, 1] && e.port == 0xBAC0));
    // Self entry is also present (auto-inserted by set_bdt)
    assert!(bdt.len() >= 2);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_fdt_from_bbmd() {
    // Start a BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Register a foreign device
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 120,
    });
    let _fd_rx = fd_transport.start().await.unwrap();
    let fd_mac = fd_transport.local_mac().to_vec();
    let (fd_ip, fd_port) = decode_bip_mac(&fd_mac).unwrap();

    // Wait for registration to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Start a third transport to query the FDT
    let mut query_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _query_rx = query_transport.start().await.unwrap();

    let fdt = query_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].ip, fd_ip);
    assert_eq!(fdt[0].port, fd_port);
    assert_eq!(fdt[0].ttl, 120);
    assert!(fdt[0].seconds_remaining <= 150);

    query_transport.stop().await.unwrap();
    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn write_bdt_to_bbmd() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let new_bdt = vec![BdtEntry {
        ip: [192, 168, 1, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 255],
    }];
    let result = client_transport
        .write_bdt(&bbmd_mac, &new_bdt)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    // Verify by reading back — includes written entry plus auto-inserted self
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(bdt
        .iter()
        .any(|e| e.ip == [192, 168, 1, 1] && e.port == 0xBAC0));

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn register_foreign_device_via_bvlc() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn foreign_device_broadcast_via_bbmd() {
    // BBMD
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Foreign device
    let mut fd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    fd_transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(bbmd_ip),
        bbmd_port,
        ttl: 60,
    });
    let _fd_rx = fd_transport.start().await.unwrap();

    // Give time for registration
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Foreign device sends a broadcast (should use Distribute-Broadcast-To-Network)
    let test_npdu = vec![0x01, 0x00, 0xAA, 0xBB];
    fd_transport.send_broadcast(&test_npdu).await.unwrap();

    // BBMD should receive it (as NPDU via Distribute-Broadcast-To-Network)
    let received = timeout(Duration::from_secs(2), bbmd_rx.recv())
        .await
        .expect("BBMD timed out")
        .expect("BBMD channel closed");

    assert_eq!(received.npdu, test_npdu);

    fd_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn bbmd_management_acl_preserved_after_start() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(vec![]);
    transport.set_bbmd_management_acl(vec![[10, 0, 0, 1]]);
    let _rx = transport.start().await.unwrap();

    {
        let state = transport.bbmd_state().unwrap();
        let s = state.lock().await;
        assert!(s.is_management_allowed(&[10, 0, 0, 1]));
        assert!(!s.is_management_allowed(&[10, 0, 0, 2]));
    }

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bvlc_request_rejects_concurrent_calls() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();

    // Manually install a pending sender to simulate an in-flight request
    {
        let (tx, _rx) = oneshot::channel();
        let mut slot = transport.bvlc_response_tx.lock().await;
        *slot = Some(tx);
    }

    // A second request should fail immediately
    let fake_target = transport.local_mac().to_vec();
    let result = transport.read_bdt(&fake_target).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("already in flight"),
        "expected 'already in flight' error, got: {err}"
    );

    transport.stop().await.unwrap();
}
