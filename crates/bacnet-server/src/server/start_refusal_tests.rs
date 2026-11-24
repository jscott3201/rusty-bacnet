//! A refused start hands its database back, and the same database then
//! starts a server that serves every object (#2752): a link that won't
//! start, a builder's own checks, a BACnet/SC dial, a seat, and a refusal
//! after a registered Network Port was published.
use std::sync::atomic::AtomicBool;

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_endpoint_core::coordinator::{
    NOTIFICATION_RESERVE, OutboundTransactionCoordinator, REQUESTER_RESERVE,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_transport::port::BipPort;
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::EngineeringUnits;
use std::net::SocketAddrV4;

const DEVICE: u32 = 10;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn identity() -> bacnet_objects::DeviceIdentity {
    bacnet_objects::DeviceIdentity::new(DEVICE, 0).unwrap()
}

fn device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: identity(),
        ..Default::default()
    })
    .unwrap()
}

/// The Device and two values.
fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device())).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    let value = AnalogValueObject::new(2, "AV-2", EngineeringUnits::NO_UNITS).unwrap();
    db.add(Box::new(value)).unwrap();
    db
}

fn config() -> ServerConfig {
    ServerConfig {
        device: identity(),
        ..Default::default()
    }
}

/// A link whose start fails as a socket already in use does.
fn port_in_use() -> TestTransport {
    TestTransport::builder()
        .start_gate(|| async { Err(Error::Transport(std::io::ErrorKind::AddrInUse.into())) })
        .build()
}

/// The refusal's error and the database it hands back, which holds `objects`.
fn refused<T: TransportPort>(
    started: Result<BACnetServer<T>, StartError>,
    objects: &[ObjectIdentifier],
) -> (Error, ObjectDatabase) {
    let Err(refused) = started else {
        panic!("the start should be refused");
    };
    let (error, db) = refused.into_parts();
    let db = db.unwrap_or_else(|| panic!("{error} handed no database back"));
    assert_eq!(db.object_list().collect::<Vec<_>>(), objects, "{error}");
    (error, db)
}

/// Every object in `objects` is served: each reads its name through the
/// server's ReadProperty evaluator, and the Device lists them all.
async fn serves<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    objects: &[ObjectIdentifier],
) {
    for object in objects {
        server
            .read_local(object, PropertyIdentifier::OBJECT_NAME, None)
            .await
            .unwrap_or_else(|error| panic!("{object}: {error}"));
    }
    let listed = server
        .read_local(
            &oid(ObjectType::DEVICE, DEVICE),
            PropertyIdentifier::OBJECT_LIST,
            None,
        )
        .await
        .unwrap();
    let expected = objects
        .iter()
        .map(|object| PropertyValue::ObjectIdentifier(*object))
        .collect();
    assert_eq!(listed, PropertyValue::List(expected));
}

fn in_use(error: &Error) -> bool {
    matches!(error, Error::Transport(error) if error.kind() == std::io::ErrorKind::AddrInUse)
}

#[tokio::test(start_paused = true)]
async fn a_start_refused_by_its_link_hands_the_database_to_the_next_start() {
    let objects: Vec<_> = database().object_list().collect();
    let (error, db) = refused(
        BACnetServer::start(config(), database(), port_in_use()).await,
        &objects,
    );
    assert!(in_use(&error), "{error}");
    let (error, db) = refused(
        BACnetServer::generic_builder()
            .device(identity())
            .database(db)
            .transport(port_in_use())
            .build()
            .await,
        &objects,
    );
    assert!(in_use(&error), "{error}");
    let mut server = BACnetServer::start(config(), db, TestTransport::new())
        .await
        .unwrap();
    serves(&server, &objects).await;
    server.stop().await.unwrap();
}

/// A builder's own checks, before its link starts, hand the database back
/// too: no transport, a setting the start refuses, and a database whose
/// Device isn't the identity's. Nothing is started.
#[tokio::test(start_paused = true)]
async fn a_builder_refused_before_its_link_starts_hands_the_database_back() {
    let objects: Vec<_> = database().object_list().collect();
    let (_, db) = refused(
        BACnetServer::<TestTransport>::generic_builder()
            .database(database())
            .build()
            .await,
        &objects,
    );
    let invalid = CovPolicy {
        max_notifications_per_event: 0,
        ..CovPolicy::default()
    };
    let (error, db) = refused(
        BACnetServer::generic_builder()
            .device(identity())
            .database(db)
            .transport(TestTransport::never_start())
            .cov_policy(invalid)
            .build()
            .await,
        &objects,
    );
    assert!(error.to_string().contains("max_notifications_per_event"));
    let other = bacnet_objects::DeviceIdentity::new(DEVICE + 1, 0).unwrap();
    let (_, db) = refused(
        BACnetServer::generic_builder()
            .device(other)
            .database(db)
            .transport(TestTransport::never_start())
            .build()
            .await,
        &objects,
    );
    let mut server = BACnetServer::generic_builder()
        .device(identity())
        .database(db)
        .transport(TestTransport::new())
        .build()
        .await
        .unwrap();
    serves(&server, &objects).await;
    server.stop().await.unwrap();
}

/// A hub that refuses the dial hands the database back; it then starts a
/// server on another link.
#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn a_refused_sc_dial_hands_the_database_back() {
    let objects: Vec<_> = database().object_list().collect();
    let (_, db) = refused(
        BACnetServer::sc_builder()
            .hub_url("not-a-websocket-url")
            .tls_config(super::sc_builder::test_tls_config())
            .device_uuid(super::sc_builder::TEST_DEVICE_UUID)
            .device(identity())
            .database(database())
            .build()
            .await,
        &objects,
    );
    let mut server = BACnetServer::start(config(), db, TestTransport::new())
        .await
        .unwrap();
    serves(&server, &objects).await;
    server.stop().await.unwrap();
}

/// A node's started link, and what its start gave.
struct Node {
    ingress: EndpointIngress<TestTransport>,
    _receivers: bacnet_endpoint_core::endpoint_ingress::IngressReceivers,
}

impl Node {
    async fn stop(&mut self) -> Result<(), Error> {
        self.ingress.stop().await.map(drop)
    }
}

/// A node's link and a seat on it, as `server_seat_tests` builds them.
async fn node_and_seat() -> (Node, bacnet_endpoint_core::seat::ServerSeat<TestTransport>) {
    let mut ingress = EndpointIngress::new(TestTransport::new(), 8);
    let receivers = ingress.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::with_reserves(
        REQUESTER_RESERVE,
        NOTIFICATION_RESERVE,
    ));
    let seat = ingress.server_seat(coordinator).unwrap();
    let node = Node {
        ingress,
        _receivers: receivers,
    };
    (node, seat)
}

/// A seated start refused by a setting hands the database back; a seat on
/// another node then serves it.
#[tokio::test(start_paused = true)]
async fn a_refused_seated_start_hands_the_database_back() {
    let objects: Vec<_> = database().object_list().collect();
    let (mut node, seat) = node_and_seat().await;
    let invalid = CovPolicy {
        max_notifications_per_event: 0,
        ..CovPolicy::default()
    };
    let (_, db) = refused(
        BACnetServer::generic_builder()
            .device(identity())
            .database(database())
            .cov_policy(invalid)
            .seat(seat)
            .build()
            .await,
        &objects,
    );
    node.stop().await.unwrap();
    let (mut node, seat) = node_and_seat().await;
    let mut server = BACnetServer::generic_builder()
        .device(identity())
        .database(db)
        .seat(seat)
        .build()
        .await
        .unwrap();
    serves(&server, &objects).await;
    server.stop().await.unwrap();
    node.stop().await.unwrap();
}

const PORT: u32 = 1;
const STATION: SocketAddrV4 = SocketAddrV4::new(std::net::Ipv4Addr::new(10, 0, 0, 2), 0xBAC0);
const BROADCAST: SocketAddrV4 =
    SocketAddrV4::new(std::net::Ipv4Addr::new(127, 255, 255, 255), 0xBAC0);

/// The values, a Network Port configured at UDP port 0 and an Audit
/// Reporter, with the Device's Audit recipient at [`STATION`].
fn audited_database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut device = device();
    let mut mac = STATION.ip().octets().to_vec();
    mac.extend_from_slice(&STATION.port().to_be_bytes());
    let recipient = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&mac),
    });
    device.provision_audit_recipient(recipient).unwrap();
    db.add(Box::new(device)).unwrap();
    let port = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: 0,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(PORT, "port", port).unwrap(),
    ))
    .unwrap();
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter
        .set_audit_level(bacnet_types::enums::AuditLevel::AUDIT_ALL)
        .unwrap();
    db.add(Box::new(reporter)).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db
}

/// A B/IP-shaped link configured at 127.0.0.1, port 0, that binds `bound`.
/// With `station_is_broadcast`, once started it reports [`STATION`] as its
/// broadcast endpoint, so the Audit recipient there has no route.
fn bip_link(bound: u16, station_is_broadcast: bool) -> TestTransport {
    let started = Arc::new(AtomicBool::new(false));
    let (on_start, on_port, on_broadcast) = (started.clone(), started.clone(), started);
    TestTransport::builder()
        .local_mac(&[127, 0, 0, 1, 0, 0])
        .normal_bip(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0))
        .on_start(move || on_start.store(true, Ordering::Release))
        .on_bip_port(move |_| {
            let port = if on_port.load(Ordering::Acquire) {
                bound
            } else {
                0
            };
            let endpoint = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, port);
            Some(BipPort::new(endpoint, BipPortMode::Normal))
        })
        .on_bip_broadcast_endpoint(move || {
            Some(
                if station_is_broadcast && on_broadcast.load(Ordering::Acquire) {
                    STATION
                } else {
                    BROADCAST
                },
            )
        })
        .build()
}

fn audited_config() -> ServerConfig {
    ServerConfig {
        registered_network_port: Some(oid(ObjectType::NETWORK_PORT, PORT)),
        audit_reporters: Some(AuditReportersConfig {
            reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
        }),
        ..config()
    }
}

async fn udp_port<T: TransportPort + 'static>(server: &BACnetServer<T>) -> PropertyValue {
    let port = oid(ObjectType::NETWORK_PORT, PORT);
    server
        .read_local(&port, PropertyIdentifier::BACNET_IP_UDP_PORT, None)
        .await
        .unwrap()
}

/// A refusal after the link bound and the registered Network Port took the
/// bind (target Audit's checks of the started link) puts the port back at
/// its configured UDP port 0, so the next start reserves it again and
/// publishes its own bind.
#[tokio::test(start_paused = true)]
async fn a_start_refused_after_publishing_its_port_puts_the_port_back() {
    let objects: Vec<_> = audited_database().object_list().collect();
    let (error, db) = refused(
        BACnetServer::start(audited_config(), audited_database(), bip_link(47809, true)).await,
        &objects,
    );
    assert!(matches!(error, Error::Remote(_)), "{error}");
    let port = db.get(&oid(ObjectType::NETWORK_PORT, PORT)).unwrap();
    assert_eq!(
        port.read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    let mut server = BACnetServer::start(audited_config(), db, bip_link(47810, false))
        .await
        .unwrap();
    serves(&server, &objects).await;
    assert_eq!(udp_port(&server).await, PropertyValue::Unsigned(47810));
    server.stop().await.unwrap();
}

/// A link that won't bind leaves a registered port at UDP port 0 reserved
/// by no one: the next start registers it.
#[tokio::test(start_paused = true)]
async fn a_registered_port_whose_link_would_not_bind_registers_again() {
    let objects: Vec<_> = audited_database().object_list().collect();
    let config = || ServerConfig {
        registered_network_port: Some(oid(ObjectType::NETWORK_PORT, PORT)),
        ..config()
    };
    let refusing = TestTransport::builder()
        .normal_bip(SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0))
        .start_gate(|| async { Err(Error::Transport(std::io::ErrorKind::AddrInUse.into())) })
        .build();
    let (error, db) = refused(
        BACnetServer::start(config(), audited_database(), refusing).await,
        &objects,
    );
    assert!(in_use(&error), "{error}");
    let mut server = BACnetServer::start(config(), db, bip_link(47811, false))
        .await
        .unwrap();
    serves(&server, &objects).await;
    assert_eq!(udp_port(&server).await, PropertyValue::Unsigned(47811));
    server.stop().await.unwrap();
}

#[path = "start_refusal_drop_tests.rs"]
mod drop_tests;
