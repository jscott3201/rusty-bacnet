use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::{ConfirmedServiceChoice, NetworkPriority, ServiceSupported};
use bytes::BytesMut;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

struct CountedTransport {
    inner: LoopbackTransport,
    starts: Arc<AtomicUsize>,
}
impl TransportPort for CountedTransport {
    fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        Some("255.255.255.255:47808".parse().unwrap())
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, data: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(data, mac).await
    }
    async fn send_broadcast(&self, data: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(data).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

fn session(
    role: SessionRole,
) -> (
    EndpointSession<CountedTransport>,
    LoopbackTransport,
    Arc<AtomicUsize>,
) {
    let (inner, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let starts = Arc::new(AtomicUsize::new(0));
    (
        EndpointSession::new(
            CountedTransport {
                inner,
                starts: starts.clone(),
            },
            role,
            SessionConfig::default(),
        )
        .unwrap(),
        peer,
        starts,
    )
}

fn oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}
fn database(instances: &[u32]) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    for &instance in instances {
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance,
                name: format!("device-{instance}"),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
    }
    db
}
async fn read(
    session: &EndpointSession<impl TransportPort + 'static>,
    property: PropertyIdentifier,
) -> PropertyValue {
    session
        .database
        .as_ref()
        .unwrap()
        .read()
        .await
        .get(&oid(123))
        .unwrap()
        .read_property(property, None)
        .unwrap()
}
fn services(value: PropertyValue) -> Vec<usize> {
    let PropertyValue::BitString { data, .. } = value else {
        panic!("expected bit string")
    };
    (0..data.len() * 8)
        .filter(|&i| data[i / 8] & (0x80 >> (i % 8)) != 0)
        .collect()
}

#[tokio::test]
async fn device_write_profile_matches_execution_with_and_without_identity() {
    for identity in [false, true] {
        for role in [SessionRole::Both, SessionRole::ServerOnly] {
            let (session, _peer, starts) = session(role);
            let mut session = session
                .with_database(database(&[123]))
                .with_device_writes(Arc::new(|_| true));
            if identity {
                session = session.with_identity(crate::DeviceIdentity::new(123, 42).unwrap());
            }
            session.start().await.unwrap();
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert_eq!(
                services(read(&session, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED).await),
                vec![12, 15]
            );
            if let Some(identity) = session.identity() {
                assert_eq!(
                    identity.services(),
                    &[
                        ServiceSupported::READ_PROPERTY,
                        ServiceSupported::WRITE_PROPERTY
                    ]
                );
            }
            session.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn device_write_preflight_errors_are_atomic_and_retryable() {
    for case in [
        "client",
        "missing",
        "empty",
        "ambiguous",
        "wildcard",
        "mismatch",
        "services",
        "source",
    ] {
        let (session, _peer, starts) = session(if case == "client" {
            SessionRole::ClientOnly
        } else {
            SessionRole::Both
        });
        let mut session = session.with_device_writes(Arc::new(|_| true));
        if case != "missing" {
            session = session.with_database(database(match case {
                "empty" => &[],
                "ambiguous" => &[123, 456],
                "wildcard" => &[ObjectIdentifier::MAX_INSTANCE],
                _ => &[123],
            }));
        }
        let identity =
            crate::DeviceIdentity::new(if case == "mismatch" { 456 } else { 123 }, 42).unwrap();
        session = session.with_identity(if case == "services" {
            identity.with_services(&[ServiceSupported::WRITE_PROPERTY_MULTIPLE])
        } else {
            identity
        });
        if case == "source" {
            session = session.with_source_audit_reporter(
                ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 88).unwrap(),
            );
        }
        let before_identity = session.identity().unwrap().services().to_vec();
        let before_device = if ["client", "mismatch", "services", "source"].contains(&case) {
            Some(read(&session, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED).await)
        } else {
            None
        };
        assert!(session.start().await.is_err(), "{case}");
        assert_eq!(starts.load(Ordering::SeqCst), 0, "{case}");
        assert_eq!(
            session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Ready as u8
        );
        assert_eq!(session.identity().unwrap().services(), before_identity);
        if let Some(before) = before_device {
            assert_eq!(
                read(&session, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED).await,
                before,
                "{case}"
            );
        }
        // Public configuration repair retains the same unstarted transport.
        // Role/source selection have no removal setters; those cases are checked
        // for atomic failure above, then dropped without inventing repair APIs.
        if case != "client" && case != "source" {
            session = session
                .with_database(database(&[123]))
                .with_identity(crate::DeviceIdentity::new(123, 42).unwrap());
            session.start().await.unwrap();
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            session.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn endpoint_device_write_actual_ingress_rejects_source_reporter_and_stops() {
    let (session, mut peer, _) = session(SessionRole::Both);
    let mut db = database(&[123]);
    db.get_mut(&oid(123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(bacnet_types::constructed::BACnetRecipient::Device(oid(999)))
        .unwrap();
    let reporter = bacnet_objects::audit::AuditReporterObject::new(1, "Source").unwrap();
    let selected = reporter.object_identifier();
    db.add(Box::new(reporter)).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut session = session
        .with_database(db)
        .with_source_audit_reporter(selected)
        .with_device_writes(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            true
        }));
    let mut receiver = peer.start().await.unwrap();
    session.start().await.unwrap();
    for (target, invoke) in [(oid(123), 41), (selected, 42)] {
        let mut value = BytesMut::new();
        bacnet_encoding::primitives::encode_app_character_string(&mut value, "wire update")
            .unwrap();
        let mut service = BytesMut::new();
        bacnet_services::write_property::WritePropertyRequest {
            object_identifier: target,
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            property_value: value.to_vec(),
            priority: None,
        }
        .encode(&mut service)
        .unwrap();
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 480,
                invoke_id: invoke,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                service_request: service.freeze(),
            }),
        )
        .unwrap();
        let mut bytes = BytesMut::new();
        encode_npdu(
            &mut bytes,
            &Npdu {
                expecting_reply: true,
                priority: NetworkPriority::NORMAL,
                payload: apdu.freeze(),
                ..Default::default()
            },
        )
        .unwrap();
        peer.send_unicast(&bytes, &[1]).await.unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let response = decode_apdu(
            bacnet_encoding::npdu::decode_npdu(received.npdu)
                .unwrap()
                .payload,
        )
        .unwrap();
        if target == oid(123) {
            assert!(matches!(response, Apdu::SimpleAck(ack) if ack.invoke_id == invoke));
        } else {
            assert!(
                matches!(response, Apdu::Error(error) if error.invoke_id == invoke && error.error_code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED)
            );
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        read(&session, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("wire update".into())
    );
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .get(&selected)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString(String::new())
    );
    let server = session.server().unwrap().clone();
    session.stop().await.unwrap();
    assert!(!server.is_session_alive());
    peer.stop().await.unwrap();
}

#[test]
fn bip_device_writes_cannot_be_discarded_by_bare_transport_build() {
    assert!(crate::bip::BipEndpointBuilder::new(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        std::net::Ipv4Addr::BROADCAST
    )
    .device_writes(Arc::new(|_| true))
    .build_transport()
    .is_err());
}

#[tokio::test]
async fn device_write_rejects_device_shaped_custom_object_without_authority() {
    struct DeviceShaped(DeviceObject);
    impl BACnetObject for DeviceShaped {
        fn object_identifier(&self) -> ObjectIdentifier {
            self.0.object_identifier()
        }
        fn object_name(&self) -> &str {
            self.0.object_name()
        }
        fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
            self.0.property_list()
        }
        fn read_property(
            &self,
            p: PropertyIdentifier,
            i: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            self.0.read_property(p, i)
        }
        fn write_property(
            &mut self,
            _: PropertyIdentifier,
            _: Option<u32>,
            _: PropertyValue,
            _: Option<u8>,
        ) -> Result<(), Error> {
            panic!("custom mutation reached")
        }
    }
    let (session, _peer, starts) = session(SessionRole::Both);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(DeviceShaped(
        DeviceObject::new(DeviceConfig {
            instance: 123,
            ..Default::default()
        })
        .unwrap(),
    )))
    .unwrap();
    let mut session = session
        .with_database(db)
        .with_device_writes(Arc::new(|_| true));
    assert!(session.start().await.is_err());
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(
        session.lifecycle.load(Ordering::Acquire),
        Lifecycle::Ready as u8
    );
}

#[tokio::test]
async fn bip_device_write_authorized_round_trip_and_service_readback() {
    use std::net::{Ipv4Addr, UdpSocket};
    let reservation = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut session =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(database(&[123]))
            .device_writes(Arc::new(|context| {
                context.source_network.is_none()
                    && context.trust == bacnet_server::mutation::MutationTrust::Unverified
            }))
            .build_session()
            .unwrap();
    session.start().await.unwrap();
    let mut client = bacnet_client::client::BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, "B/IP update").unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        client.write_property(
            &mac,
            oid(123),
            PropertyIdentifier::DESCRIPTION,
            None,
            value.to_vec(),
            Some(8),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        read(&session, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("B/IP update".into())
    );
    client
        .write_property(
            &mac,
            oid(123),
            PropertyIdentifier::DESCRIPTION,
            None,
            vec![0],
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&session, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("B/IP update".into())
    );
    let ack = client
        .read_property(
            &mac,
            oid(123),
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap();
    let (profile, end) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(end, ack.property_value.len());
    assert_eq!(services(profile), vec![12, 15]);
    client.stop().await.unwrap();
    session.stop().await.unwrap();
}
