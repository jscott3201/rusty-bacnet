//! An Object_Name another object holds, refused over the wire (#1434).

use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};

/// Application CharacterString (UTF-8) `text`.
fn name(text: &str) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut bytes, text).unwrap();
    bytes.to_vec()
}

fn property_value(property_identifier: PropertyIdentifier, value: Vec<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value,
        priority: None,
    }
}

/// The class and code a refusal reached the client with, and the detail its
/// service's error body added.
fn refusal<T: std::fmt::Debug>(
    result: Result<T, Error>,
) -> (ErrorClass, ErrorCode, Option<ErrorDetail>) {
    let (class, code, detail) = match result {
        Err(Error::Protocol { class, code }) => (class, code, None),
        Err(Error::Structured {
            class,
            code,
            detail,
        }) => (class, code, Some(*detail)),
        other => panic!("expected an error PDU, got {other:?}"),
    };
    (class, code, detail)
}

/// WriteProperty, WritePropertyMultiple and a CreateObject initial value each
/// answer a name already in use with PROPERTY / DUPLICATE_NAME: the WP and
/// WPM error tables (Clauses 15.9.1.3.1 and 15.10.1.3.1) give that pair, and
/// Clause 18.3 files the code under PROPERTY. Nothing is renamed or created.
#[tokio::test]
async fn a_name_in_use_is_property_duplicate_name_over_wp_wpm_and_create_object() {
    let mut server = make_server().await;
    let mut client = make_client().await;
    let mac = server.local_mac().to_vec();
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let objects = server.database().read().await.len();
    let taken = "Fan Status";

    let result = client
        .write_property(
            &mac,
            ai,
            PropertyIdentifier::OBJECT_NAME,
            None,
            name(taken),
            None,
        )
        .await;
    assert_eq!(
        refusal(result),
        (ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME, None),
        "WriteProperty"
    );

    let result = client
        .write_property_multiple(
            &mac,
            vec![WriteAccessSpecification {
                object_identifier: ai,
                list_of_properties: vec![property_value(
                    PropertyIdentifier::OBJECT_NAME,
                    name(taken),
                )],
            }],
        )
        .await;
    assert_eq!(
        refusal(result),
        (
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_NAME,
            Some(ErrorDetail::FirstFailedWriteAttempt(
                BACnetObjectPropertyReference::new(ai, PropertyIdentifier::OBJECT_NAME.to_raw())
            ))
        ),
        "WritePropertyMultiple"
    );

    // CreateObject's own table has no row for a name in use; the initial
    // value is refused as WriteProperty refuses it, at its position.
    let result = client
        .create_object(
            &mac,
            ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
            vec![
                property_value(PropertyIdentifier::DESCRIPTION, name("d")),
                property_value(PropertyIdentifier::OBJECT_NAME, name(taken)),
            ],
        )
        .await;
    assert_eq!(
        refusal(result),
        (
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_NAME,
            Some(ErrorDetail::FirstFailedElementNumber(2))
        ),
        "CreateObject"
    );

    {
        let db = server.database().read().await;
        assert_eq!(db.len(), objects);
        assert_eq!(db.get(&ai).unwrap().object_name(), "Zone Temp");
        assert_eq!(
            db.find_by_name(taken).map(|o| o.object_identifier()),
            Some(ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap())
        );
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
