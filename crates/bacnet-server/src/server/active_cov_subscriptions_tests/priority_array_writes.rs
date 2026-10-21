//! Real dispatcher evidence for the read-only Priority_Array contract (§19.2.1).
use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::EngineeringUnits;

#[tokio::test]
async fn priority_array_wire_wp_must_not_ack_direct_slot_write() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let object =
        AnalogOutputObject::new(842, "read-only-array", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = object.object_identifier();
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value, 42.0);
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    let response = wire
        .send(&direct(), (ConfirmedServiceChoice::WRITE_PROPERTY, request))
        .await;
    wire.server.stop().await.unwrap();
    error(
        response,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        wire.server.read_local(&oid, PV, None).await.unwrap(),
        PropertyValue::Real(0.0)
    );
}

const PA: PropertyIdentifier = PropertyIdentifier::PRIORITY_ARRAY;

fn objects() -> Vec<Box<dyn BACnetObject>> {
    use bacnet_objects::{analog::*, binary::*, lighting::*, multistate::*, value_types::*};
    use bacnet_types::enums::EngineeringUnits;
    vec![
        Box::new(AnalogOutputObject::new(842, "AO", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(AnalogValueObject::new(842, "AV", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(BinaryOutputObject::new(842, "BO").unwrap()),
        Box::new(BinaryValueObject::new(842, "BV").unwrap()),
        Box::new(MultiStateOutputObject::new(842, "MSO", 3).unwrap()),
        Box::new(MultiStateValueObject::new(842, "MSV", 3).unwrap()),
        Box::new(LightingOutputObject::new(842, "LO").unwrap()),
        Box::new(BinaryLightingOutputObject::new(842, "BLO").unwrap()),
        Box::new(IntegerValueObject::new(842, "IV").unwrap()),
        Box::new(PositiveIntegerValueObject::new(842, "PIV").unwrap()),
        Box::new(LargeAnalogValueObject::new(842, "LAV").unwrap()),
        Box::new(CharacterStringValueObject::new(842, "CSV").unwrap()),
        Box::new(OctetStringValueObject::new(842, "OSV").unwrap()),
        Box::new(BitStringValueObject::new(842, "BSV").unwrap()),
        Box::new(DateValueObject::new(842, "DV").unwrap()),
        Box::new(TimeValueObject::new(842, "TV").unwrap()),
        Box::new(DateTimeValueObject::new(842, "DTV").unwrap()),
        Box::new(DatePatternValueObject::new(842, "DPV").unwrap()),
        Box::new(TimePatternValueObject::new(842, "TPV").unwrap()),
        Box::new(DateTimePatternValueObject::new(842, "DTPV").unwrap()),
    ]
}

fn denied(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY
        && code == ErrorCode::WRITE_ACCESS_DENIED),
        "{result:?}"
    );
}

#[test]
fn priority_array_all_twenty_profiles_trait_metadata_and_pics_are_read_only() {
    use bacnet_objects::property_metadata::PropertyWriteCapability;
    let cases = objects();
    assert_eq!(cases.len(), 20);
    let mut database = ObjectDatabase::new();
    for mut object in cases {
        let oid = object.object_identifier();
        assert!(object.is_array_property(PA));
        assert!(!object.is_writable_property(PA), "{oid:?}");
        let rows = object.property_metadata().into_owned();
        let row = rows
            .iter()
            .find(|row| row.property_identifier == PA)
            .unwrap();
        assert_eq!(
            row.write_capability,
            PropertyWriteCapability::ReadOnly,
            "{oid:?}"
        );
        let default = object.read_property(PV, None).unwrap();
        object
            .write_property_from(
                PV,
                None,
                default.clone(),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        assert_eq!(object.read_property(PA, Some(8)).unwrap(), default);
        assert_eq!(
            object.read_property(PA, Some(0)).unwrap(),
            PropertyValue::Unsigned(16)
        );
        let before = object.read_property(PA, None).unwrap();
        for index in [None, Some(0), Some(1), Some(8), Some(16), Some(17)] {
            for value in [default.clone(), PropertyValue::Null] {
                denied(object.write_property(PA, index, value, Some(8)));
                assert_eq!(object.read_property(PA, None).unwrap(), before, "{oid:?}");
                assert_eq!(object.read_property(PV, None).unwrap(), default, "{oid:?}");
            }
        }
        object
            .write_property_from(
                PV,
                None,
                PropertyValue::Null,
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        assert_eq!(
            object.read_property(PA, Some(8)).unwrap(),
            PropertyValue::Null
        );
        assert_eq!(object.read_property(PV, None).unwrap(), default);
        database.add(object).unwrap();
    }
    let pics = crate::pics::generate_pics(
        &database,
        &ServerConfig::default(),
        &crate::pics::PicsConfig::default(),
    );
    assert_eq!(pics.supported_object_types.len(), 20);
    for support in pics.supported_object_types {
        let row = support
            .supported_properties
            .iter()
            .find(|row| row.property_id == PA)
            .unwrap();
        assert!(row.access.readable);
        assert!(!row.access.writable, "{:?}", support.object_type);
    }
    let mut door =
        bacnet_objects::access_control::AccessDoorObject::new(842, "door control").unwrap();
    assert!(!door.is_writable_property(PA));
    denied(door.write_property(PA, Some(8), PropertyValue::Null, None));
}

fn wp_request(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
) -> (ConfirmedServiceChoice, BytesMut) {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, &value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: bytes.to_vec(),
        priority,
    }
    .encode(&mut request)
    .unwrap();
    (ConfirmedServiceChoice::WRITE_PROPERTY, request)
}

#[tokio::test]
async fn priority_array_wire_and_local_denial_preserves_all_three_writer_states() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    let representatives = objects().into_iter().filter(|o| {
        matches!(
            o.object_identifier().object_type(),
            ObjectType::ANALOG_OUTPUT
                | ObjectType::BINARY_LIGHTING_OUTPUT
                | ObjectType::INTEGER_VALUE
        )
    });
    for object in representatives {
        let oid = object.object_identifier();
        let value = match oid.object_type() {
            ObjectType::ANALOG_OUTPUT => PropertyValue::Real(42.0),
            ObjectType::BINARY_LIGHTING_OUTPUT => PropertyValue::Enumerated(1),
            _ => PropertyValue::Signed(42),
        };
        wire.server.database().write().await.add(object).unwrap();
        simple_ack(
            wire.send(&direct(), wp_request(oid, PV, None, value.clone(), Some(8)))
                .await,
        );
        let before = wire.server.read_local(&oid, PA, None).await.unwrap();
        for index in [None, Some(0), Some(1), Some(8), Some(16), Some(17)] {
            for attempted in [value.clone(), PropertyValue::Null] {
                error(
                    wire.send(
                        &direct(),
                        wp_request(oid, PA, index, attempted.clone(), None),
                    )
                    .await,
                    ErrorClass::PROPERTY,
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                denied(
                    wire.server
                        .write_local(
                            &oid,
                            PA,
                            index,
                            attempted,
                            None,
                            crate::LocalCommandSource::ServerDevice,
                        )
                        .await,
                );
                assert_eq!(
                    wire.server.read_local(&oid, PA, None).await.unwrap(),
                    before
                );
                assert_eq!(wire.server.read_local(&oid, PV, None).await.unwrap(), value);
            }
        }
        simple_ack(
            wire.send(
                &direct(),
                wp_request(oid, PV, None, PropertyValue::Null, Some(8)),
            )
            .await,
        );
        assert_eq!(
            wire.server.read_local(&oid, PA, Some(8)).await.unwrap(),
            PropertyValue::Null
        );
    }
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn priority_array_wpm_denial_keeps_prefix_and_reports_failed_coordinate() {
    use bacnet_services::{
        common::BACnetPropertyValue,
        wpm::{WriteAccessSpecification, WritePropertyMultipleError, WritePropertyMultipleRequest},
    };
    let mut wire = Wire::start(ServerConfig::default()).await;
    for object in objects().into_iter().filter(|o| {
        matches!(
            o.object_identifier().object_type(),
            ObjectType::ANALOG_OUTPUT
                | ObjectType::BINARY_LIGHTING_OUTPUT
                | ObjectType::INTEGER_VALUE
        )
    }) {
        let oid = object.object_identifier();
        let (first, suffix) = match oid.object_type() {
            ObjectType::ANALOG_OUTPUT => (PropertyValue::Real(42.0), PropertyValue::Real(99.0)),
            ObjectType::BINARY_LIGHTING_OUTPUT => {
                (PropertyValue::Enumerated(1), PropertyValue::Enumerated(0))
            }
            _ => (PropertyValue::Signed(42), PropertyValue::Signed(99)),
        };
        wire.server.database().write().await.add(object).unwrap();
        for index in [None, Some(0), Some(1), Some(8), Some(16), Some(17)] {
            for denied_value in [first.clone(), PropertyValue::Null] {
                let mut bytes = BytesMut::new();
                WritePropertyMultipleRequest {
                    list_of_write_access_specs: vec![WriteAccessSpecification {
                        object_identifier: oid,
                        list_of_properties: [
                            (PV, None, first.clone(), Some(8)),
                            (PA, index, denied_value, None),
                            (PV, None, suffix.clone(), Some(8)),
                        ]
                        .into_iter()
                        .map(
                            |(property_identifier, property_array_index, value, priority)| {
                                let mut encoded = BytesMut::new();
                                bacnet_encoding::primitives::encode_property_value(
                                    &mut encoded,
                                    &value,
                                )
                                .unwrap();
                                BACnetPropertyValue {
                                    property_identifier,
                                    property_array_index,
                                    value: encoded.to_vec(),
                                    priority,
                                }
                            },
                        )
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
                    panic!("expected WPM error: {response:?}");
                };
                let failure = WritePropertyMultipleError::from_error_pdu(&pdu).unwrap();
                assert_eq!(
                    (failure.error_class, failure.error_code),
                    (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
                );
                let mut coordinate = BACnetObjectPropertyReference::new(oid, PA.to_raw());
                coordinate.property_array_index = index;
                assert_eq!(failure.first_failed_write_attempt, coordinate);
                assert_eq!(wire.server.read_local(&oid, PV, None).await.unwrap(), first);
                assert_eq!(
                    wire.server.read_local(&oid, PA, Some(8)).await.unwrap(),
                    first
                );
                wire.server
                    .write_local(
                        &oid,
                        PV,
                        None,
                        PropertyValue::Null,
                        Some(8),
                        crate::LocalCommandSource::ServerDevice,
                    )
                    .await
                    .unwrap();
            }
        }
    }
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn priority_array_denial_preserves_existing_wire_preflight() {
    use crate::mutation::MutationPolicy;
    use bacnet_types::enums::EngineeringUnits;
    let mut wire = Wire::start(ServerConfig::default()).await;
    let object =
        AnalogOutputObject::new(842, "preflight", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let oid = object.object_identifier();
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 843).unwrap();
    error(
        wire.send(
            &direct(),
            wp_request(missing, PA, Some(8), PropertyValue::Null, None),
        )
        .await,
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    );
    error(
        wire.send(
            &direct(),
            wp_request(oid, PV, Some(8), PropertyValue::Null, None),
        )
        .await,
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    // Well-framed unknown application tag: the decoder still rejects it before dispatch.
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PA,
        property_array_index: Some(8),
        property_value: vec![0xd1, 0],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    error(
        wire.send(&direct(), (ConfirmedServiceChoice::WRITE_PROPERTY, request))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_eq!(
        wire.server.read_local(&oid, PA, Some(8)).await.unwrap(),
        PropertyValue::Null
    );
    wire.server.stop().await.unwrap();
    let mut wire = Wire::start(ServerConfig {
        mutation_policy: MutationPolicy::DenyAll,
        ..ServerConfig::default()
    })
    .await;
    error(
        wire.send(
            &direct(),
            wp_request(av(1), PA, Some(8), PropertyValue::Null, None),
        )
        .await,
        ErrorClass::SERVICES,
        ErrorCode::SERVICE_REQUEST_DENIED,
    );
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn priority_array_custom_object_write_policy_is_not_globally_restricted() {
    struct CustomArray(PropertyValue);
    impl BACnetObject for CustomArray {
        fn object_identifier(&self) -> ObjectIdentifier {
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 842).unwrap()
        }
        fn object_name(&self) -> &str {
            "custom-array"
        }
        fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
            std::borrow::Cow::Owned(vec![PA])
        }
        fn read_property(
            &self,
            _: PropertyIdentifier,
            _: Option<u32>,
        ) -> Result<PropertyValue, Error> {
            Ok(self.0.clone())
        }
        fn write_property(
            &mut self,
            property: PropertyIdentifier,
            _: Option<u32>,
            value: PropertyValue,
            _: Option<u8>,
        ) -> Result<(), Error> {
            assert_eq!(property, PA);
            self.0 = value;
            Ok(())
        }
    }
    let mut wire = Wire::start(ServerConfig::default()).await;
    let object = CustomArray(PropertyValue::Null);
    let oid = object.object_identifier();
    wire.server
        .database()
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
    simple_ack(
        wire.send(
            &direct(),
            wp_request(oid, PA, Some(8), PropertyValue::Unsigned(42), None),
        )
        .await,
    );
    assert_eq!(
        wire.server.read_local(&oid, PA, Some(8)).await.unwrap(),
        PropertyValue::Unsigned(42)
    );
    wire.server.stop().await.unwrap();
}
