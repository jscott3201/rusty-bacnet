//! A BBMD's BDT written through the registered Network Port and activated
//! by ReinitializeDevice, on a real `BipTransport` (#1578, Clause 12.56.13,
//! 12.56.34): the write waits with Changes_Pending set while Read-BDT still
//! answers with the BDT in use, and activation puts it into use.

use super::network_port_mode_tests::{confirmed, host, port_oid, rp};
use super::*;
use bacnet_encoding::constructed::{decode_bdt_entry_list, encode_bdt_entry_list};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::bbmd::BdtEntry;
use bacnet_types::constructed::BACnetBDTEntry;
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};

pub(super) const TRUE: [u8; 1] = [0x11];
pub(super) const FALSE: [u8; 1] = [0x10];

/// A BBMD server on loopback, its port registered, that accepts every
/// ReinitializeDevice.
async fn bbmd_server() -> BACnetServer<BipTransport> {
    bbmd_server_with(true).await
}

/// A BBMD server on loopback, its port registered, with a ReinitializeDevice
/// handler that accepts everything or none at all.
async fn bbmd_server_with(handler: bool) -> BACnetServer<BipTransport> {
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
        on_reinitialize: handler
            .then(|| Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

/// WriteProperty of `octets` to the port's `property`: the error class and
/// code, if refused.
pub(super) async fn wp<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    property: P,
    octets: Vec<u8>,
) -> Result<(), (ErrorClass, ErrorCode)> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: port_oid(),
        property_identifier: property,
        property_array_index: None,
        property_value: octets,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    match confirmed(server, ConfirmedServiceChoice::WRITE_PROPERTY, request).await {
        Apdu::SimpleAck(_) => Ok(()),
        Apdu::Error(error) => Err((error.error_class, error.error_code)),
        other => panic!("unexpected WriteProperty reply {other:?}"),
    }
}

pub(super) async fn reinitialize<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    state: ReinitializedState,
) -> Apdu {
    let mut request = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: None,
    }
    .encode(&mut request)
    .unwrap();
    confirmed(server, ConfirmedServiceChoice::REINITIALIZE_DEVICE, request).await
}

fn bdt_octets(table: &[BACnetBDTEntry]) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_bdt_entry_list(&mut octets, table).unwrap();
    octets.to_vec()
}

/// The BDT a Read-BDT from another B/IP node gets back from the server.
async fn read_bdt(server: &BACnetServer<BipTransport>) -> Vec<BdtEntry> {
    let mut client = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _rx = client.start().await.unwrap();
    let target = server.test_network().transport().local_mac().to_vec();
    let bdt = client.read_bdt(&target).await.unwrap();
    client.stop().await.unwrap();
    bdt
}

fn peer_row(ip: [u8; 4]) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: host(ip, 0xBAC0),
        broadcast_mask: Some([0xFF; 4]),
    }
}

#[tokio::test]
async fn a_written_bdt_takes_effect_on_activate_changes() {
    let mut server = bbmd_server().await;
    let own = server
        .test_network()
        .transport()
        .bip_port()
        .unwrap()
        .endpoint;
    let own_row = BACnetBDTEntry {
        bbmd_address: host([127, 0, 0, 1], own.port()),
        broadcast_mask: Some([0xFF; 4]),
    };
    let before = read_bdt(&server).await;
    assert_eq!(before.len(), 1, "only its own row: {before:?}");
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);

    let table = vec![peer_row([10, 0, 0, 2]), own_row.clone()];
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        bdt_octets(&table),
    )
    .await
    .unwrap();
    wp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS, TRUE.to_vec())
        .await
        .unwrap();

    // Pending: the property reads as written, Read-BDT as in use.
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    let read = rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
        .await
        .unwrap();
    assert_eq!(decode_bdt_entry_list(&read).unwrap(), table);
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        TRUE
    );
    assert_eq!(read_bdt(&server).await, before);
    assert!(server.test_network().transport().fdt_counters().is_some());
    let accepting = || {
        server
            .test_network()
            .transport()
            .bbmd_state()
            .unwrap()
            .lock()
            .unwrap()
            .accepts_foreign_device_registrations()
    };
    assert!(!accepting());

    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    let after = read_bdt(&server).await;
    assert_eq!(
        after,
        [
            BdtEntry {
                ip: [10, 0, 0, 2],
                port: 0xBAC0,
                broadcast_mask: [0xFF; 4],
            },
            before[0].clone(),
        ]
    );
    let read = rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
        .await
        .unwrap();
    assert_eq!(decode_bdt_entry_list(&read).unwrap(), table);
    assert!(accepting());
    server.stop().await.unwrap();
}

/// A BDT the BBMD's policy refuses (here a multicast peer) is refused by
/// the write and changes neither the property, Changes_Pending, nor the
/// BDT in use, which an activation then leaves as it was.
#[tokio::test]
async fn a_refused_bdt_leaves_the_property_and_the_bbmd_untouched() {
    let mut server = bbmd_server().await;
    let before_property = rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
        .await
        .unwrap();
    let before = read_bdt(&server).await;
    let refused = wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        bdt_octets(&[peer_row([224, 0, 0, 1])]),
    )
    .await;
    assert_eq!(
        refused,
        Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
    );
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(
        rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap(),
        before_property
    );
    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(read_bdt(&server).await, before);
    server.stop().await.unwrap();
}

/// A WARMSTART activates too; a COLDSTART leaves the changes pending, as
/// the clause names only those two (Clause 12.56.13).
#[tokio::test]
async fn warmstart_activates_and_coldstart_does_not() {
    let mut server = bbmd_server().await;
    wp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS, TRUE.to_vec())
        .await
        .unwrap();
    assert!(matches!(
        reinitialize(&server, ReinitializedState::COLDSTART).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    assert!(matches!(
        reinitialize(&server, ReinitializedState::WARMSTART).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        TRUE
    );
    server.stop().await.unwrap();
}

/// With no ReinitializeDevice handler nothing could activate a written
/// value, so the rows refuse writes (#1578).
#[tokio::test]
async fn without_a_reinitialize_handler_the_rows_refuse_writes() {
    let mut server = bbmd_server_with(false).await;
    for (property, octets) in [
        (P::BBMD_BROADCAST_DISTRIBUTION_TABLE, Vec::new()),
        (P::BBMD_ACCEPT_FD_REGISTRATIONS, TRUE.to_vec()),
    ] {
        assert_eq!(
            wp(&server, property, octets).await,
            Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
            "{property}"
        );
    }
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    server.stop().await.unwrap();
}
