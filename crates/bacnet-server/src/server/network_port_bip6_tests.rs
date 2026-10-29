//! A registered B/IPv6 Network Port (#1579) in NORMAL, FOREIGN and BBMD
//! modes, read through the server's confirmed-request path on the shared
//! test transport. The integration tests repeat NORMAL and FOREIGN on a
//! real `Bip6Transport`, which has no BBMD mode; BBMD mode here is a view
//! a custom transport could lend, and shows that a B/IPv6 port's BBMD rows
//! take writes and activation through the same path as a B/IP port's.

use super::network_port_activation_tests::{FALSE, TRUE, reinitialize, wp};
use super::network_port_mode_tests::{port_oid, property_list, rp};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::{
    decode_bdt_entry_list, decode_fdt_entry_list, decode_host_n_port, encode_bdt_entry_list,
};
use bacnet_objects::network_port::Bip6PortConfig;
use bacnet_transport::port::{Bip6Port, BipPort};
use bacnet_types::bip_port::{BbmdChanges, BbmdTables, BipPortMode};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostNPort};
use bacnet_types::enums::{PropertyIdentifier as P, ReinitializedState};
use bacnet_types::error::RemoteError;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Mutex as StdMutex;

const PORT_IP: Ipv6Addr = Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 5);
const UDP: u16 = 47808;
const VMAC: [u8; 3] = [0x12, 0x34, 0x56];
const GROUP: Ipv6Addr = Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0);
const IPV6_ROWS: [P; 7] = [
    P::BACNET_IPV6_MODE,
    P::IPV6_ADDRESS,
    P::IPV6_PREFIX_LENGTH,
    P::BACNET_IPV6_UDP_PORT,
    P::IPV6_DEFAULT_GATEWAY,
    P::BACNET_IPV6_MULTICAST_ADDRESS,
    P::IPV6_DNS_SERVER,
];
const IPV4_ROWS: [P; 6] = [
    P::BACNET_IP_MODE,
    P::IP_ADDRESS,
    P::IP_SUBNET_MASK,
    P::IP_DEFAULT_GATEWAY,
    P::BACNET_IP_UDP_PORT,
    P::IP_DNS_SERVER,
];
const BBMD_ROWS: [P; 3] = [
    P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
    P::BBMD_ACCEPT_FD_REGISTRATIONS,
    P::BBMD_FOREIGN_DEVICE_TABLE,
];
const FOREIGN_ROWS: [P; 2] = [P::FD_BBMD_ADDRESS, P::FD_SUBSCRIPTION_LIFETIME];

fn v6(last: u16) -> Ipv6Addr {
    Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, last)
}

fn host(ip: Ipv6Addr, port: u16) -> BACnetHostNPort {
    BACnetHostNPort::from_socket_addr(SocketAddrV6::new(ip, port, 0, 0).into())
}

/// A B/IPv6 BDT row: a host and port, and no mask.
fn row(ip: Ipv6Addr) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: host(ip, UDP),
        broadcast_mask: None,
    }
}

fn config() -> Bip6PortConfig {
    Bip6PortConfig {
        ip_address: PORT_IP.octets(),
        udp_port: UDP,
        default_gateway: v6(1).octets(),
        dns_servers: vec![v6(0x53).octets(), v6(0x54).octets()],
        ..Default::default()
    }
}

/// The served Device, whose instance gives [`VMAC`] (Clause H.7.2).
const DEVICE_INSTANCE: u32 = 0x12_3456;

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip6(1, "Port", config()).unwrap(),
    ))
    .unwrap();
    db
}

fn bind(mode: BipPortMode) -> Bip6Port {
    Bip6Port {
        endpoint: SocketAddrV6::new(PORT_IP, UDP, 0, 0),
        vmac: Some(VMAC),
        multicast_group: GROUP,
        mode,
    }
}

async fn start_with(
    port: Bip6Port,
    db: ObjectDatabase,
    handler: bool,
) -> Result<BACnetServer<TestTransport>, Error> {
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0).unwrap(),
        registered_network_port: Some(port_oid()),
        on_reinitialize: handler
            .then(|| Arc::new(|_: &_, _: &mut _| Ok(())) as ReinitializeHandler),
        ..Default::default()
    };
    let transport = TestTransport::builder().bip6_port(port).build();
    BACnetServer::start(config, db, transport).await
}

async fn start(mode: BipPortMode, handler: bool) -> BACnetServer<TestTransport> {
    start_with(bind(mode), database(), handler).await.unwrap()
}

/// An OCTET STRING of more than four octets, as an application value.
fn octets(value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x65, value.len() as u8];
    bytes.extend_from_slice(value);
    bytes
}

fn unknown_property() -> Result<Vec<u8>, (ErrorClass, ErrorCode)> {
    Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
}

#[tokio::test]
async fn normal_mode_serves_the_ipv6_rows_and_no_ipv4_ones() {
    let server = start(BipPortMode::Normal, false).await;
    // Network_Type IPV6 (9), BACnet_IPv6_Mode NORMAL (0).
    assert_eq!(rp(&server, P::NETWORK_TYPE).await.unwrap(), [0x91, 9]);
    assert_eq!(rp(&server, P::BACNET_IPV6_MODE).await.unwrap(), [0x91, 0]);
    // The published VMAC replaces the configured (zero) one.
    assert_eq!(
        rp(&server, P::IPV6_ADDRESS).await.unwrap(),
        octets(&PORT_IP.octets())
    );
    assert_eq!(
        rp(&server, P::BACNET_IPV6_UDP_PORT).await.unwrap(),
        [0x22, 0xBA, 0xC0]
    );
    assert_eq!(
        rp(&server, P::MAC_ADDRESS).await.unwrap(),
        [0x63, 0x12, 0x34, 0x56]
    );
    assert_eq!(
        rp(&server, P::IPV6_PREFIX_LENGTH).await.unwrap(),
        [0x21, 64]
    );
    assert_eq!(
        rp(&server, P::IPV6_DEFAULT_GATEWAY).await.unwrap(),
        octets(&v6(1).octets())
    );
    // The group, then the UDP port the transport sends broadcasts to.
    let mut multicast = GROUP.octets().to_vec();
    multicast.extend_from_slice(&UDP.to_be_bytes());
    assert_eq!(
        rp(&server, P::BACNET_IPV6_MULTICAST_ADDRESS).await.unwrap(),
        octets(&multicast)
    );
    let dns = [octets(&v6(0x53).octets()), octets(&v6(0x54).octets())].concat();
    assert_eq!(rp(&server, P::IPV6_DNS_SERVER).await.unwrap(), dns);

    let listed = property_list(&server).await;
    for property in IPV6_ROWS {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in IPV4_ROWS.into_iter().chain(BBMD_ROWS).chain(FOREIGN_ROWS) {
        assert!(!listed.contains(&property), "{property} not listed");
        assert_eq!(
            rp(&server, property).await,
            unknown_property(),
            "{property}"
        );
    }
}

#[tokio::test]
async fn foreign_mode_serves_an_ipv6_bbmd_and_its_lifetime_read_only() {
    let bbmd = host(v6(0xBB), 0xBAC1);
    let server = start(BipPortMode::fixed_foreign(bbmd.clone(), 300), true).await;
    // BACnet_IPv6_Mode FOREIGN (1).
    assert_eq!(rp(&server, P::BACNET_IPV6_MODE).await.unwrap(), [0x91, 1]);
    let read = rp(&server, P::FD_BBMD_ADDRESS).await.unwrap();
    assert_eq!(decode_host_n_port(&read, 0).unwrap(), (bbmd, read.len()));
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        [0x22, 0x01, 0x2C]
    );
    let listed = property_list(&server).await;
    for property in IPV6_ROWS.into_iter().chain(FOREIGN_ROWS) {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in BBMD_ROWS.into_iter().chain(IPV4_ROWS) {
        assert!(!listed.contains(&property), "{property} not listed");
        assert_eq!(
            rp(&server, property).await,
            unknown_property(),
            "{property}"
        );
    }
    // A registration that can't move refuses writes, even behind a
    // ReinitializeDevice handler, and nothing waits.
    assert_eq!(
        wp(&server, P::FD_SUBSCRIPTION_LIFETIME, vec![0x21, 60]).await,
        Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
    );
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    // So do the address rows, which are configuration.
    assert_eq!(
        wp(&server, P::IPV6_PREFIX_LENGTH, vec![0x21, 48]).await,
        Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
    );
}

/// A B/IPv6 BBMD's tables, lent by a transport that takes written values.
#[derive(Default)]
struct Tables {
    bdt: StdMutex<Vec<BACnetBDTEntry>>,
    fdt: StdMutex<Vec<BACnetFDTEntry>>,
    activated: StdMutex<Vec<BbmdChanges>>,
}

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        self.bdt.lock().unwrap().clone()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        true
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        self.fdt.lock().unwrap().clone()
    }
    fn check_broadcast_distribution_table(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        // B/IPv6 rows carry no mask.
        match table.iter().all(|entry| entry.broadcast_mask.is_none()) {
            true => Ok(()),
            false => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
            )),
        }
    }
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        if let Some(table) = &changes.broadcast_distribution_table {
            *self.bdt.lock().unwrap() = table.clone();
        }
        self.activated.lock().unwrap().push(changes.clone());
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn bbmd_mode_serves_ipv6_tables_and_activates_a_written_bdt() {
    let tables = Arc::new(Tables::default());
    *tables.bdt.lock().unwrap() = vec![row(PORT_IP)];
    let registrant = BACnetFDTEntry {
        address: SocketAddr::V6(SocketAddrV6::new(v6(9), UDP, 0, 0)),
        time_to_live: 60,
        remaining_time_to_live: 90,
    };
    *tables.fdt.lock().unwrap() = vec![registrant.clone()];
    let server = start(
        BipPortMode::Bbmd {
            tables: Some(Arc::clone(&tables) as Arc<dyn BbmdTables>),
        },
        true,
    )
    .await;
    // BACnet_IPv6_Mode BBMD (2).
    assert_eq!(rp(&server, P::BACNET_IPV6_MODE).await.unwrap(), [0x91, 2]);
    let bdt = |bytes: Vec<u8>| decode_bdt_entry_list(&bytes).unwrap();
    assert_eq!(
        bdt(rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap()),
        [row(PORT_IP)]
    );
    // [0] address and port, 18 octets [1] 60 [2] 90
    let fdt = rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap();
    assert_eq!(&fdt[..2], [0x0D, 18]);
    assert_eq!(decode_fdt_entry_list(&fdt).unwrap(), [registrant]);
    let listed = property_list(&server).await;
    for property in IPV6_ROWS.into_iter().chain(BBMD_ROWS) {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in FOREIGN_ROWS.into_iter().chain(IPV4_ROWS) {
        assert!(!listed.contains(&property), "{property} not listed");
    }

    // A written BDT waits, read back as written, until activation.
    let table = vec![row(v6(2)), row(PORT_IP)];
    let mut written = BytesMut::new();
    encode_bdt_entry_list(&mut written, &table).unwrap();
    wp(
        &server,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        written.to_vec(),
    )
    .await
    .unwrap();
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), TRUE);
    assert_eq!(
        bdt(rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap()),
        table
    );
    assert_eq!(tables.broadcast_distribution_table(), [row(PORT_IP)]);
    assert!(tables.activated.lock().unwrap().is_empty());

    let reply = reinitialize(&server, ReinitializedState::ACTIVATE_CHANGES).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(rp(&server, P::CHANGES_PENDING).await.unwrap(), FALSE);
    assert_eq!(tables.broadcast_distribution_table(), table);
    assert_eq!(tables.activated.lock().unwrap().len(), 1);
    assert_eq!(
        bdt(rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap()),
        table
    );
}

/// The message a start that should fail failed with.
fn refusal(started: Result<BACnetServer<TestTransport>, Error>) -> String {
    match started {
        Err(Error::Encoding(message)) => message,
        Err(other) => panic!("an Encoding refusal: {other:?}"),
        Ok(_) => panic!("the start should fail"),
    }
}

/// Registration refuses a B/IPv6 link whose object or address doesn't fit:
/// an IPV4 object; the unspecified, a multicast, an IPv4-mapped or a
/// link-local address; another address; and a B/IP link for the IPV6
/// object.
#[tokio::test]
async fn registration_refuses_a_b_ipv6_link_that_does_not_match() {
    let mut ipv4 = ObjectDatabase::new();
    ipv4.add(Box::new(
        NetworkPortObject::new_bip(1, "Port", BipPortConfig::default()).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        refusal(start_with(bind(BipPortMode::Normal), ipv4, false).await),
        "registration requires an IPV6 port"
    );
    // These are refused even when the object declares the same address.
    let concrete = "registered B/IPv6 requires a concrete unicast interface";
    let mapped: Ipv6Addr = "::ffff:192.0.2.1".parse().unwrap();
    let link_local: Ipv6Addr = "fe80::5".parse().unwrap();
    for (declared, ip, expected) in [
        (Ipv6Addr::UNSPECIFIED, Ipv6Addr::UNSPECIFIED, concrete),
        (GROUP, GROUP, concrete),
        (
            PORT_IP,
            mapped,
            "registered B/IPv6 refuses an IPv4-mapped address: register IPv4 as B/IP",
        ),
        (
            link_local,
            link_local,
            "registered B/IPv6 refuses a link-local address: no IPv6_Zone_Index is modeled",
        ),
        (
            PORT_IP,
            v6(6),
            "Network Port configuration differs from transport interface/UDP",
        ),
    ] {
        let mut db = ObjectDatabase::new();
        let config = Bip6PortConfig {
            ip_address: declared.octets(),
            ..config()
        };
        db.add(Box::new(
            NetworkPortObject::new_bip6(1, "Port", config).unwrap(),
        ))
        .unwrap();
        let port = Bip6Port {
            endpoint: SocketAddrV6::new(ip, UDP, 0, 0),
            ..bind(BipPortMode::Normal)
        };
        assert_eq!(refusal(start_with(port, db, false).await), expected, "{ip}");
    }
    // An IPv4-mapped address can't be declared either.
    let mapped_config = Bip6PortConfig {
        ip_address: mapped.octets(),
        ..config()
    };
    assert!(matches!(
        NetworkPortObject::new_bip6(1, "Port", mapped_config),
        Err(Error::Remote(RemoteError::Error {
            code: ErrorCode::VALUE_OUT_OF_RANGE,
            ..
        }))
    ));
    // A B/IP link can't register the B/IPv6 object either.
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        ..Default::default()
    };
    let transport = TestTransport::builder()
        .bip_port(BipPort::new(
            SocketAddrV4::new([127, 0, 0, 1].into(), UDP),
            BipPortMode::Normal,
        ))
        .build();
    assert_eq!(
        refusal(BACnetServer::start(config, database(), transport).await),
        "registration requires an IPV4 port"
    );
}

/// Registration refuses a B/IPv6 link whose VMAC isn't the served Device's
/// (#2057): a transport built for another Device instance, or one built
/// with none (its VMAC random). The refusal comes before anything is
/// reserved, and the server doesn't start.
#[tokio::test]
async fn registration_refuses_a_vmac_that_is_not_the_served_devices() {
    let normal = bind(BipPortMode::Normal);
    for (vmac, expected) in [
        (
            // Device 1234's VMAC.
            Some([0x00, 0x04, 0xD2]),
            "registered B/IPv6 refused: the transport's VMAC 0x0004D2 is not Device 1193046's \
             VMAC 0x123456; build the transport with the served Device instance",
        ),
        (
            None,
            "registered B/IPv6 refused: the transport has no Device instance in 0..=4194303, \
             so its VMAC would be random; build it with Device instance 1193046",
        ),
    ] {
        let port = Bip6Port {
            vmac,
            ..normal.clone()
        };
        let mut db = database();
        let mut transport = TestTransport::builder().bip6_port(port.clone()).build();
        let error =
            network_port::prepare(&mut db, &mut transport, Some(port_oid()), DEVICE_INSTANCE);
        assert!(
            matches!(&error, Err(Error::Encoding(message)) if message == expected),
            "{error:?}"
        );
        assert_eq!(db.registered_port_internal(), None, "{expected}");
        // Nothing holds the port: it can be reserved.
        drop(
            db.reserve_bip6_port_internal(port_oid(), PORT_IP.octets(), UDP)
                .unwrap(),
        );
        assert_eq!(refusal(start_with(port, database(), false).await), expected);
    }
    // The served Device's own VMAC registers.
    start(BipPortMode::Normal, false)
        .await
        .stop()
        .await
        .unwrap();
}
