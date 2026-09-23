//! Typed source recipient provision, route configuration and atomic preflight.

use super::*;
use crate::bip::BipEndpointBuilder;
use bacnet_types::enums::Reliability;
use std::net::{Ipv4Addr, SocketAddrV4};

const BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);

fn builder() -> BipEndpointBuilder {
    BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, BROADCAST)
}

fn destination() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, 456)
}

fn address() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 42), 0xbac1)
}

fn reliability(object: &dyn BACnetObject) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
}

// Move a builder-validated value into the existing counting fixture solely to
// observe preflight/start/send ordering. There is no generic public setter.
fn configured_session(
    role: SessionRole,
) -> (
    EndpointSession<ObservedTransport>,
    LoopbackTransport,
    Arc<Observations>,
) {
    let mut built = builder()
        .source_audit_device_binding(destination(), address())
        .build_session()
        .unwrap();
    let (mut session, peer, observed) = session(role);
    session.source_audit_bindings = std::mem::take(&mut built.source_audit_bindings);
    (session, peer, observed)
}

#[test]
fn source_bindings_validate_all_entries_and_cannot_be_discarded() {
    for (device, endpoint) in [
        (selected(), address()),
        (
            oid(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE),
            address(),
        ),
        (
            destination(),
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 47808),
        ),
        (
            destination(),
            SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 1), 47808),
        ),
        (destination(), SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808)),
        (destination(), SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
    ] {
        assert!(builder()
            .source_audit_device_binding(device, endpoint)
            .build_session()
            .is_err());
    }
    assert!(builder()
        .source_audit_device_binding(destination(), address())
        .source_audit_device_binding(destination(), address())
        .build_session()
        .is_err());
    assert!(builder()
        .source_audit_device_binding(destination(), address())
        .build_transport()
        .is_err());
    for instance in [0, ObjectIdentifier::MAX_INSTANCE - 1] {
        assert!(builder()
            .source_audit_device_binding(oid(ObjectType::DEVICE, instance), address())
            .build_session()
            .is_ok());
    }
}

#[tokio::test]
async fn provision_and_bindings_without_source_selection_remain_inert() {
    let (session, _peer, observed) = configured_session(SessionRole::ClientOnly);
    let mut session = session.with_database(database());
    session.start().await.unwrap();
    assert!(!session
        .database
        .as_ref()
        .unwrap()
        .read()
        .await
        .get(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .property_list()
        .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
    assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_requires_client_capable_role_before_wrapping_or_start() {
    let (session, _peer, observed) = configured_session(SessionRole::ServerOnly);
    let mut session = session
        .with_database(database())
        .with_source_audit_reporter(selected());
    rejected(&mut session, &observed, "requires a client role").await;
}

#[tokio::test]
async fn recipient_preserves_all_source_preflight_failures_and_retry() {
    for case in [
        "no database",
        "no device",
        "two devices",
        "identity mismatch",
        "wrong selection type",
        "missing reporter",
        "no capability",
        "capability mismatch",
        "conflicting source",
        "unknown source",
    ] {
        let (session, _peer, observed) = configured_session(SessionRole::Both);
        let mut db = database();
        let expected = match case {
            "no database" => "attached local database",
            "no device" => {
                db.remove(&oid(ObjectType::DEVICE, 123)).unwrap();
                "exactly one"
            }
            "two devices" => {
                let mut other = crate::identity::DeviceIdentity::new(456, 42)
                    .unwrap()
                    .build_database()
                    .unwrap();
                db.add(other.remove(&destination()).unwrap().unwrap())
                    .unwrap();
                "exactly one"
            }
            "identity mismatch" => "does not match session identity",
            "wrong selection type" => "must be an Audit Reporter",
            "missing reporter" => "absent from the database",
            "no capability" | "capability mismatch" => {
                db.add(Box::new(ForeignReporter {
                    oid: selected(),
                    source: Some(false),
                    capability: (case == "capability mismatch")
                        .then(|| AuditReporterObject::new(3, "Wrong identity").unwrap()),
                }))
                .unwrap();
                "lacks the Audit Reporter capability"
            }
            _ => {
                db.add(Box::new(ForeignReporter {
                    oid: target(),
                    source: (case == "conflicting source").then_some(true),
                    capability: None,
                }))
                .unwrap();
                if case == "conflicting source" {
                    "conflicting source"
                } else {
                    "cannot determine"
                }
            }
        };
        let mut session = session
            .with_identity(
                crate::identity::DeviceIdentity::new(
                    if case == "identity mismatch" {
                        456
                    } else {
                        123
                    },
                    42,
                )
                .unwrap(),
            )
            .with_source_audit_reporter(match case {
                "wrong selection type" => destination(),
                "missing reporter" => oid(ObjectType::AUDIT_REPORTER, 99),
                _ => selected(),
            });
        if case != "no database" {
            session = session.with_database(db);
        }
        let bindings = session.source_audit_bindings.clone();
        // Reuses pointer/source/deletion assertions and exact start/send counters.
        rejected(&mut session, &observed, expected).await;
        assert_eq!(session.source_audit_bindings, bindings);
        session = session
            .with_database(database())
            .with_identity(crate::identity::DeviceIdentity::new(123, 42).unwrap())
            .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        session.stop().await.unwrap();
        assert_eq!(session.source_audit_bindings, bindings);
        assert_eq!(observed.starts.load(Ordering::SeqCst), 1);
        assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
        assert_eq!(session.active_leases(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn retained_config_sets_availability_without_clearing_failure_or_sending() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for configured in [false, true] {
            let (session, _peer, observed) = configured_session(role);
            let db = database();
            let reporter = db
                .get(&selected())
                .unwrap()
                .audit_reporter_internal()
                .unwrap();
            let status = reporter.status_internal();
            status.set_configured(configured);
            // Existing failures must not be cleared by static configuration.
            status.complete_delivery(status.begin_delivery(), false);
            let expected = PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw());
            let epoch = status.begin_delivery();
            let auditing_failure_epoch = status.auditing_failure_epoch();
            let mut session = session
                .with_database(db)
                .with_source_audit_reporter(selected());
            let bindings = session.source_audit_bindings.clone();
            session.start().await.unwrap();
            tokio::time::advance(Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            assert_eq!(session.source_audit_bindings, bindings);
            assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
            assert_eq!(session.active_leases(), 0);
            {
                let db = session.database.as_ref().unwrap().read().await;
                assert_eq!(reliability(db.get(&selected()).unwrap()), expected);
            }
            let weak = Arc::downgrade(session.database.as_ref().unwrap());
            session.stop().await.unwrap();
            assert_eq!(session.source_audit_bindings, bindings);
            assert_eq!(session.active_leases(), 0);
            drop(session);
            assert!(weak.upgrade().is_none());
            assert_eq!(observed.starts.load(Ordering::SeqCst), 1);
            assert_eq!(observed.stops.load(Ordering::SeqCst), 1);
            assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
            assert_eq!(status.begin_delivery(), epoch);
            assert_eq!(status.auditing_failure_epoch(), auditing_failure_epoch);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn drop_without_stop_adds_no_recipient_work_or_retained_database() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let (session, _peer, observed) = configured_session(role);
        let mut session = session
            .with_database(database())
            .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        let weak = Arc::downgrade(session.database.as_ref().unwrap());
        let coordinator = session.coordinator();
        drop(session);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(weak.upgrade().is_none());
        assert_eq!(coordinator.active_count().unwrap(), 0);
        assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn direct_bip_ipv4_client_only_and_both_start_stop_silently() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for confirmed in [false, true] {
            let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let std::net::SocketAddr::V4(address) = peer.local_addr().unwrap() else {
                panic!("IPv4 peer required");
            };
            let mut db = database();
            db.get_mut(&selected())
                .unwrap()
                .configure_audit_reporter_internal(
                    AuditLevel::AUDIT_ALL,
                    bacnet_types::bitstring::AuditOperationFlags::from_bits(0xff).unwrap(),
                    confirmed,
                    None,
                    bacnet_types::bitstring::BACnetPriorityFilter::all(),
                )
                .unwrap();
            let reporter = db
                .get(&selected())
                .unwrap()
                .audit_reporter_internal()
                .unwrap();
            let original = std::ptr::from_ref(reporter);
            let status = reporter.status_internal();
            let initial_delivery = status.begin_delivery();
            let configuration_error =
                PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw());
            assert_eq!(reliability(reporter), configuration_error);
            let mut session = builder()
                .role(role)
                .database(db)
                .source_audit_device_binding(destination(), address)
                .build_session()
                .unwrap();
            if role == SessionRole::Both {
                session = session.with_device_writes(Arc::new(|_| true));
            }
            session = session.with_source_audit_reporter(selected());
            let bindings = session.source_audit_bindings.clone();
            session.start().await.unwrap();
            assert!(session.client().is_some());
            assert_eq!(session.server().is_some(), role == SessionRole::Both);
            assert_eq!(session.source_audit_bindings, bindings);
            {
                let db = session.database.as_ref().unwrap().read().await;
                assert!(source(&db, selected()));
                assert!(!source(&db, target()));
                let reporter = db
                    .get(&selected())
                    .unwrap()
                    .audit_reporter_internal()
                    .unwrap();
                assert!(std::ptr::eq(original, reporter));
                assert!(Arc::ptr_eq(&status, &reporter.status_internal()));
                assert_eq!(
                    reliability(reporter),
                    PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
                );
                let device = db.get(&oid(ObjectType::DEVICE, 123)).unwrap();
                assert!(device
                    .property_list()
                    .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
                assert!(device
                    .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
                    .is_ok());
                assert!(device
                    .required_properties()
                    .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
                assert!(
                    device.is_writable_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT)
                );
                assert!(!device
                    .required_properties()
                    .contains(&PropertyIdentifier::AUDIT_LEVEL));
                assert!(!device
                    .required_properties()
                    .contains(&PropertyIdentifier::AUDITABLE_OPERATIONS));
            }
            let mut bytes = [0; 2048];
            assert!(
                tokio::time::timeout(Duration::from_millis(30), peer.recv_from(&mut bytes))
                    .await
                    .is_err()
            );
            assert_eq!(session.active_leases(), 0);
            session.stop().await.unwrap();
            assert_eq!(session.source_audit_bindings, bindings);
            drop(session);
            assert!(
                matches!(peer.try_recv_from(&mut bytes), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
            );
            assert_eq!(status.begin_delivery(), initial_delivery);
        }
    }
}

#[tokio::test]
async fn source_post_start_broadcast_fact_invalidates_device_route_without_callbacks() {
    let (session, _peer, observed) = session(SessionRole::ClientOnly);
    let mut session = session
        .with_database(database())
        .with_source_audit_reporter(selected());
    session
        .source_audit_bindings
        .push((destination(), "192.168.1.255:47808".parse().unwrap()));
    session.start().await.unwrap();
    assert_eq!(observed.bip_reads.load(Ordering::SeqCst), 2);
    {
        let mut db = session.database.as_ref().unwrap().write().await;
        let reporter = db
            .get(&selected())
            .unwrap()
            .audit_reporter_internal()
            .unwrap();
        assert_eq!(
            reliability(reporter),
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        let mut bytes = bytes::BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut bytes,
            &bacnet_types::constructed::BACnetRecipient::Address(
                bacnet_types::constructed::BACnetAddress {
                    network_number: 0,
                    mac_address: bacnet_types::MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC0]),
                },
            ),
        );
        assert!(db
            .get_mut(&oid(ObjectType::DEVICE, 123))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(bytes.to_vec()),
                None
            )
            .is_err());
    }
    assert_eq!(observed.bip_reads.load(Ordering::SeqCst), 2);
    assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    session.stop().await.unwrap();
}
