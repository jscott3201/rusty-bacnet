//! What a session that would build a full server refuses before its link
//! starts (#2617): what it can't build one with, and what the server's own
//! start would refuse, each leaving the session ready to retry with its
//! database.
use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{AuditLevel, AuditOperation};

/// A session that would build its server at start refuses, before its
/// link starts, every setting it could not build one with, and starts once
/// corrected: the refusal leaves it as it was.
/// Refused at start: whose error says it, and whose link never started.
async fn refused(
    session: &mut EndpointSession<TestTransport>,
    link: &TestTransportHandle,
    why: &str,
) {
    let error = Box::pin(session.start())
        .await
        .expect_err("refused at start");
    assert!(error.to_string().contains(why), "{why}: {error}");
    assert!(!session.is_running());
    assert_eq!(link.starts(), 0, "{why}: the link started");
}

#[tokio::test(start_paused = true)]
async fn a_full_server_the_session_could_not_build_is_refused_before_the_link_starts() {
    let wire = Wire::new();
    let identity = || DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let port = wire.port(NODE);
    let link = port.handle();
    let mut session = EndpointSession::new(
        port.build(),
        SessionRole::ClientOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_server(BACnetServer::generic_builder());
    refused(&mut session, &link, "no server role").await;

    let port = wire.port(PEER);
    let link = port.handle();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .with_server(BACnetServer::generic_builder());
    refused(&mut session, &link, "Device identity").await;
    let mut session = session.device(identity());
    // The builder's own database, link, port and identity.
    let cases: [(bacnet_server::server::ServerBuilder<TestTransport>, &str); 4] = [
        (
            BACnetServer::generic_builder().database(database(NODE_DEVICE, 1.0)),
            "give it to the session",
        ),
        (
            BACnetServer::generic_builder().transport(Wire::new().port(9).build()),
            "transport",
        ),
        (
            BACnetServer::generic_builder().registered_network_port(
                ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap(),
            ),
            "registered Network Port",
        ),
        (
            BACnetServer::generic_builder()
                .device(DeviceIdentity::new(NODE_DEVICE + 1, 42).unwrap()),
            "Device identity",
        ),
    ];
    for (builder, why) in cases {
        session = session.with_server(builder);
        refused(&mut session, &link, why).await;
    }
    // Corrected, it starts and serves.
    let port = wire.port(OTHER);
    let link = port.handle();
    let mut session = EndpointSession::new(
        port.build(),
        SessionRole::ServerOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_server(BACnetServer::generic_builder());
    refused(&mut session, &link, "Device identity").await;
    let mut session = session.device(identity());
    session.start().await.unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    assert!(
        session
            .server()
            .unwrap()
            .read_local(&device, PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .await
            .is_ok()
    );
    session.stop().await.unwrap();
    assert_eq!(link.stops(), 1);
}

/// What every server start checks of its settings, on the settings the
/// seat gives the server, is checked before the session's link starts too:
/// the policies and budgets the server's requests are held to, the device
/// bindings against the link's group rule, the Audit Reporters and their
/// routes, and the Device's segment timeout. Each refusal leaves the
/// session ready, with its database: corrected, it starts and serves it.
#[tokio::test(start_paused = true)]
async fn a_server_setting_its_start_would_refuse_is_refused_before_the_link_starts() {
    use bacnet_server::server::{AuditReportersConfig, DeviceBinding, TimeSyncPolicy};
    let wire = Wire::new();
    let identity = || DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    // A MAC the link's own group check calls a group address: its
    // transport's, which a server's start asks, not a stored copy.
    let group = Wire::mac(77);
    let member = group.clone();
    let port = wire
        .port(PEER)
        .on_is_group_destination(move |mac| mac == member.as_slice());
    let link = port.handle();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(identity())
            .with_database(database(NODE_DEVICE, 7.0));
    let device = |instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    let reporter = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();
    let time_sync = TimeSyncPolicy {
        max_sources: 0,
        ..TimeSyncPolicy::default()
    };
    let cases: [(bacnet_server::server::ServerBuilder<TestTransport>, &str); 4] = [
        (
            BACnetServer::generic_builder().time_sync_policy(time_sync),
            "max_sources",
        ),
        (
            BACnetServer::generic_builder()
                .device_binding(DeviceBinding::local(device(9), &group).unwrap())
                .unwrap(),
            "group address",
        ),
        (
            BACnetServer::generic_builder().audit_reporters(AuditReportersConfig {
                reporters: Vec::new(),
            }),
            "1..=64",
        ),
        // A Reporter the database lacks, on a Device with no Audit
        // recipient: refused as the database's.
        (
            BACnetServer::generic_builder().audit_reporters(AuditReportersConfig {
                reporters: vec![reporter],
            }),
            "write-access-denied",
        ),
    ];
    for (builder, why) in cases {
        session = session.with_server(builder);
        refused(&mut session, &link, why).await;
    }
    // Corrected, it starts and serves the database it was given.
    session = session.with_server(BACnetServer::generic_builder());
    Box::pin(session.start()).await.unwrap();
    assert_eq!(link.starts(), 1);
    let value = session
        .server()
        .unwrap()
        .read_local(&ai1(), PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    assert_eq!(value, PropertyValue::Real(7.0));
    session.stop().await.unwrap();

    // The Device's segment timeout, which the server's start checks.
    let port = wire.port(OTHER);
    let link = port.handle();
    let long = identity().with_apdu_segment_timeout_ms(u64::MAX / 2);
    let db = build_database_with_extra(&long, Vec::new()).unwrap();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(long)
            .with_database(db)
            .with_server(BACnetServer::generic_builder());
    refused(&mut session, &link, "segment timeout").await;
}

/// A B/IP-shaped link's broadcast endpoint, 10.0.0.255:0xBAC0; its group
/// rule also takes in the broadcast IP at another port.
const BROADCAST: std::net::SocketAddrV4 =
    std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
const BROADCAST_ELSEWHERE: [u8; 6] = [10, 0, 0, 255, 0xBA, 0xC1];
const STATION: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];

/// The node's database with Audit Reporter 1, recording writes confirmed,
/// and the Device's Audit recipient at `recipient` on this network.
fn audited(recipient: &[u8]) -> bacnet_objects::database::ObjectDatabase {
    let identity = DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut reporter = AuditReporterObject::new(1, "Reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations).unwrap();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut db = build_database_with_extra(&identity, vec![Box::new(reporter)]).unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    db.get_mut(&device)
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(recipient),
        }))
        .unwrap();
    db
}

/// A builder with target Audit is checked against the link before it
/// starts: an Audit recipient at an address the link calls a group is
/// refused with the link unstarted, and one at a station starts.
#[tokio::test(start_paused = true)]
async fn an_audit_recipient_at_a_group_address_is_refused_before_the_link_starts() {
    for (recipient, refused_why) in [(BROADCAST_ELSEWHERE, Some("denied")), (STATION, None)] {
        let wire = Wire::new();
        let port = wire
            .port(PEER)
            .on_bip_broadcast_endpoint(|| Some(BROADCAST))
            .group_mac(&BROADCAST_ELSEWHERE);
        let link = port.handle();
        let reporter = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();
        let builder = BACnetServer::generic_builder().audit_reporters(
            bacnet_server::server::AuditReportersConfig {
                reporters: vec![reporter],
            },
        );
        let mut session =
            EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
                .unwrap()
                .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
                .with_database(audited(&recipient))
                .with_server(builder);
        match refused_why {
            Some(why) => refused(&mut session, &link, why).await,
            None => {
                Box::pin(session.start()).await.unwrap();
                assert_eq!(link.starts(), 1);
                session.stop().await.unwrap();
            }
        }
    }
}
