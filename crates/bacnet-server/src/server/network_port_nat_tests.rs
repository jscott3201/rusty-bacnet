//! BACnet_IP_NAT_Traversal and BACnet_IP_Global_Address on a real
//! `BipTransport` BBMD (#940, Clause 12.56.32, 12.56.33): served on the
//! registered IPv4 port, written through WriteProperty with the BDT that
//! lists the global address, and put into effect together by
//! ReinitializeDevice, after which Read-BDT answers with the global row.

use super::network_port_activation_tests::{FALSE, TRUE, reinitialize, wp};
use super::network_port_mode_tests::{host, port_oid, property_list, rp};
use super::*;
use bacnet_encoding::constructed::{decode_host_n_port, encode_bdt_entry_list, encode_host_n_port};
use bacnet_transport::bbmd::BdtEntry;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};

const GLOBAL: ([u8; 4], u16) = ([203, 0, 113, 1], 47809);

async fn bbmd_server() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: 0,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "bbmd", config).unwrap(),
    ))
    .unwrap();
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.enable_bbmd(Vec::new());
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        on_reinitialize: Some(Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

fn host_octets(address: &BACnetHostNPort) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_host_n_port(&mut octets, address).unwrap();
    octets.to_vec()
}

fn two_hop(address: BACnetHostNPort) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: address,
        broadcast_mask: Some([0xFF; 4]),
    }
}

async fn global_read(server: &BACnetServer<BipTransport>) -> BACnetHostNPort {
    let octets = rp(server, P::BACNET_IP_GLOBAL_ADDRESS).await.unwrap();
    decode_host_n_port(&octets, 0).unwrap().0
}

#[tokio::test]
async fn nat_rows_are_served_written_and_activated_with_the_bdt() {
    let mut server = bbmd_server().await;
    let list = property_list(&server).await;
    assert!(list.contains(&P::BACNET_IP_NAT_TRAVERSAL));
    assert!(list.contains(&P::BACNET_IP_GLOBAL_ADDRESS));
    assert_eq!(
        rp(&server, P::BACNET_IP_NAT_TRAVERSAL).await.unwrap(),
        FALSE
    );
    assert_eq!(
        global_read(&server).await,
        BACnetHostNPort {
            host: BACnetHostAddress::None,
            port: 0
        }
    );

    // A loopback global address is refused when written; the rest is
    // checked alone, in any order.
    assert_eq!(
        wp(
            &server,
            P::BACNET_IP_GLOBAL_ADDRESS,
            host_octets(&host([127, 0, 0, 1], 47809))
        )
        .await,
        Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
    );
    wp(&server, P::BACNET_IP_NAT_TRAVERSAL, TRUE.to_vec())
        .await
        .unwrap();
    let global = host(GLOBAL.0, GLOBAL.1);
    wp(&server, P::BACNET_IP_GLOBAL_ADDRESS, host_octets(&global))
        .await
        .unwrap();
    // With a BDT that has a peer but not the global address, the
    // activation refuses the set before the handler runs, and the values
    // stay pending. (An empty BDT would do: a BBMD with no peers.)
    let peer = host([198, 51, 100, 2], 47809);
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        bdt_octets(&[two_hop(peer.clone())]),
    )
    .await
    .unwrap();
    assert_invalid_configuration(
        &reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
    );
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    let table = [two_hop(global.clone()), two_hop(peer.clone())];
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        bdt_octets(&table),
    )
    .await
    .unwrap();
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    assert_eq!(rp(&server, P::BACNET_IP_NAT_TRAVERSAL).await.unwrap(), TRUE);
    assert_eq!(global_read(&server).await, global);
    let state = || {
        let transport = server.test_network().transport();
        let state = transport.bbmd_state().unwrap().lock().unwrap();
        (state.nat_traversal(), state.own_address())
    };
    let (nat, own) = state();
    assert!(!nat, "pending only");
    assert_eq!(*own.ip(), Ipv4Addr::LOCALHOST);

    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    let (nat, own) = state();
    assert!(nat);
    assert_eq!(own, std::net::SocketAddrV4::new(GLOBAL.0.into(), GLOBAL.1));
    let rows = server
        .test_network()
        .transport()
        .bbmd_state()
        .unwrap()
        .lock()
        .unwrap()
        .bdt();
    assert_eq!(
        rows,
        [
            BdtEntry {
                ip: GLOBAL.0,
                port: GLOBAL.1,
                broadcast_mask: [0xFF; 4]
            },
            BdtEntry {
                ip: [198, 51, 100, 2],
                port: 47809,
                broadcast_mask: [0xFF; 4]
            },
        ],
        "no private own row behind NAT"
    );
    assert_eq!(rp(&server, P::BACNET_IP_NAT_TRAVERSAL).await.unwrap(), TRUE);

    // While NAT is on, the global address moves: the new address and the
    // BDT that lists it, written in either order, activate together.
    let moved = host([203, 0, 113, 9], 47809);
    wp(&server, P::BACNET_IP_GLOBAL_ADDRESS, host_octets(&moved))
        .await
        .unwrap();
    assert_invalid_configuration(
        &reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
    );
    let table = [two_hop(moved.clone()), two_hop(peer)];
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        bdt_octets(&table),
    )
    .await
    .unwrap();
    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    let (nat, own) = state();
    assert!(nat);
    assert_eq!(
        own,
        std::net::SocketAddrV4::new([203, 0, 113, 9].into(), 47809)
    );
    assert_eq!(global_read(&server).await, moved);
    server.stop().await.unwrap();
}

fn bdt_octets(table: &[BACnetBDTEntry]) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_bdt_entry_list(&mut octets, table).unwrap();
    octets.to_vec()
}

fn assert_invalid_configuration(reply: &Apdu) {
    let Apdu::Error(error) = reply else {
        panic!("an Error PDU, not {reply:?}");
    };
    assert_eq!(error.error_class, ErrorClass::PROPERTY);
    assert_eq!(error.error_code, ErrorCode::INVALID_CONFIGURATION_DATA);
}
