use super::*;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::error::RemoteError;
use std::sync::Mutex;

struct WritableObject {
    oid: ObjectIdentifier,
    calls: Arc<Mutex<Vec<PropertyIdentifier>>>,
    description: String,
}
impl BACnetObject for WritableObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        if self.oid.object_type() == ObjectType::DEVICE {
            "writable-device"
        } else {
            "writable-other"
        }
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![
            SERVICES,
            ACTIVE,
            MULTIPLE,
            LIST,
            PropertyIdentifier::DESCRIPTION,
        ])
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(if property == PropertyIdentifier::DESCRIPTION {
            PropertyValue::CharacterString(self.description.clone())
        } else {
            PropertyValue::Null
        })
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        self.calls.lock().unwrap().push(property);
        if let (PropertyIdentifier::DESCRIPTION, PropertyValue::CharacterString(text)) =
            (property, value)
        {
            self.description = text;
        }
        Ok(())
    }
}

fn wp(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
) -> (ConfirmedServiceChoice, BytesMut) {
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: value,
        priority: None,
    }
    .encode(&mut bytes)
    .unwrap();
    (ConfirmedServiceChoice::WRITE_PROPERTY, bytes)
}

#[tokio::test]
async fn device_execution_owned_wp_rejects_custom_writer() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(WritableObject {
            oid: device(),
            calls: calls.clone(),
            description: "custom".into(),
        }))
        .unwrap();
    let bindings = PropertyIdentifier::DEVICE_ADDRESS_BINDING;
    for property in [SERVICES, ACTIVE, MULTIPLE, LIST, bindings] {
        let before = wire
            .server
            .read_local(&device(), property, None)
            .await
            .unwrap();
        error(
            wire.send(&direct(), wp(device(), property, None, vec![0]))
                .await,
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(
            wire.server
                .read_local(&device(), property, None)
                .await
                .unwrap(),
            before
        );
    }
    // Existing preflight errors still win over the owned-field write guard.
    for (oid, index, bytes, class, code) in [
        (
            ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap(),
            Some(0),
            vec![],
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            device(),
            Some(0),
            vec![],
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            device(),
            None,
            vec![],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
        ),
    ] {
        error(
            wire.send(&direct(), wp(oid, SERVICES, index, bytes)).await,
            class,
            code,
        );
    }
    let local = wire
        .server
        .write_local(
            &device(),
            SERVICES,
            None,
            PropertyValue::Null,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await;
    assert!(
        matches!(local, Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY && code == ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert!(calls.lock().unwrap().is_empty());
    simple_ack(
        wire.send(
            &direct(),
            wp(
                device(),
                PropertyIdentifier::DESCRIPTION,
                None,
                text("wire"),
            ),
        )
        .await,
    );
    assert_eq!(
        wire.server
            .read_local(&device(), PropertyIdentifier::DESCRIPTION, None)
            .await
            .unwrap(),
        PropertyValue::CharacterString("wire".into())
    );
    wire.server
        .write_local(
            &device(),
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("local".into()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(
        wire.server
            .read_local(&device(), PropertyIdentifier::DESCRIPTION, None)
            .await
            .unwrap(),
        PropertyValue::CharacterString("local".into())
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![PropertyIdentifier::DESCRIPTION; 2]
    );
    // The reservation is specific to Device OIDs, not a global property filter.
    let other_calls = Arc::new(Mutex::new(Vec::new()));
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(WritableObject {
            oid: av(1),
            calls: other_calls.clone(),
            description: "other".into(),
        }))
        .unwrap();
    simple_ack(
        wire.send(&direct(), wp(av(1), SERVICES, None, vec![0]))
            .await,
    );
    assert_eq!(*other_calls.lock().unwrap(), vec![SERVICES]);
    wire.server.stop().await.unwrap();
}

fn text(value: &str) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut bytes, value).unwrap();
    bytes.to_vec()
}

#[tokio::test]
async fn device_execution_owned_wpm_preserves_successful_prefix_and_failure_coordinate() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::{
        WriteAccessSpecification, WritePropertyMultipleError, WritePropertyMultipleRequest,
    };
    let mut wire = Wire::start(ServerConfig::default()).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(WritableObject {
            oid: device(),
            calls: calls.clone(),
            description: "initial".into(),
        }))
        .unwrap();
    for property in [SERVICES, ACTIVE, MULTIPLE, LIST] {
        for with_prefix in [false, true] {
            calls.lock().unwrap().clear();
            let mut properties = Vec::new();
            if with_prefix {
                properties.push((PropertyIdentifier::DESCRIPTION, text("prefix")));
            }
            properties.push((property, vec![0]));
            properties.push((PropertyIdentifier::DESCRIPTION, text("suffix")));
            let mut bytes = BytesMut::new();
            WritePropertyMultipleRequest {
                list_of_write_access_specs: vec![WriteAccessSpecification {
                    object_identifier: device(),
                    list_of_properties: properties
                        .into_iter()
                        .map(|(property_identifier, value)| BACnetPropertyValue {
                            property_identifier,
                            value,
                            property_array_index: None,
                            priority: None,
                        })
                        .collect(),
                }],
            }
            .encode(&mut bytes)
            .unwrap();
            let response = wire
                .send(
                    &direct(),
                    (ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, bytes),
                )
                .await;
            let Apdu::Error(pdu) = response else {
                panic!("expected WPM Error, got {response:?}");
            };
            let failure = WritePropertyMultipleError::from_error_pdu(&pdu).unwrap();
            assert_eq!(
                (failure.error_class, failure.error_code),
                (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
            );
            assert_eq!(
                failure.first_failed_write_attempt,
                BACnetObjectPropertyReference::new(device(), property.to_raw())
            );
            assert_eq!(
                *calls.lock().unwrap(),
                if with_prefix {
                    vec![PropertyIdentifier::DESCRIPTION]
                } else {
                    vec![]
                }
            );
            if with_prefix {
                assert_eq!(
                    wire.server
                        .read_local(&device(), PropertyIdentifier::DESCRIPTION, None)
                        .await
                        .unwrap(),
                    PropertyValue::CharacterString("prefix".into())
                );
            }
        }
    }
    wire.server.stop().await.unwrap();
}
