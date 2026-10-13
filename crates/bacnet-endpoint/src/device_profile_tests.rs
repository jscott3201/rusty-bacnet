//! Device Tags metadata does not expand the endpoint's narrow write admission.
use super::*;
use bacnet_objects::object_profile::{ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::{ErrorClass, ErrorCode, RejectReason};
use std::net::Ipv4Addr;

const TAGS: PropertyIdentifier = PropertyIdentifier::TAGS;
const LOCATION: PropertyIdentifier = PropertyIdentifier::PROFILE_LOCATION;
const NAME: PropertyIdentifier = PropertyIdentifier::PROFILE_NAME;
const EXHAUST: &[u8] = &[0x0d, 8, 0, b'e', b'x', b'h', b'a', b'u', b's', b't'];
const FLOOR: &[u8] = &[0x0d, 6, 0, b'f', b'l', b'o', b'o', b'r', 0x21, 3];
#[derive(Default)]
struct Store(AtomicUsize);
impl TagsPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(None)
    }
    fn save(&self, _: ObjectIdentifier, _: &TagsSnapshot) -> Result<(), Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn text(value: &str) -> Vec<u8> {
    let mut bytes = vec![0x75, value.len() as u8 + 1, 0];
    bytes.extend(value.as_bytes());
    bytes
}
fn primitive(bytes: &[u8]) -> PropertyValue {
    let (value, end) = bacnet_encoding::primitives::decode_application_value(bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    value
}
fn assert_reject<T: std::fmt::Debug>(result: Result<T, Error>) {
    assert!(
        matches!(result, Err(Error::Reject { reason }) if reason == RejectReason::UNRECOGNIZED_SERVICE.to_raw()),
        "{result:?}"
    );
}
fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32),
        "{result:?}"
    );
}
fn profiled_database(
    identity: &crate::DeviceIdentity,
    tags: bool,
    store: Arc<Store>,
) -> ObjectDatabase {
    let mut db = crate::identity::build_database_with_extra(identity, vec![]).unwrap();
    let selected = oid(identity.instance());
    let original = db.get(&selected).unwrap();
    let name = original.object_name().to_owned();
    let identity_rows = [
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::VENDOR_IDENTIFIER,
        PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
        PropertyIdentifier::SEGMENTATION_SUPPORTED,
        PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
        PropertyIdentifier::DEVICE_UUID,
        PropertyIdentifier::OBJECT_LIST,
    ];
    let before = identity_rows.map(|p| original.read_property(p, None).unwrap());
    let mut object = DeviceObject::with_tags_persistence(
        DeviceConfig {
            instance: identity.instance(),
            name,
            vendor_id: identity.vendor_id(),
            max_apdu_length: identity.max_apdu_length().into(),
            segmentation_supported: identity.segmentation(),
            ..Default::default()
        },
        store,
    )
    .unwrap();
    object.set_services_supported(identity.services());
    object.set_device_uuid(identity.device_uuid());
    object.set_object_list(vec![selected]);
    object
        .set_profile(ObjectProfile {
            tags: tags.then(|| vec![BACnetNameValue::semantic("exhaust")]),
            profile_location: Some("https://example.com/p.xdd".into()),
            profile_name: Some("555-device".into()),
        })
        .unwrap();
    assert_eq!(
        identity_rows.map(|p| object.read_property(p, None).unwrap()),
        before
    );
    // Replace the one identity-owned Device; no second Device-shaped stand-in.
    db.add(Box::new(object)).unwrap();
    assert_eq!(db.selected_device(), Some(selected));
    db
}
#[tokio::test]
async fn device_profiles_read_but_tags_never_enter_endpoint_mutation_authority() {
    for writes in [false, true] {
        for provisioned in [false, true] {
            let identity = crate::DeviceIdentity::new(123, 42).unwrap();
            let store = Arc::new(Store::default());
            let db = profiled_database(&identity, provisioned, store.clone());
            let calls = Arc::new(AtomicUsize::new(0));
            let mut endpoint =
                crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                    .role(SessionRole::ServerOnly)
                    .database(db)
                    .identity(identity)
                    .build_session()
                    .unwrap();
            if writes {
                let calls = calls.clone();
                endpoint = endpoint.with_device_writes(Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    true
                }));
            }
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
            let expected_services = if writes { vec![12, 15] } else { vec![12] };
            for target in [oid(123), oid(ObjectIdentifier::MAX_INSTANCE)] {
                for (p, bytes) in [
                    (LOCATION, text("https://example.com/p.xdd")),
                    (NAME, text("555-device")),
                ] {
                    assert_eq!(
                        client
                            .read_property(&mac, target, p, None)
                            .await
                            .unwrap()
                            .property_value,
                        bytes
                    );
                }
                let result = client.read_property(&mac, target, TAGS, None).await;
                if provisioned {
                    assert_eq!(result.unwrap().property_value, EXHAUST);
                    assert_eq!(
                        client
                            .read_property(&mac, target, TAGS, Some(0))
                            .await
                            .unwrap()
                            .property_value,
                        [0x21, 1]
                    );
                    assert_eq!(
                        client
                            .read_property(&mac, target, TAGS, Some(1))
                            .await
                            .unwrap()
                            .property_value,
                        EXHAUST
                    );
                } else {
                    assert_error(result, ErrorCode::UNKNOWN_PROPERTY);
                }
                let read = client
                    .read_property(
                        &mac,
                        target,
                        PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
                        None,
                    )
                    .await
                    .unwrap();
                assert_eq!(services(primitive(&read.property_value)), expected_services);
                let list = client
                    .read_property(&mac, target, PropertyIdentifier::PROPERTY_LIST, None)
                    .await
                    .unwrap()
                    .property_value;
                let mut offset = 0;
                let mut properties = Vec::new();
                while offset < list.len() {
                    let (PropertyValue::Enumerated(p), next) =
                        bacnet_encoding::primitives::decode_application_value(&list, offset)
                            .unwrap()
                    else {
                        panic!("property identifier");
                    };
                    properties.push(p);
                    offset = next;
                }
                assert!(properties.windows(2).all(|p| p[0] < p[1]));
                assert_eq!(properties.contains(&TAGS.to_raw()), provisioned);
                assert!(
                    properties.contains(&LOCATION.to_raw()) && properties.contains(&NAME.to_raw())
                );
                assert!(!properties.contains(&PropertyIdentifier::PROPERTY_LIST.to_raw()));
                let count = client
                    .read_property(&mac, target, PropertyIdentifier::PROPERTY_LIST, Some(0))
                    .await
                    .unwrap();
                assert_eq!(
                    primitive(&count.property_value),
                    PropertyValue::Unsigned(properties.len() as u64)
                );
                for (index, p) in properties.iter().enumerate() {
                    let item = client
                        .read_property(
                            &mac,
                            target,
                            PropertyIdentifier::PROPERTY_LIST,
                            Some(index as u32 + 1),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        primitive(&item.property_value),
                        PropertyValue::Enumerated(*p)
                    );
                }
                for p in [
                    PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
                    PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
                ] {
                    assert!(!properties.contains(&p.to_raw()));
                    assert_error(
                        client.read_property(&mac, target, p, None).await,
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                }
                assert!(client
                    .read_property(
                        &mac,
                        target,
                        PropertyIdentifier::DEVICE_ADDRESS_BINDING,
                        None
                    )
                    .await
                    .unwrap()
                    .property_value
                    .is_empty());
            }
            let result = client
                .write_property(&mac, oid(123), TAGS, None, FLOOR.to_vec(), None)
                .await;
            if writes {
                assert_error(
                    result,
                    if provisioned {
                        ErrorCode::WRITE_ACCESS_DENIED
                    } else {
                        ErrorCode::UNKNOWN_PROPERTY
                    },
                );
            } else {
                assert_reject(result);
            }
            assert_reject(
                client
                    .write_property_multiple(
                        &mac,
                        vec![WriteAccessSpecification {
                            object_identifier: oid(123),
                            list_of_properties: vec![BACnetPropertyValue {
                                property_identifier: TAGS,
                                property_array_index: None,
                                value: FLOOR.to_vec(),
                                priority: None,
                            }],
                        }],
                    )
                    .await,
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert_eq!(store.0.load(Ordering::SeqCst), 0);
            if provisioned {
                assert_eq!(
                    client
                        .read_property(&mac, oid(123), TAGS, None)
                        .await
                        .unwrap()
                        .property_value,
                    EXHAUST
                );
            }
            if writes {
                client
                    .write_property(
                        &mac,
                        oid(123),
                        PropertyIdentifier::DESCRIPTION,
                        None,
                        text("authorized"),
                        None,
                    )
                    .await
                    .unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert_eq!(
                    read(&endpoint, PropertyIdentifier::DESCRIPTION).await,
                    PropertyValue::CharacterString("authorized".into())
                );
            }
            let served = client
                .read_property(
                    &mac,
                    oid(123),
                    PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(
                services(primitive(&served.property_value)),
                expected_services
            );
            assert_eq!(store.0.load(Ordering::SeqCst), 0);
            client.stop().await.unwrap();
            endpoint.stop().await.unwrap();
        }
    }
}
