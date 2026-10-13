use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

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
            .identity(identity)
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
            matches!(result, Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32 && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32),
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
            let mut session = session.with_database(database(&[123])).with_identity(
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
    let mut session = session.with_identity(
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
