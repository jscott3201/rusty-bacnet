//! A registered B/IPv6 Network Port on the shared endpoint (#1940), read by
//! a B/IPv6 client over loopback: startup publishes the bound port, the VMAC
//! and the mode, NORMAL or FOREIGN, and the port serves the IPV6 rows
//! (Table 12-72); a mismatched identity entry reserves nothing; stop lets
//! the port go.
use super::port_retry::{bind_eventually, rerun_on_lost_port};
use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::constructed::decode_host_n_port;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_transport::bip6::{Bip6ForeignDeviceConfig, Bip6Transport};
use bacnet_types::constructed::BACnetHostNPort;
use bacnet_types::enums::PropertyIdentifier as P;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV6};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

const INSTANCE: u32 = 1234;
/// The VMAC of Device 1234 (Clause H.7.2).
const VMAC: [u8; 3] = [0x00, 0x04, 0xD2];

fn port() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

/// Device 1234 with B/IPv6 port 1 on ::1, any UDP port, network 6.
fn identity() -> crate::DeviceIdentity {
    crate::DeviceIdentity::new(INSTANCE, 555)
        .unwrap()
        .with_bip6_port(1, 6, Ipv6Addr::LOCALHOST, 0)
        .unwrap()
}

/// A server-only endpoint on `transport` with port 1 registered; its
/// database comes from [`identity`], its identity from `id`.
fn endpoint(transport: Bip6Transport, id: crate::DeviceIdentity) -> EndpointSession<Bip6Transport> {
    EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
        .unwrap()
        .with_database(identity().build_database().unwrap())
        .device(id)
        .with_registered_network_port(port())
}

fn transport() -> Bip6Transport {
    Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(INSTANCE))
}

/// The port's property as the database serves it.
async fn local(endpoint: &EndpointSession<Bip6Transport>, property: P) -> PropertyValue {
    let db = endpoint.database.as_ref().unwrap().read().await;
    db.get(&port())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// The UDP port the started endpoint is bound to.
async fn bound(endpoint: &EndpointSession<Bip6Transport>) -> u16 {
    let PropertyValue::Unsigned(bound) = local(endpoint, P::BACNET_IPV6_UDP_PORT).await else {
        panic!("BACnet_IPv6_UDP_Port is Unsigned");
    };
    u16::try_from(bound).unwrap()
}

/// A B/IPv6 client that has resolved the endpoint's VMAC.
async fn client(bound: u16) -> BACnetClient<Bip6Transport> {
    let client = BACnetClient::generic_builder()
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10)))
        .build()
        .await
        .unwrap();
    let address = SocketAddrV6::new(Ipv6Addr::LOCALHOST, bound, 0, 0);
    let vmac = timeout(
        Duration::from_secs(5),
        client.transport().resolve_virtual_address(address),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(vmac.octets(), VMAC);
    client
}

/// One property of the port, read over the wire.
async fn read(client: &BACnetClient<Bip6Transport>, property: P) -> PropertyValue {
    let ack = timeout(
        Duration::from_secs(5),
        client.read_property(&VMAC, port(), property, None),
    )
    .await
    .unwrap()
    .unwrap_or_else(|error| panic!("{property}: {error:?}"));
    decode_application_value(&ack.property_value, 0).unwrap().0
}

/// The port's Property_List, read over the wire.
async fn property_list(client: &BACnetClient<Bip6Transport>) -> Vec<P> {
    let ack = timeout(
        Duration::from_secs(5),
        client.read_property(&VMAC, port(), P::PROPERTY_LIST, None),
    )
    .await
    .unwrap()
    .unwrap();
    let bytes = ack.property_value;
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

/// A NORMAL and a FOREIGN endpoint each publish their bind, VMAC and mode,
/// and serve the IPV6 rows over ReadProperty and Property_List.
#[tokio::test]
async fn a_normal_or_foreign_b_ipv6_endpoint_serves_its_network_port() {
    for foreign in [false, true] {
        // A BBMD that only listens: the registration goes to it.
        let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
        let bbmd_port = bbmd.local_addr().unwrap().port();
        let mut transport = transport();
        if foreign {
            transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
                bbmd_ip: Ipv6Addr::LOCALHOST,
                bbmd_port,
                ttl: 300,
            });
        }
        let mut endpoint = endpoint(transport, identity());
        endpoint.start().await.unwrap();
        let bound = bound(&endpoint).await;
        assert_ne!(bound, 0);
        // The identity entry follows the bind.
        let entry = &endpoint.identity.as_ref().unwrap().network_ports()[0];
        assert_eq!((entry.udp_port, entry.mac.as_slice()), (bound, &VMAC[..]));

        let mut client = client(bound).await;
        let mode = u32::from(foreign);
        for (property, expected) in [
            (P::NETWORK_TYPE, PropertyValue::Enumerated(9)),
            (P::BACNET_IPV6_MODE, PropertyValue::Enumerated(mode)),
            (
                P::IPV6_ADDRESS,
                PropertyValue::OctetString(Ipv6Addr::LOCALHOST.octets().to_vec()),
            ),
            (
                P::BACNET_IPV6_UDP_PORT,
                PropertyValue::Unsigned(bound.into()),
            ),
            (P::MAC_ADDRESS, PropertyValue::OctetString(VMAC.to_vec())),
            (P::NETWORK_NUMBER, PropertyValue::Unsigned(6)),
        ] {
            assert_eq!(read(&client, property).await, expected, "{property}");
        }
        // The transport's default site-local group, at its bound port.
        let mut group = Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0)
            .octets()
            .to_vec();
        group.extend_from_slice(&bound.to_be_bytes());
        assert_eq!(
            read(&client, P::BACNET_IPV6_MULTICAST_ADDRESS).await,
            PropertyValue::OctetString(group)
        );
        let listed = property_list(&client).await;
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
        for property in [P::BACNET_IP_MODE, P::IP_ADDRESS] {
            assert!(!listed.contains(&property), "{property} not listed");
        }
        assert_eq!(listed.contains(&P::FD_BBMD_ADDRESS), foreign);
        assert_eq!(listed.contains(&P::FD_SUBSCRIPTION_LIFETIME), foreign);
        if foreign {
            assert_eq!(
                read(&client, P::FD_SUBSCRIPTION_LIFETIME).await,
                PropertyValue::Unsigned(300)
            );
            // FD_BBMD_Address names the BBMD the transport registers with.
            let ack = timeout(
                Duration::from_secs(5),
                client.read_property(&VMAC, port(), P::FD_BBMD_ADDRESS, None),
            )
            .await
            .unwrap()
            .unwrap();
            let address = ack.property_value;
            let expected = BACnetHostNPort::from_socket_addr(
                SocketAddrV6::new(Ipv6Addr::LOCALHOST, bbmd_port, 0, 0).into(),
            );
            assert_eq!(
                decode_host_n_port(&address, 0).unwrap(),
                (expected, address.len())
            );
        }
        client.stop().await.unwrap();
        endpoint.stop().await.unwrap();
    }
}

/// An identity entry that differs from the database's port is refused
/// before anything is reserved, and the session stays ready: corrected, it
/// starts. A different placeholder VMAC is no mismatch: publication
/// replaces it with the transport's.
#[tokio::test]
async fn a_mismatched_b_ipv6_identity_entry_reserves_nothing() {
    let id = |entry| {
        crate::DeviceIdentity::new(INSTANCE, 555)
            .unwrap()
            .with_network_port(entry)
            .unwrap()
    };
    let mut vmac = crate::NetworkPortEntry::bip6(1, 6, Ipv6Addr::LOCALHOST, 0);
    vmac.mac = bacnet_types::MacAddr::from_slice(&[1, 2, 3]);
    for (case, mismatched) in [
        (
            "network number",
            id(crate::NetworkPortEntry::bip6(1, 7, Ipv6Addr::LOCALHOST, 0)),
        ),
        (
            "UDP port",
            id(crate::NetworkPortEntry::bip6(
                1,
                6,
                Ipv6Addr::LOCALHOST,
                47808,
            )),
        ),
        (
            "address",
            id(crate::NetworkPortEntry::bip6(
                1,
                6,
                Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 1),
                0,
            )),
        ),
        (
            "IPV4 entry",
            id(crate::NetworkPortEntry::bip(1, 6, Ipv4Addr::LOCALHOST, 0)),
        ),
    ] {
        let mut endpoint = endpoint(transport(), mismatched);
        let error = endpoint.start().await.unwrap_err().to_string();
        assert!(error.contains("differs from database"), "{case}: {error}");
        assert!(!endpoint.is_running(), "{case}");
        assert!(endpoint.registered_port_lease.upgrade().is_none(), "{case}");
        {
            let mut db = endpoint.database.as_ref().unwrap().write().await;
            assert_eq!(db.registered_port_internal(), None, "{case}");
            // Nothing holds the port: it can be reserved, and the
            // reservation ends with its token.
            drop(
                db.reserve_bip6_port_internal(port(), Ipv6Addr::LOCALHOST.octets(), 0)
                    .unwrap_or_else(|error| panic!("{case}: {error}")),
            );
        }
        endpoint = endpoint.device(identity());
        endpoint.start().await.unwrap();
        assert_eq!(
            endpoint
                .database
                .as_ref()
                .unwrap()
                .read()
                .await
                .registered_port_internal(),
            Some(port()),
            "{case}"
        );
        endpoint.stop().await.unwrap();
    }
    let mut endpoint = endpoint(transport(), id(vmac));
    endpoint.start().await.unwrap();
    let entry = &endpoint.identity.as_ref().unwrap().network_ports()[0];
    assert_eq!(entry.mac.as_slice(), &VMAC[..]);
    endpoint.stop().await.unwrap();
}

/// A transport whose VMAC isn't the served Device's (#2057), built for
/// another instance or for none (a random VMAC), is refused before anything
/// is reserved. Without an identity the database's Device is the served one.
#[tokio::test]
async fn a_b_ipv6_vmac_from_another_device_reserves_nothing() {
    let other = || Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(10));
    let random = || Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    let no_identity = |transport| {
        EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
            .unwrap()
            .with_database(identity().build_database().unwrap())
            .with_registered_network_port(port())
    };
    let mismatch = "registered B/IPv6 refused: the transport's VMAC 0x00000A is not Device 1234's \
                    VMAC 0x0004D2; build the transport with the served Device instance";
    let none = "registered B/IPv6 refused: the transport has no Device instance in \
                0..=4194303, so its VMAC would be random; build it with Device instance 1234";
    for (case, mut endpoint, expected) in [
        ("other", endpoint(other(), identity()), mismatch),
        ("random", endpoint(random(), identity()), none),
        ("other, no identity", no_identity(other()), mismatch),
    ] {
        let error = endpoint.start().await.unwrap_err().to_string();
        assert!(error.contains(expected), "{case}: {error}");
        assert!(!endpoint.is_running(), "{case}");
        assert!(endpoint.registered_port_lease.upgrade().is_none(), "{case}");
        let mut db = endpoint.database.as_ref().unwrap().write().await;
        assert_eq!(db.registered_port_internal(), None, "{case}");
        drop(
            db.reserve_bip6_port_internal(port(), Ipv6Addr::LOCALHOST.octets(), 0)
                .unwrap_or_else(|error| panic!("{case}: {error}")),
        );
    }
    // The database's Device 1234 matches a transport built for it.
    let mut endpoint = no_identity(transport());
    endpoint.start().await.unwrap();
    endpoint.stop().await.unwrap();
}

/// Stop releases the registration and the UDP port.
#[tokio::test]
async fn stop_releases_the_b_ipv6_port() {
    rerun_on_lost_port(stop_releases).await;
}

/// One run of the test above; the closing rebind can lose the freed port
/// to another process, which reruns it.
async fn stop_releases() -> std::io::Result<()> {
    let mut endpoint = endpoint(transport(), identity());
    endpoint.start().await.unwrap();
    let bound = bound(&endpoint).await;
    let db = Arc::clone(endpoint.database.as_ref().unwrap());
    {
        let mut db = db.write().await;
        assert_eq!(db.registered_port_internal(), Some(port()));
        // While the endpoint runs it holds the reservation.
        assert!(
            db.reserve_bip6_port_internal(port(), Ipv6Addr::LOCALHOST.octets(), 0)
                .is_err()
        );
    }
    endpoint.stop().await.unwrap();
    {
        let mut db = db.write().await;
        assert_eq!(db.registered_port_internal(), None);
        // The port now describes the bind it published; a next owner
        // reserves that.
        drop(
            db.reserve_bip6_port_internal(port(), Ipv6Addr::LOCALHOST.octets(), bound)
                .unwrap(),
        );
    }
    let released = SocketAddrV6::new(Ipv6Addr::LOCALHOST, bound, 0, 0);
    bind_eventually(Duration::from_secs(5), || {
        std::net::UdpSocket::bind(released)
    })
    .await
    .map(drop)
}

/// A B/IPv6 identity entry declares an IPV6 port with a 3-octet VMAC, and
/// an entry mixing the address families is refused.
#[test]
fn b_ipv6_identity_entries_declare_ipv6_ports() {
    let entry = crate::NetworkPortEntry::bip6(1, 6, Ipv6Addr::LOCALHOST, 47808);
    assert_eq!(entry.network_type, 9);
    assert_eq!((entry.ip, entry.mac.as_slice()), ([0; 4], &[0u8; 3][..]));
    let db = crate::DeviceIdentity::new(INSTANCE, 555)
        .unwrap()
        .with_network_port(entry.clone())
        .unwrap()
        .build_database()
        .unwrap();
    let declared = db.get(&port()).unwrap();
    for (property, expected) in [
        (P::NETWORK_TYPE, PropertyValue::Enumerated(9)),
        (
            P::IPV6_ADDRESS,
            PropertyValue::OctetString(Ipv6Addr::LOCALHOST.octets().to_vec()),
        ),
        (P::BACNET_IPV6_UDP_PORT, PropertyValue::Unsigned(47808)),
        (P::MAC_ADDRESS, PropertyValue::OctetString(vec![0; 3])),
        (P::NETWORK_NUMBER, PropertyValue::Unsigned(6)),
    ] {
        assert_eq!(
            declared.read_property(property, None).unwrap(),
            expected,
            "{property}"
        );
    }
    let refused = |entry| {
        crate::DeviceIdentity::new(INSTANCE, 555)
            .unwrap()
            .with_network_port(entry)
            .is_err()
    };
    let mut short = entry.clone();
    short.mac = bacnet_types::MacAddr::from_slice(&[1, 2]);
    assert!(refused(short));
    let mut with_ipv4 = entry;
    with_ipv4.ip = [127, 0, 0, 1];
    assert!(refused(with_ipv4));
    let mut with_ipv6 = crate::NetworkPortEntry::bip(1, 6, Ipv4Addr::LOCALHOST, 0);
    with_ipv6.ipv6 = Ipv6Addr::LOCALHOST.octets();
    assert!(refused(with_ipv6));
}
