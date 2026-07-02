use super::*;

#[tokio::test]
async fn management_acl_allows_listed_sender_write_bdt_and_delete_fdt() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    bbmd_transport.set_bbmd_management_acl(vec![[127, 0, 0, 1]]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let bdt_entry = BdtEntry {
        ip: [192, 168, 40, 10],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let result = client_transport
        .write_bdt(&bbmd_mac, std::slice::from_ref(&bdt_entry))
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(bdt.iter().any(|entry| entry == &bdt_entry));

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();

    let result = client_transport
        .delete_fdt_entry(&bbmd_mac, fd_ip, fd_port)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    assert!(client_transport
        .read_fdt(&bbmd_mac)
        .await
        .unwrap()
        .is_empty());

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn management_acl_denies_unlisted_sender_and_preserves_bdt_and_fdt() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let initial_bdt = BdtEntry {
        ip: [10, 10, 0, 1],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    bbmd_transport.enable_bbmd(vec![initial_bdt.clone()]);
    bbmd_transport.set_bbmd_management_acl(vec![[10, 0, 0, 1]]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();

    let replacement_bdt = [BdtEntry {
        ip: [192, 168, 40, 20],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    }];
    let result = client_transport
        .write_bdt(&bbmd_mac, &replacement_bdt)
        .await
        .unwrap();
    assert_eq!(
        result,
        BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
    );
    let bdt = client_transport.read_bdt(&bbmd_mac).await.unwrap();
    assert!(
        bdt.iter().any(|entry| entry == &initial_bdt),
        "denied Write-BDT must preserve the prior BDT"
    );
    assert!(
        !bdt.iter().any(|entry| entry == &replacement_bdt[0]),
        "denied Write-BDT must not apply the replacement BDT"
    );

    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 60)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();

    let result = client_transport
        .delete_fdt_entry(&bbmd_mac, fd_ip, fd_port)
        .await
        .unwrap();
    assert_eq!(
        result,
        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
    );
    let fdt = client_transport.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].ip, fd_ip);
    assert_eq!(fdt[0].port, fd_port);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}
