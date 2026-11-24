//! A refused build or start hands the object database back (#2766): each
//! link builder's refusal gives back the database it was given, as given,
//! and a start that ends its session gives back the one it took. Either
//! serves every object from the next start.
use super::*;
use bacnet_transport::bip::{BipMulticast, BipTransport};
use bacnet_transport::mstp::LoopbackSerial;
use bacnet_transport::sc::LoopbackWebSocket;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};

fn identity() -> crate::DeviceIdentity {
    crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap()
}

fn objects(db: &ObjectDatabase) -> Vec<ObjectIdentifier> {
    db.object_list().collect()
}

/// The error and the database `refused` must carry.
fn handed_back(refused: StartError) -> (Error, ObjectDatabase) {
    match refused.into_parts() {
        (error, Some(db)) => (error, db),
        (error, None) => panic!("no database came back with: {error}"),
    }
}

/// A peer on `wire` reads every one of `objects` from the node.
async fn reads_every_object(wire: &Wire, objects: &[ObjectIdentifier]) {
    let mut peer = peer(wire).await;
    for &oid in objects {
        let read = peer
            .read_property(&Wire::mac(NODE), oid, PropertyIdentifier::OBJECT_NAME, None)
            .await;
        assert!(read.is_ok_and(|ack| ack.object_identifier == oid), "{oid}");
    }
    peer.stop().await.unwrap();
}

/// `database` serves each of its objects from a session of both roles on
/// a good link.
async fn serves_every_object(database: ObjectDatabase) {
    let expected = objects(&database);
    let wire = Wire::new();
    let mut session = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_database(database);
    session.start().await.unwrap();
    reads_every_object(&wire, &expected).await;
    session.stop().await.unwrap();
}

/// A routed source Audit binding to network 0, which every builder refuses.
fn network_zero() -> (ObjectIdentifier, u16, MacAddr) {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap();
    (device, 0, MacAddr::from_slice(&[5]))
}

/// The B/IP builder gives the database back from a refusal before it moves
/// out of the builder (routes, session settings) and from one after (the
/// transport's multicast refusal); it is the database given each time.
#[tokio::test(start_paused = true)]
async fn a_refused_bip_build_hands_the_database_back_as_given() {
    use crate::bip::BipEndpointBuilder;
    let mut db = database();
    let expected = objects(&db);
    let (device, network, mac) = network_zero();
    let group = BipMulticast::new(Ipv4Addr::new(239, 255, 0, 1)).unwrap();
    let local = || BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let refusals = [
        local().source_audit_routed_device_binding(device, network, mac),
        local().queue_capacity(0),
        BipEndpointBuilder::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST)
            .multicast(Some(group)),
    ];
    for (n, builder) in refusals.into_iter().enumerate() {
        let refused = builder.device(identity()).database(db).build_session();
        let (error, back) = handed_back(refused.err().expect("refused"));
        assert_eq!(objects(&back), expected, "refusal {n}: {error}");
        db = back;
    }
    // A session setting is checked before the transport's multicast group.
    let both = BipEndpointBuilder::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST)
        .multicast(Some(group))
        .queue_capacity(0);
    let refused = both.database(db).build_session().err().expect("refused");
    let (error, back) = handed_back(refused);
    assert!(error.to_string().contains("queue capacity"), "{error}");
    db = back;
    let wire = Wire::new();
    let mut session = local()
        .device(identity())
        .database(db)
        .build_session_over(wire.port(NODE).build())
        .unwrap();
    session.start().await.unwrap();
    reads_every_object(&wire, &expected).await;
    session.stop().await.unwrap();
}

/// The MS/TP builder's refusals, its link settings', the session's and a
/// route's, give the database back as given.
#[tokio::test(start_paused = true)]
async fn a_refused_mstp_build_hands_the_database_back_as_given() {
    use crate::mstp::MstpEndpointBuilder;
    let mut db = database();
    let expected = objects(&db);
    let (device, network, mac) = network_zero();
    let station = || MstpEndpointBuilder::new(LoopbackSerial::pair().0, 3);
    let refusals = [
        station().max_master(2),
        station().queue_capacity(0),
        station().source_audit_routed_device_binding(device, network, mac),
    ];
    for (n, builder) in refusals.into_iter().enumerate() {
        let refused = builder.device(identity()).database(db).build_session();
        let (error, back) = handed_back(refused.err().expect("refused"));
        assert_eq!(objects(&back), expected, "refusal {n}: {error}");
        db = back;
    }
    // A session setting is checked before the link's source Audit bindings.
    let both = station()
        .source_audit_routed_device_binding(device, network, MacAddr::from_slice(&[5]))
        .queue_capacity(0);
    let refused = both.database(db).build_session().err().expect("refused");
    let (error, db) = handed_back(refused);
    assert!(error.to_string().contains("queue capacity"), "{error}");
    // Serving over MS/TP takes a token-passing peer: `rb17_mstp_proof`
    // serves a refused build's database from the MS/TP builder.
    serves_every_object(db).await;
}

/// The SC builder's refusals give the database back as given. A loopback
/// session serves only with a hub's side of the handshake driven by hand:
/// `sc_link` dials a real hub, after a failed dial, to serve the database
/// from the SC builder.
#[tokio::test(start_paused = true)]
async fn a_refused_sc_build_hands_the_database_back_as_given() {
    use crate::sc::ScEndpointBuilder;
    let mut db = database();
    let expected = objects(&db);
    let (device, network, mac) = network_zero();
    let node = || ScEndpointBuilder::new([0x02, 0, 0, 0, 0, 0x01], [0x11; 16]);
    let refusals = [
        ScEndpointBuilder::new([0; 6], [0x11; 16]),
        node().heartbeat(1_000, 2_000),
        node().queue_capacity(0),
        node().source_audit_routed_device_binding(device, network, mac),
    ];
    for (n, builder) in refusals.into_iter().enumerate() {
        let refused = builder
            .device(identity())
            .database(db)
            .build_loopback_session(LoopbackWebSocket::pair().0);
        let (error, back) = handed_back(refused.err().expect("refused"));
        assert_eq!(objects(&back), expected, "refusal {n}: {error}");
        db = back;
    }
    serves_every_object(db).await;
}

/// A link that won't start ends the session, whose start hands back the
/// database it was given, or the stock one it built from the identity. The
/// session keeps nothing for a second start.
#[tokio::test(start_paused = true)]
async fn a_link_that_wont_start_hands_the_database_back() {
    for given in [true, false] {
        let wire = Wire::new();
        let refusing = wire
            .port(NODE)
            .start_gate(|| async { Err(Error::Transport(std::io::ErrorKind::AddrInUse.into())) })
            .build();
        let session = EndpointSession::new(refusing, SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(identity());
        let (mut session, expected) = if given {
            let db = database();
            let expected = objects(&db);
            (session.with_database(db), expected)
        } else {
            (session, vec![identity().device_oid()])
        };
        let (error, db) = handed_back(session.start().await.unwrap_err());
        assert!(matches!(error, Error::Transport(_)), "{error}");
        assert_eq!(objects(&db), expected);
        assert!(!session.can_start(), "the session ended");
        let again = session.start().await.unwrap_err();
        assert!(again.database().is_none(), "{again}");
        assert!(session.stop().await.is_err());
        serves_every_object(db).await;
    }
}

/// A refusal before the link starts leaves the session ready, its database
/// with it: the error carries none, and the corrected session serves it.
#[tokio::test(start_paused = true)]
async fn a_refusal_before_the_link_keeps_the_database_with_the_session() {
    let wire = Wire::new();
    let db = database();
    let expected = objects(&db);
    let mut session = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(db);
    let refused = session.start().await.unwrap_err();
    assert!(refused.to_string().contains("identity"), "{refused}");
    assert!(refused.database().is_none());
    assert!(
        session.can_start(),
        "the session can be corrected and retried"
    );
    let mut session = session.device(identity());
    session.start().await.unwrap();
    reads_every_object(&wire, &expected).await;
    session.stop().await.unwrap();
}

/// The station target Audit's recipient names, which the started link
/// reports as its broadcast endpoint.
const STATION: std::net::SocketAddrV4 =
    std::net::SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 2), 0xBAC0);

/// [`database`] with its Device's Audit recipient at [`STATION`] and an
/// Audit Reporter, and the server that selects that Reporter.
fn audited() -> (ObjectDatabase, ServerBuilder<TestTransport>) {
    let mut db = database();
    let mut mac = STATION.ip().octets().to_vec();
    mac.extend_from_slice(&STATION.port().to_be_bytes());
    provision(&mut db, &mac);
    let reporter = bacnet_objects::audit::AuditReporterObject::new(1, "Reporter").unwrap();
    db.add(Box::new(reporter)).unwrap();
    let audited = read_only_server().audit_reporters(bacnet_server::server::AuditReportersConfig {
        reporters: vec![ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap()],
    });
    (db, audited)
}

/// Give the Device of `db` the Audit recipient at `mac` on this network.
fn provision(db: &mut ObjectDatabase, mac: &[u8]) {
    let device = identity().device_oid();
    db.get_mut(&device)
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(mac),
        }))
        .unwrap();
}

/// A link that reports `broadcast` as its broadcast endpoint once
/// started, 10.0.0.255 before.
fn reporting(wire: &Wire, broadcast: std::net::SocketAddrV4) -> TestTransport {
    let started = Arc::new(AtomicBool::new(false));
    let (on_start, on_broadcast) = (Arc::clone(&started), started);
    wire.port(NODE)
        .on_start(move || on_start.store(true, Ordering::Release))
        .on_bip_broadcast_endpoint(move || {
            Some(if on_broadcast.load(Ordering::Acquire) {
                broadcast
            } else {
                std::net::SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 255), 0xBAC0)
            })
        })
        .build()
}

/// A server build the started link refuses ends the session; the start
/// hands the database back, as the server's refusal gave it, and it serves
/// every object from the next start. Target Audit's checks of the started
/// link, which reports the Audit recipient's address as its broadcast once
/// started, refuse after the link is up (#2752).
#[tokio::test(start_paused = true)]
async fn a_server_build_the_started_link_refuses_hands_the_database_back() {
    let (db, server) = audited();
    let expected = objects(&db);
    let link = reporting(&Wire::new(), STATION);
    let mut session = EndpointSession::new(link, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(identity())
        .with_database(db)
        .with_server(server);
    let (error, db) = handed_back(session.start().await.unwrap_err());
    assert!(
        error.remote()
            == Some(RemoteError::Error {
                class: ErrorClass::SERVICES,
                code: ErrorCode::SERVICE_REQUEST_DENIED,
            }),
        "{error}"
    );
    assert!(session.server().is_none());
    assert_eq!(objects(&db), expected);
    let wire = Wire::new();
    let unicast = std::net::SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
    let mut session = EndpointSession::new(
        reporting(&wire, unicast),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_database(db)
    .with_server(audited().1);
    session.start().await.unwrap();
    reads_every_object(&wire, &expected).await;
    session.stop().await.unwrap();
}

/// Refusals once the start has built its server.
#[path = "session_hand_back_built_tests.rs"]
mod built;

/// A start refused after it published its registered Network Port, bound
/// to an ephemeral UDP port, hands the port back as configured (UDP 0), so
/// the next start reserves it again on another ephemeral port.
#[tokio::test]
async fn a_published_port_comes_back_as_configured() {
    let port = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
    let identity = crate::DeviceIdentity::new(785, 555)
        .unwrap()
        .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)
        .unwrap();
    let link = || BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    // A client-only session with an APDU size no request can state is
    // refused after the link started and the port was published.
    let config = SessionConfig {
        max_apdu_length: 1,
        ..SessionConfig::default()
    };
    let mut session = EndpointSession::new(link(), SessionRole::ClientOnly, config)
        .unwrap()
        .with_database(identity.build_database().unwrap())
        .with_registered_network_port(port);
    let (_, db) = handed_back(session.start().await.unwrap_err());
    let mut session = EndpointSession::new(link(), SessionRole::ServerOnly, Default::default())
        .unwrap()
        .device(identity)
        .with_database(db)
        .with_registered_network_port(port);
    session.start().await.unwrap();
    assert!(
        session
            .bip_local_address()
            .is_some_and(|bound| bound.port() != 0)
    );
    session.stop().await.unwrap();
}
