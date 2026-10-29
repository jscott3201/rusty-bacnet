//! A foreign device's BBMD and time-to-live written through the registered
//! Network Port and activated by ReinitializeDevice, on a real
//! `BipTransport` (#1825, Clause 12.56.13, 12.56.37, 12.56.38): the writes
//! wait with Changes_Pending set while the device keeps registering where
//! it did, and activation moves its next registration.

use super::network_port_activation_tests::{FALSE, TRUE, reinitialize, wp};
use super::network_port_mode_tests::{host, port_oid, rp};
use super::*;
use bacnet_encoding::constructed::encode_host_n_port;
use bacnet_transport::bip::ForeignDeviceConfig;
use bacnet_types::constructed::BACnetHostNPort;
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};
use tokio::net::UdpSocket;

/// Real loopback: generous, since nothing here waits on a timer (a 60 s
/// time-to-live renews at 30 s).
const WAIT: Duration = Duration::from_secs(10);

async fn bbmd() -> UdpSocket {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}

fn port_of(peer: &UdpSocket) -> u16 {
    peer.local_addr().unwrap().port()
}

/// A foreign-device server on loopback registered with `bbmd` for 60 s,
/// its port registered, with a ReinitializeDevice handler that accepts
/// everything, or none at all.
async fn foreign_server(bbmd: &UdpSocket, handler: bool) -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: 0,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "foreign", config).unwrap(),
    ))
    .unwrap();
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: port_of(bbmd),
        ttl: 60,
        renewal_interval: None,
    });
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        on_reinitialize: handler
            .then(|| Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

/// The Register-Foreign-Device `peer` receives next: its time-to-live, and
/// where it came from.
async fn registration(peer: &UdpSocket) -> (u16, std::net::SocketAddr) {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(WAIT, peer.recv_from(&mut packet))
        .await
        .expect("a registration")
        .unwrap();
    assert_eq!(&packet[..4], &[0x81, 5, 0, 6], "{:?}", &packet[..len]);
    (u16::from_be_bytes([packet[4], packet[5]]), source)
}

fn address_octets(address: &BACnetHostNPort) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_host_n_port(&mut octets, address).unwrap();
    octets.to_vec()
}

/// An application-tagged Unsigned.
fn unsigned(value: u16) -> Vec<u8> {
    match value.to_be_bytes() {
        [0, low] => vec![0x21, low],
        [high, low] => vec![0x22, high, low],
    }
}

#[tokio::test]
async fn written_bbmd_and_lifetime_move_the_next_registration_on_activate_changes() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut server = foreign_server(&first, true).await;
    let (ttl, source) = registration(&first).await;
    assert_eq!(ttl, 60);
    first.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();

    let old = host([127, 0, 0, 1], port_of(&first));
    let new = host([127, 0, 0, 1], port_of(&second));
    wp(&server, P::FD_BBMD_ADDRESS, address_octets(&new))
        .await
        .unwrap();
    wp(&server, P::FD_SUBSCRIPTION_LIFETIME, unsigned(120))
        .await
        .unwrap();
    // Pending: the properties read as written, the device registers as before.
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    assert_eq!(
        rp(&server, P::FD_BBMD_ADDRESS).await.unwrap(),
        address_octets(&new)
    );
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        unsigned(120)
    );
    let config = server.test_network().transport().foreign_device_config();
    assert_eq!(
        config.map(|c| (c.bbmd_port, c.ttl)),
        Some((port_of(&first), 60))
    );

    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    let (ttl, _) = registration(&second).await;
    assert_eq!(ttl, 120);
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(
        rp(&server, P::FD_BBMD_ADDRESS).await.unwrap(),
        address_octets(&new)
    );
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        unsigned(120)
    );
    assert_ne!(old, new);
    server.stop().await.unwrap();
}

/// A BBMD the device couldn't register with, here its own address, is
/// refused by the write and changes neither the property, Changes_Pending,
/// nor the registration, which an activation then leaves as it was.
#[tokio::test]
async fn a_refused_value_leaves_the_property_and_the_registration_untouched() {
    let first = bbmd().await;
    let mut server = foreign_server(&first, true).await;
    registration(&first).await;
    let own = server
        .test_network()
        .transport()
        .bip_port()
        .unwrap()
        .endpoint;
    let before = rp(&server, P::FD_BBMD_ADDRESS).await.unwrap();
    for (property, octets) in [
        (
            P::FD_BBMD_ADDRESS,
            address_octets(&host([127, 0, 0, 1], own.port())),
        ),
        (P::FD_SUBSCRIPTION_LIFETIME, unsigned(0)),
    ] {
        assert_eq!(
            wp(&server, property, octets).await,
            Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
            "{property}"
        );
    }
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(rp(&server, P::FD_BBMD_ADDRESS).await.unwrap(), before);
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        unsigned(60)
    );
    assert!(matches!(
        reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await,
        Apdu::SimpleAck(_)
    ));
    let status = server
        .test_network()
        .transport()
        .bvlc_client_snapshot()
        .foreign_registration;
    assert_eq!(status.attempts, 1);
    assert_eq!(status.last_ttl, Some(60));
    assert_eq!(
        status.last_bbmd.map(|bbmd| bbmd.port()),
        Some(port_of(&first))
    );
    server.stop().await.unwrap();
}

/// With no ReinitializeDevice handler nothing could activate a written
/// value, so the rows refuse writes.
#[tokio::test]
async fn without_a_reinitialize_handler_the_rows_refuse_writes() {
    let first = bbmd().await;
    let mut server = foreign_server(&first, false).await;
    for (property, octets) in [
        (
            P::FD_BBMD_ADDRESS,
            address_octets(&host([127, 0, 0, 1], 47809)),
        ),
        (P::FD_SUBSCRIPTION_LIFETIME, unsigned(120)),
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
