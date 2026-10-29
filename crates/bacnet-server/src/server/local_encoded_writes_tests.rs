//! `write_local_encoded` takes the octets a network WriteProperty carries
//! and decodes them as the WriteProperty handler does, so a value read with
//! `read_local` and encoded writes back (#1296 review).

use super::*;
use crate::server::clock::clocked_test_database;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::{BinaryOutputObject, BinaryValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

async fn server() -> BACnetServer<TestTransport> {
    let mut db = clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(1296, 0)
                .expect("valid Device identity")
                .with_name("Encoded writes"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db.add(Box::new(
        ScheduleObject::new(1, "SCH-1", PropertyValue::Null).unwrap(),
    ))
    .unwrap();
    let stage = |limit, values: Vec<bool>| BACnetStageLimitValue {
        limit,
        values,
        deadband: 1.0,
    };
    let target = |object_type| BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: oid(object_type, 1),
    };
    db.add(Box::new(
        StagingObject::new(
            1,
            "STG-1",
            StagingConfig {
                present_value: 5.0,
                min_present_value: 0.0,
                units: EngineeringUnits::DEGREES_CELSIUS,
                priority_for_writing: 8,
                stages: vec![
                    stage(10.0, vec![false, true]),
                    stage(20.0, vec![true, true]),
                ],
                target_references: vec![
                    target(ObjectType::BINARY_OUTPUT),
                    target(ObjectType::BINARY_VALUE),
                ],
                stage_names: None,
            },
        )
        .unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(1296, 0).unwrap())
        .transport(TestTransport::new())
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap()
}

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut encoded = bytes::BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    encoded.to_vec()
}

#[tokio::test]
async fn a_local_read_writes_back_through_the_network_decoding() {
    let server = server().await;
    let schedule = oid(ObjectType::SCHEDULE, 1);
    let staging = oid(ObjectType::STAGING, 1);
    for (object, property, index) in [
        // A date range, two application dates on the wire.
        (schedule, PropertyIdentifier::EFFECTIVE_PERIOD, None),
        // Each stage is a REAL, a BIT STRING and a REAL.
        (staging, PropertyIdentifier::STAGES, None),
        (staging, PropertyIdentifier::STAGES, Some(2)),
        (staging, PropertyIdentifier::TARGET_REFERENCES, None),
    ] {
        let value = server.read_local(&object, property, index).await.unwrap();
        server
            .write_local_encoded(
                &object,
                property,
                index,
                &encode(&value),
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap_or_else(|error| panic!("{object} {property} {index:?}: {error:?}"));
        assert_eq!(
            server.read_local(&object, property, index).await.unwrap(),
            value,
            "{object} {property} {index:?}"
        );
    }
}

#[tokio::test]
async fn an_encoded_local_write_is_refused_as_a_network_write_is() {
    let server = server().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let description = encode(&PropertyValue::CharacterString("x".into()));
    for (object, property, index, value, class, code) in [
        (
            av,
            PropertyIdentifier::DESCRIPTION,
            Some(1),
            description.clone(),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            av,
            PropertyIdentifier::STATE_TEXT,
            Some(1),
            description.clone(),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            av,
            PropertyIdentifier::DESCRIPTION,
            None,
            Vec::new(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            oid(ObjectType::ANALOG_VALUE, 2),
            PropertyIdentifier::DESCRIPTION,
            None,
            description,
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
    ] {
        let error = server
            .write_local_encoded(
                &object,
                property,
                index,
                &value,
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
                if c == class && e == code),
            "{property} {index:?}: {error:?}"
        );
    }
}

async fn write_null(
    server: &BACnetServer<TestTransport>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<(), Error> {
    server
        .write_local_encoded(
            &object,
            property,
            index,
            &[0x00],
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

/// A NULL to a property that isn't commandable and has no NULL in its
/// datatype succeeds and changes nothing, locally as over the network
/// (#1396); the checks before the value still answer, and a commandable
/// Present_Value still relinquishes.
#[tokio::test]
async fn an_encoded_local_null_is_judged_as_a_network_null_is() {
    let server = server().await;
    let av = oid(ObjectType::ANALOG_VALUE, 1);
    let bo = oid(ObjectType::BINARY_OUTPUT, 1);
    let schedule = oid(ObjectType::SCHEDULE, 1);
    let staging = oid(ObjectType::STAGING, 1);
    for (object, property, index) in [
        (av, PropertyIdentifier::OBJECT_NAME, None),
        (av, PropertyIdentifier::COV_INCREMENT, None),
        (schedule, PropertyIdentifier::EFFECTIVE_PERIOD, None),
        (schedule, PropertyIdentifier::WEEKLY_SCHEDULE, Some(3)),
        (staging, PropertyIdentifier::STAGES, None),
        (staging, PropertyIdentifier::STAGES, Some(2)),
        (staging, PropertyIdentifier::TARGET_REFERENCES, None),
    ] {
        let before = server.read_local(&object, property, None).await.unwrap();
        write_null(&server, object, property, index)
            .await
            .unwrap_or_else(|error| panic!("{object} {property} {index:?}: {error:?}"));
        assert_eq!(
            server.read_local(&object, property, None).await.unwrap(),
            before,
            "{object} {property} {index:?}"
        );
    }
    for (object, property, index) in [
        (av, PropertyIdentifier::STATUS_FLAGS, None),
        // The number of stages is fixed.
        (staging, PropertyIdentifier::STAGES, Some(0)),
    ] {
        let error = write_null(&server, object, property, index)
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::WRITE_ACCESS_DENIED),
            "{property} {index:?}: {error:?}"
        );
    }
    let active = encode(&PropertyValue::Enumerated(1));
    server
        .write_local_encoded(
            &bo,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &active,
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    server
        .write_local_encoded(
            &bo,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &[0x00],
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(
        server
            .read_local(&bo, PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}
