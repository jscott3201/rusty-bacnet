use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

#[tokio::test]
async fn endpoint_rp_only_wire_omits_nonexecuted_cov_properties() {
    wire_contract(false, false).await;
}

#[tokio::test]
async fn endpoint_execution_view_covers_write_opt_in_and_custom_device_reader() {
    wire_contract(true, false).await;
    wire_contract(false, true).await;
}

async fn wire_contract(writes: bool, custom: bool) {
    use std::net::Ipv4Addr;
    let identity = crate::DeviceIdentity::new(123, 42).unwrap();
    let mut db = crate::identity::build_database_with_extra(&identity, vec![]).unwrap();
    if custom {
        db.add(Box::new(CustomDevice)).unwrap();
    }
    let mut endpoint =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .device(identity)
            .build_session()
            .unwrap();
    if writes {
        endpoint = endpoint.with_device_writes(Arc::new(|_| true));
    }
    endpoint.start().await.unwrap();
    // The endpoint bound port 0; the client talks to the port it actually got.
    let port = endpoint.bip_local_address().unwrap().port();
    let mut client = bacnet_client::client::BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);
    let profile = client
        .read_property(
            &mac,
            oid(123),
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap();
    let (profile, _) =
        bacnet_encoding::primitives::decode_application_value(&profile.property_value, 0).unwrap();
    assert_eq!(
        services(profile),
        if writes { vec![12, 15] } else { vec![12] }
    );
    let list = client
        .read_property(&mac, oid(123), PropertyIdentifier::PROPERTY_LIST, None)
        .await
        .unwrap();
    let mut properties = Vec::new();
    let mut pos = 0;
    while pos < list.property_value.len() {
        let (value, next) =
            bacnet_encoding::primitives::decode_application_value(&list.property_value, pos)
                .unwrap();
        let PropertyValue::Enumerated(value) = value else {
            panic!("property identifier");
        };
        properties.push(value);
        pos = next;
    }
    let mut reads = Vec::new();
    for property in [
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
    ] {
        reads.push((
            property,
            client.read_property(&mac, oid(123), property, None).await,
        ));
    }
    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
    for (property, result) in reads {
        assert!(
            !properties.contains(&property.to_raw()),
            "RP-only endpoint must omit {property:?} from Property_List"
        );
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY && code == ErrorCode::UNKNOWN_PROPERTY),
            "nonexecuted COV property must be unknown"
        );
    }
}

struct CustomDevice;
impl BACnetObject for CustomDevice {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(123)
    }
    fn object_name(&self) -> &str {
        "custom-device"
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Owned(vec![
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
            PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
        ])
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => {
                return Ok(PropertyValue::ObjectIdentifier(oid(123)));
            }
            PropertyIdentifier::VENDOR_IDENTIFIER => return Ok(PropertyValue::Unsigned(42)),
            PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED => {
                return Ok(PropertyValue::Unsigned(1476));
            }
            _ => {}
        }
        // Startup validates the ordinary Device declaration. Execution-owned
        // service/COV properties below must still bypass the custom reader.
        if property == PropertyIdentifier::SEGMENTATION_SUPPORTED {
            return Ok(PropertyValue::Enumerated(
                bacnet_types::enums::Segmentation::NONE.to_raw().into(),
            ));
        }
        panic!("execution-owned reads must not enter custom reader")
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("read-only fixture")
    }
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        property != PropertyIdentifier::PROPERTY_LIST
    }
}

#[tokio::test]
async fn endpoint_execution_profile_refusals_precede_ingress_and_allow_repair() {
    for role in [SessionRole::Both, SessionRole::ServerOnly] {
        for declared in [
            vec![],
            vec![ServiceSupported::SUBSCRIBE_COV],
            vec![
                ServiceSupported::READ_PROPERTY,
                ServiceSupported::WRITE_PROPERTY,
            ],
            vec![
                ServiceSupported::READ_PROPERTY,
                ServiceSupported::WRITE_GROUP,
            ],
        ] {
            let (session, _peer, starts) = session(role);
            let mut session = session.with_database(database(&[123])).device(
                crate::DeviceIdentity::new(123, 42)
                    .unwrap()
                    .with_services(&declared),
            );
            assert!(session.start().await.is_err());
            assert_eq!(starts.load(Ordering::SeqCst), 0);
            assert_eq!(
                session.lifecycle.load(Ordering::Acquire),
                Lifecycle::Ready as u8
            );
            session.identity = Some(crate::DeviceIdentity::new(123, 42).unwrap());
            session.start().await.unwrap();
            session.stop().await.unwrap();
            assert_eq!(starts.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn endpoint_client_only_keeps_local_declaration_without_responder_claim() {
    let (session, _peer, starts) = session(SessionRole::ClientOnly);
    let declared = [ServiceSupported::SUBSCRIBE_COV];
    let mut session = session.device(
        crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .with_services(&declared),
    );
    session.start().await.unwrap();
    assert!(session.server().is_none());
    assert_eq!(session.identity().unwrap().services(), declared);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    session.stop().await.unwrap();
}

#[tokio::test]
async fn generated_identity_preflight_failure_can_change_identity_and_retry() {
    let (session, _peer, starts) = session(SessionRole::Both);
    let mut session = session.device(
        crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .with_max_apdu(1474)
            .unwrap()
            .with_services(&[]),
    );
    assert!(session.start().await.is_err());
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert!(
        session.database.is_none(),
        "failed stock generation is not committed"
    );
    assert_eq!(session.identity().unwrap().max_apdu_length(), 1474);
    session = session.device(
        crate::DeviceIdentity::new(456, 99)
            .unwrap()
            .with_max_apdu(206)
            .unwrap(),
    );
    session.start().await.unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .selected_device(),
        Some(oid(456))
    );
    session.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_wildcard_keeps_frozen_identity_and_identity_free_selection_stays_global() {
    use std::net::Ipv4Addr;
    for composed in [true, false] {
        let identity = crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .with_max_apdu(1474)
            .unwrap();
        let mut builder =
            crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
                .role(SessionRole::ServerOnly)
                .database(identity.build_database().unwrap());
        if composed {
            builder = builder.device(identity.clone());
        }
        let mut endpoint = builder.build_session().unwrap();
        endpoint.start().await.unwrap();
        let mut client = bacnet_client::client::BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .build()
            .await
            .unwrap();
        let mac = bacnet_transport::bvll::encode_bip_mac(
            [127, 0, 0, 1],
            endpoint.bip_local_address().unwrap().port(),
        );
        let database = endpoint.database.as_ref().unwrap();
        database
            .write()
            .await
            .add(Box::new(
                DeviceObject::new(DeviceConfig {
                    identity: crate::DeviceIdentity::new(1, 17).unwrap(),
                    ..Default::default()
                })
                .unwrap(),
            ))
            .unwrap();
        assert_eq!(database.read().await.selected_device(), Some(oid(1)));
        let wildcard = oid(ObjectIdentifier::MAX_INSTANCE);
        for phase in 0..3 {
            if phase == 1 {
                database.write().await.remove(&oid(123)).unwrap();
            } else if phase == 2 {
                database
                    .write()
                    .await
                    .add(Box::new(
                        DeviceObject::new(DeviceConfig {
                            identity: crate::DeviceIdentity::new(123, 99)
                                .unwrap()
                                .with_max_apdu(480)
                                .unwrap(),
                            ..Default::default()
                        })
                        .unwrap(),
                    ))
                    .unwrap();
            }
            for (property, expected) in [
                (
                    PropertyIdentifier::OBJECT_IDENTIFIER,
                    PropertyValue::ObjectIdentifier(oid(if composed { 123 } else { 1 })),
                ),
                (
                    PropertyIdentifier::VENDOR_IDENTIFIER,
                    PropertyValue::Unsigned(if composed { 42 } else { 17 }),
                ),
                (
                    PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
                    PropertyValue::Unsigned(if composed { 1474 } else { 1476 }),
                ),
            ] {
                let result = client.read_property(&mac, wildcard, property, None).await;
                if composed && phase == 1 {
                    assert!(
                        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
                        if class == ErrorClass::OBJECT && code == ErrorCode::UNKNOWN_OBJECT),
                        "{result:?}"
                    );
                } else {
                    let ack = result.unwrap();
                    assert_eq!(ack.object_identifier, oid(if composed { 123 } else { 1 }));
                    assert_eq!(
                        bacnet_encoding::primitives::decode_application_value(
                            &ack.property_value,
                            0
                        )
                        .unwrap()
                        .0,
                        expected
                    );
                }
            }
        }
        client.stop().await.unwrap();
        endpoint.stop().await.unwrap();
    }
}
