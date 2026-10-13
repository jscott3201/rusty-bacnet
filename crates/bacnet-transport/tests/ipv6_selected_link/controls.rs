use super::*;

/// A separate-port peer needs no shared bind or privileged packet capture.
#[tokio::test]
#[ignore = "requires an isolated IPv6 link or explicitly selected loopback-only fixture"]
async fn explicit_own_port_peer_proves_multicast_intake_unicast_and_control_sources() {
    let (selected, index) = fixture();
    let mut transport = Bip6Transport::new(selected, 0, None);
    let mut incoming = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let (announced, port) = (*local.ip(), local.port());
    assert_eq!(announced, selected);
    let peer = udp(selected, 0, index, false);
    let peer_vmac = [0x40, 0x88, 0x16];
    let mut group = vec![0x82, 2, 0, 11];
    group.extend_from_slice(&peer_vmac);
    group.extend_from_slice(NPDU);
    peer.send_to(&group, SocketAddrV6::new(GROUP, port, 0, index))
        .await
        .unwrap();
    let admitted = tokio::time::timeout(DEADLINE, incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admitted.npdu.as_ref(), NPDU);
    assert!(admitted.link_layer_group);
    transport
        .send_unicast(NPDU, &admitted.source_mac)
        .await
        .unwrap();
    let response = tokio::time::timeout(DEADLINE, wire::receive(&peer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((response.destination, response.index), (selected, index));
    assert_eq!(
        (*response.source.ip(), response.source.port()),
        (selected, port)
    );
    assert_eq!(response.bytes[..4], [0x82, 1, 0, 14]);
    assert_eq!(response.bytes[7..10], peer_vmac);
    assert_eq!(&response.bytes[10..], NPDU);
    let local_vmac = &response.bytes[4..7];
    // AR targets the retained VMAC; VAR targets its concrete UDP address.
    for (function, expected) in [(3, 5), (6, 7)] {
        let mut request = vec![0x82, function, 0, if function == 3 { 10 } else { 7 }];
        request.extend_from_slice(&peer_vmac);
        let destination = if function == 3 {
            request.extend_from_slice(local_vmac);
            SocketAddrV6::new(GROUP, port, 0, index)
        } else {
            SocketAddrV6::new(selected, port, 0, 0)
        };
        peer.send_to(&request, destination).await.unwrap();
        let ack = tokio::time::timeout(DEADLINE, wire::receive(&peer))
            .await
            .unwrap()
            .unwrap();
        let mut expected_bytes = vec![0x82, expected, 0, 10];
        expected_bytes.extend_from_slice(local_vmac);
        expected_bytes.extend_from_slice(&peer_vmac);
        assert_eq!(ack.bytes, expected_bytes);
        assert_eq!((*ack.source.ip(), ack.source.port()), (selected, port));
        assert_eq!((ack.destination, ack.index), (selected, index));
    }
    transport.stop().await.unwrap();
    assert!(incoming.recv().await.is_none());
}
