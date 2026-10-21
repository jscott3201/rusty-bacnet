//! A registered B/IPv6 Network Port on a real `Bip6Transport` (#1579), read
//! by a B/IPv6 client over loopback: startup publishes the bound port, the
//! VMAC and the mode, NORMAL or FOREIGN, and the port serves the IPV6 rows
//! (Table 12-72) with that mode's own.
#![cfg(feature = "ipv6")]

use std::net::{Ipv6Addr, SocketAddrV6};

// The rebind of a stopped owner's port can lose it to another process.
#[path = "../../bacnet-endpoint/tests/support/port_retry.rs"]
mod port_retry;
use port_retry::{bind_eventually, rerun_on_lost_port};

use bacnet_client::client::BACnetClient;
use bacnet_encoding::constructed::{decode_host_n_port, encode_host_n_port};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::network_port::{Bip6PortConfig, NetworkPortObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip6::{decode_bvll6, Bip6ForeignDeviceConfig, Bip6Transport, Bvll6};
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetHostNPort;
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier as P, ReinitializedState};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

const SERVER_INSTANCE: u32 = 1234;
/// The VMAC of Device 1234 (Clause H.7.2).
const SERVER_VMAC: [u8; 3] = [0x00, 0x04, 0xD2];

fn port() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

/// Network Port 1 on ::1, any UDP port.
fn loopback_port() -> NetworkPortObject {
    NetworkPortObject::new_bip6(
        1,
        "B/IPv6",
        Bip6PortConfig {
            network_number: 6,
            ip_address: Ipv6Addr::LOCALHOST.octets(),
            udp_port: 0,
            prefix_length: 128,
            ..Default::default()
        },
    )
    .unwrap()
}

/// A server with port 1 registered, and a ReinitializeDevice handler that
/// accepts every request when `handler` is set.
async fn server_with(transport: Bip6Transport, handler: bool) -> BACnetServer<Bip6Transport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(loopback_port())).unwrap();
    let mut builder = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(SERVER_INSTANCE, 0).unwrap())
        .database(db)
        .transport(transport)
        .registered_network_port(port());
    if handler {
        builder = builder.on_reinitialize(|_, _| Ok(()));
    }
    builder.build().await.unwrap()
}

async fn server(transport: Bip6Transport) -> BACnetServer<Bip6Transport> {
    server_with(transport, false).await
}

/// A client that knows the server's VMAC.
async fn client(server: &BACnetServer<Bip6Transport>) -> BACnetClient<Bip6Transport> {
    let client = BACnetClient::generic_builder()
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10)))
        .build()
        .await
        .unwrap();
    let address = server.transport().unwrap().local_address().unwrap();
    let address = SocketAddrV6::new(*address.ip(), address.port(), 0, 0);
    let vmac = timeout(
        Duration::from_secs(5),
        client.transport().resolve_virtual_address(address),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(vmac.octets(), SERVER_VMAC);
    client
}

/// Read one property of the port: its value bytes.
async fn read(client: &BACnetClient<Bip6Transport>, property: P) -> Vec<u8> {
    timeout(
        Duration::from_secs(5),
        client.read_property(&SERVER_VMAC, port(), property, None),
    )
    .await
    .unwrap()
    .unwrap_or_else(|error| panic!("{property}: {error:?}"))
    .property_value
}

/// The value of an application-tagged property.
fn value(bytes: &[u8]) -> PropertyValue {
    decode_application_value(bytes, 0).unwrap().0
}

/// The port's Property_List.
async fn property_list(client: &BACnetClient<Bip6Transport>) -> Vec<P> {
    let bytes = read(client, P::PROPERTY_LIST).await;
    let mut listed = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, end) = decode_application_value(&bytes, offset).unwrap();
        let PropertyValue::Enumerated(raw) = value else {
            panic!("Property_List holds enumerations: {value:?}");
        };
        listed.push(P::from_raw(raw));
        offset = end;
    }
    listed
}

/// The rows every B/IPv6 port serves, whatever its mode, read back as the
/// bound transport is.
async fn assert_ipv6_rows(client: &BACnetClient<Bip6Transport>, bound: u16, mode: u32) {
    assert_eq!(
        value(&read(client, P::NETWORK_TYPE).await),
        PropertyValue::Enumerated(9)
    );
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_MODE).await),
        PropertyValue::Enumerated(mode)
    );
    assert_eq!(
        value(&read(client, P::IPV6_ADDRESS).await),
        PropertyValue::OctetString(Ipv6Addr::LOCALHOST.octets().to_vec())
    );
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_UDP_PORT).await),
        PropertyValue::Unsigned(bound.into())
    );
    assert_eq!(
        value(&read(client, P::MAC_ADDRESS).await),
        PropertyValue::OctetString(SERVER_VMAC.to_vec())
    );
    // The transport's default site-local group, at its bound port.
    let mut group = Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0)
        .octets()
        .to_vec();
    group.extend_from_slice(&bound.to_be_bytes());
    assert_eq!(
        value(&read(client, P::BACNET_IPV6_MULTICAST_ADDRESS).await),
        PropertyValue::OctetString(group)
    );
    assert_eq!(
        value(&read(client, P::NETWORK_NUMBER).await),
        PropertyValue::Unsigned(6)
    );
    let listed = property_list(client).await;
    for property in [
        P::BACNET_IPV6_MODE,
        P::IPV6_ADDRESS,
        P::IPV6_PREFIX_LENGTH,
        P::BACNET_IPV6_UDP_PORT,
        P::IPV6_DEFAULT_GATEWAY,
        P::BACNET_IPV6_MULTICAST_ADDRESS,
        P::IPV6_DNS_SERVER,
    ] {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in [
        P::BACNET_IP_MODE,
        P::IP_ADDRESS,
        P::BBMD_FOREIGN_DEVICE_TABLE,
    ] {
        assert!(!listed.contains(&property), "{property} not listed");
    }
}

#[tokio::test]
async fn a_normal_b_ipv6_server_registers_its_network_port() {
    let mut server = server(Bip6Transport::new(
        Ipv6Addr::LOCALHOST,
        0,
        Some(SERVER_INSTANCE),
    ))
    .await;
    let bound = server.transport().unwrap().local_address().unwrap().port();
    assert_ne!(bound, 0);
    let mut client = client(&server).await;
    assert_ipv6_rows(&client, bound, 0).await;
    let listed = property_list(&client).await;
    assert!(!listed.contains(&P::FD_BBMD_ADDRESS));
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// The time-to-live of the next Register-Foreign-Device `bbmd` receives.
async fn registration(bbmd: &UdpSocket) -> u16 {
    let mut buf = [0u8; 1600];
    loop {
        let (len, _) = timeout(Duration::from_secs(5), bbmd.recv_from(&mut buf))
            .await
            .expect("a registration")
            .unwrap();
        if let Ok(Bvll6::RegisterForeignDevice { ttl, .. }) = decode_bvll6(&buf[..len]) {
            return ttl;
        }
    }
}

async fn write(
    client: &BACnetClient<Bip6Transport>,
    property: P,
    value: Vec<u8>,
) -> Result<(), Error> {
    timeout(
        Duration::from_secs(5),
        client.write_property(&SERVER_VMAC, port(), property, None, value, None),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_foreign_b_ipv6_server_serves_its_bbmd_and_lifetime() {
    // A BBMD that only listens: the server's registration goes to it.
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let bbmd_port = bbmd.local_addr().unwrap().port();
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(SERVER_INSTANCE));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port,
        ttl: 300,
    });
    let mut server = server(transport).await;
    assert_eq!(registration(&bbmd).await, 300);
    let bound = server.transport().unwrap().local_address().unwrap().port();
    let mut client = client(&server).await;
    assert_ipv6_rows(&client, bound, 1).await;
    let address = read(&client, P::FD_BBMD_ADDRESS).await;
    let expected = BACnetHostNPort::from_socket_addr(
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, bbmd_port, 0, 0).into(),
    );
    assert_eq!(
        decode_host_n_port(&address, 0).unwrap(),
        (expected, address.len())
    );
    assert_eq!(
        value(&read(&client, P::FD_SUBSCRIPTION_LIFETIME).await),
        PropertyValue::Unsigned(300)
    );
    let listed = property_list(&client).await;
    assert!(listed.contains(&P::FD_SUBSCRIPTION_LIFETIME));
    // Without a ReinitializeDevice handler nothing could activate a
    // written value, so the rows refuse writes.
    let refused = write(&client, P::FD_SUBSCRIPTION_LIFETIME, vec![0x21, 60]).await;
    assert!(
        matches!(
            refused,
            Err(Error::Protocol {
                code: ErrorCode::WRITE_ACCESS_DENIED,
                ..
            })
        ),
        "{refused:?}"
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// FD_BBMD_Address and FD_Subscription_Lifetime are writable (Clause
/// 12.56.37, 12.56.38): a write waits with Changes_Pending set, and
/// ACTIVATE_CHANGES moves the running registration to the new BBMD.
#[tokio::test]
async fn activate_changes_moves_a_b_ipv6_foreign_registration() {
    let old = UdpSocket::bind("[::1]:0").await.unwrap();
    let new = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(SERVER_INSTANCE));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: old.local_addr().unwrap().port(),
        ttl: 300,
    });
    let mut server = server_with(transport, true).await;
    assert_eq!(registration(&old).await, 300);
    let mut client = client(&server).await;
    let mut address = BytesMut::new();
    encode_host_n_port(
        &mut address,
        &BACnetHostNPort::from_socket_addr(new.local_addr().unwrap()),
    )
    .unwrap();
    write(&client, P::FD_BBMD_ADDRESS, address.to_vec())
        .await
        .unwrap();
    write(&client, P::FD_SUBSCRIPTION_LIFETIME, vec![0x21, 120])
        .await
        .unwrap();
    assert_eq!(
        value(&read(&client, P::CHANGES_PENDING).await),
        PropertyValue::Boolean(true)
    );
    // Pending: the rows read as written, the transport registers as before.
    assert_eq!(
        value(&read(&client, P::FD_SUBSCRIPTION_LIFETIME).await),
        PropertyValue::Unsigned(120)
    );
    assert_eq!(
        server
            .transport()
            .unwrap()
            .foreign_device_config()
            .unwrap()
            .ttl,
        300
    );
    // A host it can't register with is refused at once.
    let mut bad = BytesMut::new();
    encode_host_n_port(
        &mut bad,
        &BACnetHostNPort::from_socket_addr("[ff05::bac0]:47808".parse().unwrap()),
    )
    .unwrap();
    let refused = write(&client, P::FD_BBMD_ADDRESS, bad.to_vec()).await;
    assert!(
        matches!(
            refused,
            Err(Error::Protocol {
                code: ErrorCode::VALUE_OUT_OF_RANGE,
                ..
            })
        ),
        "{refused:?}"
    );

    timeout(
        Duration::from_secs(5),
        client.reinitialize_device(&SERVER_VMAC, ReinitializedState::ACTIVATE_CHANGES, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(registration(&new).await, 120);
    assert_eq!(
        value(&read(&client, P::CHANGES_PENDING).await),
        PropertyValue::Boolean(false)
    );
    let config = server.transport().unwrap().foreign_device_config().unwrap();
    assert_eq!(
        (config.bbmd_port, config.ttl),
        (new.local_addr().unwrap().port(), 120)
    );
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Register port `oid` of `db` for `transport` and start it, as a server
/// does.
async fn register_and_start(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    transport: &mut Bip6Transport,
) -> Result<(), Error> {
    let configured = transport.bip6_port().unwrap().endpoint;
    let (_, lease) =
        db.reserve_bip6_port_internal(oid, configured.ip().octets(), configured.port())?;
    transport.retain_network_port_lease_internal(lease)?;
    let _rx = transport.start().await?;
    let bound = transport.bip6_port().unwrap();
    db.publish_bip6_port_internal(
        oid,
        bound.endpoint.ip().octets(),
        bound.endpoint.port(),
        bound.vmac.unwrap(),
        bound.multicast_group.octets(),
        1476,
        bound.mode,
    )
}

/// The registration lease ends with the transport: a start that fails
/// leaves the port free for the next owner, and so does a stop.
#[tokio::test]
async fn a_failed_start_and_a_stop_leave_the_port_free_for_the_next_owner() {
    rerun_on_lost_port(failed_start_and_stop).await;
}

/// One run of the test above. The rebind of the stopped owner's UDP port can
/// lose it to another process (#1419), which reruns the test.
async fn failed_start_and_stop() -> std::io::Result<()> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(loopback_port())).unwrap();
    // Port 2 names an address this host doesn't have, so its start fails
    // to bind.
    let absent = Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0xDEAD, 0x77);
    let port_2 = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 2).unwrap();
    db.add(Box::new(
        NetworkPortObject::new_bip6(
            2,
            "absent",
            Bip6PortConfig {
                ip_address: absent.octets(),
                udp_port: 0,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let mut unbindable = Bip6Transport::new(absent, 0, Some(SERVER_INSTANCE));
    assert!(register_and_start(&mut db, port_2, &mut unbindable)
        .await
        .is_err());
    drop(unbindable);
    assert_eq!(db.registered_port_internal(), None);
    // A foreign device needs a Device instance, so this start fails after
    // the port was reserved and the lease handed over.
    let mut failing = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    failing.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: 47808,
        ttl: 60,
    });
    assert!(register_and_start(&mut db, port(), &mut failing)
        .await
        .is_err());
    // While the failed transport lives it still holds the reservation.
    assert!(db
        .reserve_bip6_port_internal(port(), Ipv6Addr::LOCALHOST.octets(), 0)
        .is_err());
    drop(failing);
    assert_eq!(db.registered_port_internal(), None);

    let mut first = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(SERVER_INSTANCE));
    register_and_start(&mut db, port(), &mut first)
        .await
        .unwrap();
    assert_eq!(db.registered_port_internal(), Some(port()));
    let read = |db: &ObjectDatabase, property| {
        db.get(&port())
            .unwrap()
            .read_property(property, None)
            .unwrap()
    };
    assert_eq!(
        read(&db, P::MAC_ADDRESS),
        PropertyValue::OctetString(SERVER_VMAC.to_vec())
    );
    let bound = first.local_address().unwrap().port();
    first.stop().await.unwrap();
    assert_eq!(db.registered_port_internal(), None);
    // The stopped owner let its UDP port go.
    let released = SocketAddrV6::new(Ipv6Addr::LOCALHOST, bound, 0, 0);
    bind_eventually(Duration::from_secs(5), || {
        std::net::UdpSocket::bind(released)
    })
    .await
    .map(drop)?;

    // A stopped owner's port registers again, here with another VMAC. The
    // port keeps the bind it last published, as a B/IP port does, so the
    // next owner binds the same UDP port.
    let mut second = Bip6Transport::new(Ipv6Addr::LOCALHOST, bound, Some(4321));
    match register_and_start(&mut db, port(), &mut second).await {
        Ok(()) => {}
        Err(Error::Transport(error)) => return Err(error),
        Err(other) => panic!("the second owner registers: {other:?}"),
    }
    assert_eq!(db.registered_port_internal(), Some(port()));
    assert_eq!(
        read(&db, P::MAC_ADDRESS),
        PropertyValue::OctetString(vec![0x00, 0x10, 0xE1])
    );
    second.stop().await.unwrap();
    Ok(())
}
