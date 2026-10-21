//! BACnet_IP_Multicast_Address on a real `BipTransport` B/IP-M node (#941,
//! Clause 12.56.26): served on the registered IPv4 port, written through
//! WriteProperty, and put into effect by ReinitializeDevice, after which
//! the node's broadcasts go to the new group; 0.0.0.0 ends multicast.

use super::network_port_activation_tests::{reinitialize, wp, FALSE, TRUE};
use super::network_port_mode_tests::{port_oid, property_list, rp};
use super::*;
use bacnet_transport::bip::BipMulticast;
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};

const GROUP: [u8; 4] = [239, 255, 95, 1];
const MOVED: [u8; 4] = [239, 255, 95, 2];

/// The application-tagged OCTET STRING of a group.
fn tagged(group: [u8; 4]) -> Vec<u8> {
    let mut octets = vec![0x64];
    octets.extend_from_slice(&group);
    octets
}

/// A UDP port no socket holds now, for the group.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn member_server(group_port: u16) -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: group_port,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "member", config).unwrap(),
    ))
    .unwrap();
    // At the group's port, as Windows requires.
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, group_port, Ipv4Addr::LOCALHOST);
    let multicast = BipMulticast::new(GROUP.into())
        .unwrap()
        .with_port(group_port)
        .unwrap();
    transport.set_multicast(Some(multicast)).unwrap();
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        on_reinitialize: Some(Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

fn group_in_use(server: &BACnetServer<BipTransport>) -> Option<[u8; 4]> {
    let transport = server.test_network().transport();
    transport
        .multicast()
        .map(|multicast| multicast.group().octets())
}

#[tokio::test]
async fn the_multicast_group_is_served_written_and_activated() {
    let group_port = free_port();
    let mut server = member_server(group_port).await;
    assert!(property_list(&server)
        .await
        .contains(&P::BACNET_IP_MULTICAST_ADDRESS));
    assert_eq!(
        rp(&server, P::BACNET_IP_MULTICAST_ADDRESS).await.unwrap(),
        tagged(GROUP)
    );
    let endpoint = server.test_network().transport().bip_broadcast_endpoint();
    assert_eq!(
        endpoint,
        Some(std::net::SocketAddrV4::new(GROUP.into(), group_port))
    );

    // Not a group: refused when written, and nothing waits.
    assert_eq!(
        wp(
            &server,
            P::BACNET_IP_MULTICAST_ADDRESS,
            tagged([10, 0, 0, 1])
        )
        .await,
        Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
    );
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);

    // A new group waits, read back as written, until activation.
    wp(&server, P::BACNET_IP_MULTICAST_ADDRESS, tagged(MOVED))
        .await
        .unwrap();
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    assert_eq!(
        rp(&server, P::BACNET_IP_MULTICAST_ADDRESS).await.unwrap(),
        tagged(MOVED)
    );
    assert_eq!(group_in_use(&server), Some(GROUP));
    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(group_in_use(&server), Some(MOVED));
    assert_eq!(
        server.test_network().transport().bip_broadcast_endpoint(),
        Some(std::net::SocketAddrV4::new(MOVED.into(), group_port))
    );

    // 0.0.0.0 ends multicast at the next activation.
    wp(&server, P::BACNET_IP_MULTICAST_ADDRESS, tagged([0; 4]))
        .await
        .unwrap();
    assert!(matches!(
        reinitialize(&server, ReinitializedState::WARMSTART).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(group_in_use(&server), None);
    assert_eq!(
        rp(&server, P::BACNET_IP_MULTICAST_ADDRESS).await.unwrap(),
        tagged([0; 4])
    );
    server.stop().await.unwrap();
}

/// A BBMD server that serves [`GROUP`] at its own port, as Windows needs.
async fn group_bbmd_server(port: u16) -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: port,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "group BBMD", config).unwrap(),
    ))
    .unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
    let multicast = BipMulticast::new(GROUP.into())
        .unwrap()
        .with_port(port)
        .unwrap();
    transport.set_multicast(Some(multicast)).unwrap();
    transport.enable_bbmd(Vec::new());
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        on_reinitialize: Some(Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

/// A BBMD that serves a group (Annex J.8.3) lists its group beside the BBMD
/// rows, read-only: its own BDT row and its group go together, so the
/// group stays as it started. A written BDT whose own row isn't two-hop
/// is refused at activation.
#[tokio::test]
async fn a_group_bbmd_serves_its_group_read_only() {
    use bacnet_encoding::constructed::encode_bdt_entry_list;
    use bacnet_types::constructed::BACnetBDTEntry;

    let port = free_port();
    let mut server = group_bbmd_server(port).await;
    let listed = property_list(&server).await;
    assert!(listed.contains(&P::BACNET_IP_MULTICAST_ADDRESS));
    assert!(listed.contains(&P::BBMD_BROADCAST_DISTRIBUTION_TABLE));
    assert_eq!(
        rp(&server, P::BACNET_IP_MULTICAST_ADDRESS).await.unwrap(),
        tagged(GROUP)
    );
    for group in [MOVED, [0; 4]] {
        assert_eq!(
            wp(&server, P::BACNET_IP_MULTICAST_ADDRESS, tagged(group)).await,
            Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
        );
    }
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(group_in_use(&server), Some(GROUP));

    // The PICS draft agrees: the group read-only, the BDT writable.
    let pics = server
        .generate_pics(&crate::pics::PicsConfig::default())
        .await;
    let writable = |property| {
        pics.supported_object_types
            .iter()
            .flat_map(|object| &object.supported_properties)
            .find(|row| row.property_id == property)
            .map(|row| row.access.writable)
    };
    assert_eq!(writable(P::BACNET_IP_MULTICAST_ADDRESS), Some(false));
    assert_eq!(writable(P::BBMD_BROADCAST_DISTRIBUTION_TABLE), Some(true));

    // The BBMD's own row with a subnet mask passes the row policy when
    // written, and is refused at activation, leaving it pending.
    let own_one_hop = BACnetBDTEntry {
        bbmd_address: super::network_port_mode_tests::host([127, 0, 0, 1], port),
        broadcast_mask: Some([255, 0, 0, 0]),
    };
    let mut octets = BytesMut::new();
    encode_bdt_entry_list(&mut octets, &[own_one_hop]).unwrap();
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        octets.to_vec(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::Error(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    let transport = server.test_network().transport();
    let state = transport.bbmd_state().unwrap();
    let own = state.lock().unwrap().bdt();
    assert!(own.iter().all(|row| row.broadcast_mask == [0xFF; 4]));
    server.stop().await.unwrap();
}
