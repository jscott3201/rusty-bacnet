use super::*;
use bacnet_transport::bip6::Bip6BroadcastScope;

async fn control(socket: &UdpSocket, function: u8) -> wire::Frame {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let frame = wire::receive(socket).await.unwrap();
            if frame.bytes.len() >= 7 && frame.bytes[..2] == [0x82, function] {
                return frame;
            }
        }
    })
    .await
    .expect("expected control frame was not observed")
}

async fn collide(observer: &UdpSocket, peer: &UdpSocket, expected: Option<[u8; 3]>) -> wire::Frame {
    let probe = tokio::time::timeout(DEADLINE, async {
        loop {
            let frame = control(observer, 3).await;
            if expected.is_none_or(|vmac| frame.bytes[4..7] == vmac) {
                return frame;
            }
        }
    })
    .await
    .unwrap();
    eprintln!("collision initial probe={probe:?}");
    assert_eq!(probe.bytes.len(), 10);
    assert_eq!(&probe.bytes[4..7], &probe.bytes[7..10]);
    let mut ack = probe.bytes.clone();
    ack[1] = 5;
    peer.send_to(&ack, probe.source).await.unwrap();
    probe
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn explicit_random_collision_reseeds_and_configured_collision_is_transactional() {
    let (selected, index) = fixture();
    let (observer, port) = observer(index, true);
    let peer = udp(selected, 0, index, false);
    let mut random = Bip6Transport::new(selected, port, None);
    let exchange = async {
        let first = collide(&observer, &peer, None).await;
        let second = tokio::time::timeout(DEADLINE, async {
            loop {
                let frame = control(&observer, 3).await;
                if frame.bytes[4..7] != first.bytes[4..7] {
                    return frame;
                }
                eprintln!("duplicate pre-reseed wire probe={frame:?}");
            }
        })
        .await
        .expect("collision ACK did not produce a new random VMAC probe");
        eprintln!("collision next probe={second:?}");
        assert_ne!(&first.bytes[4..7], &second.bytes[4..7]);
        for frame in [first, second] {
            assert_eq!((*frame.source.ip(), frame.source.port()), (selected, port));
            assert_eq!(frame.destination, GROUP);
            assert_eq!(frame.index, index);
        }
    };
    let (started, ()) = tokio::join!(random.start(), exchange);
    let mut incoming = started.unwrap();
    random.stop().await.unwrap();
    assert!(incoming.recv().await.is_none());
    assert_eq!(random.local_mac(), &[0; 3]);

    let mut configured = Bip6Transport::new(selected, port, Some(0x12_3456));
    let (failed, _) = tokio::join!(
        configured.start(),
        collide(&observer, &peer, Some([0x12, 0x34, 0x56]))
    );
    let error = failed.unwrap_err();
    assert!(
        matches!(error, bacnet_types::error::Error::Transport(e) if e.kind() == std::io::ErrorKind::AddrInUse)
    );
    assert_eq!(configured.local_mac(), &[0; 3]);
    assert!(configured.send_broadcast(NPDU).await.is_err());
    let incoming = configured.start().await.unwrap();
    assert_eq!(configured.local_mac(), [0x12, 0x34, 0x56]);
    assert_eq!(
        configured
            .local_address()
            .map(|local| (*local.ip(), local.port())),
        Some((selected, port))
    );
    configured.stop().await.unwrap();
    drop(incoming);
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn explicit_cancel_start_then_restart_stop_and_drop_reclaim_owner() {
    let (selected, index) = fixture();
    let (observer, port) = observer(index, true);
    let mut transport = Bip6Transport::new(selected, port, None);
    {
        let startup = transport.start();
        tokio::pin!(startup);
        tokio::select! {
            probe = control(&observer, 3) => {
                assert_eq!((*probe.source.ip(), probe.source.port()), (selected, port));
            }
            result = &mut startup => panic!("startup completed before its observed collision probe: {result:?}"),
        }
        // Drop the startup future after the actual probe, before publication.
    }
    assert_eq!(transport.local_mac(), &[0; 3]);
    assert!(transport.send_broadcast(NPDU).await.is_err());
    let mut incoming = transport.start().await.unwrap();
    transport.stop().await.unwrap();
    assert!(incoming.recv().await.is_none());
    let mut restarted = transport.start().await.unwrap();
    drop(transport);
    assert!(tokio::time::timeout(DEADLINE, restarted.recv())
        .await
        .unwrap()
        .is_none());
    drop(observer);
    // An exclusive bind after receiver teardown proves no owner kept the port.
    let exclusive =
        std::net::UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0)).unwrap();
    drop(exclusive);
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn explicit_all_broadcast_scopes_have_selected_source_destination_and_ingress() {
    let (selected, index) = fixture();
    let (observer, port) = observer(index, false);
    let mut transport = Bip6Transport::new(selected, port, None);
    let mut incoming = transport.start().await.unwrap();
    let peer = udp(selected, 0, index, false);
    for (scope, nibble) in [
        (Bip6BroadcastScope::LinkLocal, 2),
        (Bip6BroadcastScope::SiteLocal, 5),
        (Bip6BroadcastScope::OrganizationLocal, 8),
    ] {
        let group = Ipv6Addr::new(0xff00 | u16::from(nibble), 0, 0, 0, 0, 0, 0, 0xbac0);
        observer.join_multicast_v6(&group, index).unwrap();
        transport.set_broadcast_scope(scope);
        let npdu = [1, 0, 0x10, 8, 0x09, nibble, 0x19, nibble];
        transport.send_broadcast(&npdu).await.unwrap();
        let frame = wire_broadcast(&observer, &npdu).await.unwrap();
        assert_eq!((*frame.source.ip(), frame.source.port()), (selected, port));
        assert_eq!((frame.destination, frame.index), (group, index));
        let incoming_npdu = [1, 0, 0x10, 8, 0x09, nibble + 10, 0x19, nibble + 10];
        let mut bytes = vec![0x82, 2, 0, 15, 0x40, 0x55, nibble];
        bytes.extend_from_slice(&incoming_npdu);
        peer.send_to(&bytes, SocketAddrV6::new(group, port, 0, index))
            .await
            .unwrap();
        let received = tokio::time::timeout(DEADLINE, incoming.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.npdu.as_ref(), incoming_npdu);
        assert!(received.link_layer_group);
    }
    transport.stop().await.unwrap();
}
