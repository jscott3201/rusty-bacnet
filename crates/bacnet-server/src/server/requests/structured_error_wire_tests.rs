//! CreateObject and SubscribeCOVPropertyMultiple errors on the wire (#1047).
//!
//! CreateObject answers every error with a CreateObject-Error (Clause 21):
//! the class and code inside an opening and closing tag 0 (0x0E ... 0x0F),
//! then the First Failed Element Number as context tag 1 (0x19 for one
//! octet). It is the position, counted from 1, of the refused initial value,
//! and zero when the request or the object is refused.
//!
//! SubscribeCOVPropertyMultiple answers with one choice of its error: a
//! failure before any COV reference is processed is the class and code
//! inside tag 0 alone; a refused reference is an opening tag 1 (0x1E)
//! around the monitored object as context tag 0 (0x0C, four octets), the
//! reference's property and index inside tag 1, and the class and code
//! inside tag 2 (0x2E ... 0x2F).

use super::mutation_tests::{Fixture, oid};
use super::*;
use crate::mutation::MutationPolicy;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};
use bacnet_types::constructed::PropertyReference;

const CREATE: ConfirmedServiceChoice = ConfirmedServiceChoice::CREATE_OBJECT;
const SCPM: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;

/// The response APDU exactly as the server encoded it.
async fn wire(fixture: &Fixture, service: ConfirmedServiceChoice, request: Bytes) -> Vec<u8> {
    let npdu = fixture
        .dispatch(service, request, 5)
        .await
        .expect("response");
    decode_npdu(npdu).unwrap().payload.to_vec()
}

fn create(specifier: ObjectSpecifier, values: Vec<(PropertyIdentifier, Vec<u8>)>) -> Bytes {
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: specifier,
        list_of_initial_values: values
            .into_iter()
            .map(|(property_identifier, value)| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value,
                priority: None,
            })
            .collect(),
    }
    .encode(&mut bytes);
    bytes.freeze()
}

/// The Error PDU header for invoke ID 5, then the CreateObject-Error body.
fn create_object_error(class: ErrorClass, code: ErrorCode, element: u8) -> Vec<u8> {
    vec![
        0x50,
        5,
        CREATE.to_raw(),
        0x0E,
        0x91,
        class.to_raw() as u8,
        0x91,
        code.to_raw() as u8,
        0x0F,
        0x19,
        element,
    ]
}

/// Application CharacterString (UTF-8) `text`, short form.
fn name(text: &str) -> Vec<u8> {
    let mut bytes = vec![0x70 | (text.len() as u8 + 1), 0];
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

#[tokio::test]
async fn create_object_refusals_go_out_as_create_object_errors() {
    let fixture = Fixture::new(None);
    let before = fixture.db.read().await.len();
    let description = || (PropertyIdentifier::DESCRIPTION, name("d"));
    for (what, request, expected) in [
        (
            "an identifier already in use",
            create(
                ObjectSpecifier::Identifier(oid(ObjectType::BINARY_VALUE, 1)),
                vec![description()],
            ),
            create_object_error(
                ErrorClass::OBJECT,
                ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
                0,
            ),
        ),
        (
            "an object type the server cannot create",
            create(ObjectSpecifier::Type(ObjectType::ANALOG_VALUE), vec![]),
            create_object_error(ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE, 0),
        ),
        (
            "a name in use, the first initial value",
            create(
                ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
                vec![
                    (PropertyIdentifier::OBJECT_NAME, name("two")),
                    description(),
                ],
            ),
            create_object_error(ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME, 1),
        ),
        (
            "an out-of-range Present_Value, the second initial value",
            create(
                ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
                vec![
                    description(),
                    (PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]),
                ],
            ),
            create_object_error(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
        ),
        (
            "an undecodable third initial value",
            create(
                ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
                vec![
                    description(),
                    (PropertyIdentifier::OBJECT_NAME, name("x")),
                    (PropertyIdentifier::PRESENT_VALUE, vec![0xD1, 0x00]),
                ],
            ),
            create_object_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING, 3),
        ),
    ] {
        assert_eq!(wire(&fixture, CREATE, request).await, expected, "{what}");
        // Nothing is created, and no name is left behind.
        let db = fixture.db.read().await;
        assert_eq!(db.len(), before, "{what}");
        assert!(
            db.check_name_available(&oid(ObjectType::BINARY_VALUE, 3), "x")
                .is_ok()
        );
    }

    // A good request still creates the object.
    let response = wire(
        &fixture,
        CREATE,
        create(
            ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
            vec![description()],
        ),
    )
    .await;
    assert_eq!(&response[..3], &[0x30, 5, CREATE.to_raw()]);
    assert_eq!(fixture.db.read().await.len(), before + 1);
}

#[tokio::test]
async fn create_object_policy_denial_names_no_element() {
    let mut fixture = Fixture::new(None);
    fixture.config.mutation_policy = MutationPolicy::DenyAll;
    assert_eq!(
        wire(
            &fixture,
            CREATE,
            create(ObjectSpecifier::Type(ObjectType::BINARY_VALUE), vec![]),
        )
        .await,
        create_object_error(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED, 0)
    );
}

fn subscribe(lifetime: u32, specs: Vec<(ObjectIdentifier, Vec<PropertyReference>)>) -> Bytes {
    let mut bytes = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 4,
        issue_confirmed_notifications: false,
        lifetime: Some(lifetime),
        max_notification_delay: Some(0),
        list_of_cov_subscription_specifications: specs
            .into_iter()
            .map(|(object, references)| COVSubscriptionSpecification {
                monitored_object_identifier: object,
                list_of_cov_references: references
                    .into_iter()
                    .map(|monitored_property| COVReference {
                        monitored_property,
                        cov_increment: None,
                        timestamped: false,
                    })
                    .collect(),
            })
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

/// `request` with its one-octet lifetime of 1 (context tag 2, 0x29) set to
/// zero, which the request encoder refuses to produce.
fn zero_lifetime(request: Bytes) -> Bytes {
    let mut bytes = request.to_vec();
    assert_eq!(&bytes[4..6], &[0x29, 1]);
    bytes[5] = 0;
    Bytes::from(bytes)
}

fn reference(property: PropertyIdentifier, index: Option<u32>) -> PropertyReference {
    PropertyReference {
        property_identifier: property,
        property_array_index: index,
    }
}

/// The Error PDU header for invoke ID 5, then the general choice.
fn general_error(class: ErrorClass, code: ErrorCode) -> Vec<u8> {
    vec![
        0x50,
        5,
        SCPM.to_raw(),
        0x0E,
        0x91,
        class.to_raw() as u8,
        0x91,
        code.to_raw() as u8,
        0x0F,
    ]
}

/// The Error PDU header for invoke ID 5, then the first-failed-subscription
/// choice naming `object` and its property reference.
fn subscription_error(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u8>,
    class: ErrorClass,
    code: ErrorCode,
) -> Vec<u8> {
    let mut apdu = vec![0x50, 5, SCPM.to_raw(), 0x1E, 0x0C];
    apdu.extend_from_slice(&object.encode());
    apdu.extend_from_slice(&[0x1E, 0x09, property.to_raw() as u8]);
    if let Some(index) = index {
        apdu.extend_from_slice(&[0x19, index]);
    }
    apdu.extend_from_slice(&[
        0x1F,
        0x2E,
        0x91,
        class.to_raw() as u8,
        0x91,
        code.to_raw() as u8,
        0x2F,
        0x1F,
    ]);
    apdu
}

#[tokio::test]
async fn subscribe_cov_property_multiple_refusals_go_out_in_their_choice() {
    let fixture = Fixture::new(None);
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    let missing = oid(ObjectType::BINARY_VALUE, 99);
    let present_value = || reference(PropertyIdentifier::PRESENT_VALUE, None);
    // The unknown object's first reference is the first that failed.
    let unknown_object = subscribe(300, vec![(missing, vec![present_value()])]);
    assert_eq!(
        wire(&fixture, SCPM, unknown_object).await,
        [
            &[0x50, 5, 30, 0x1E, 0x0C, 0x01, 0x40, 0x00, 0x63][..],
            &[0x1E, 0x09, 85, 0x1F, 0x2E, 0x91, 1, 0x91, 31, 0x2F, 0x1F],
        ]
        .concat(),
        "golden vector: (BINARY_VALUE, 99) PRESENT_VALUE, OBJECT / UNKNOWN_OBJECT"
    );
    // The good reference before a refused one stays subscribed (#1058); the
    // second request renews it, and the zero lifetime changes nothing.
    for (what, request, expected) in [
        (
            "a property that does not exist, after a good reference",
            subscribe(
                300,
                vec![(
                    bv,
                    vec![
                        present_value(),
                        reference(PropertyIdentifier::from_raw(200), None),
                    ],
                )],
            ),
            subscription_error(
                bv,
                PropertyIdentifier::from_raw(200),
                None,
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            ),
        ),
        (
            "an index on a property that is not an array, in the second object",
            subscribe(
                300,
                vec![
                    (bv, vec![present_value()]),
                    (
                        oid(ObjectType::BINARY_VALUE, 2),
                        vec![reference(PropertyIdentifier::PRESENT_VALUE, Some(1))],
                    ),
                ],
            ),
            subscription_error(
                oid(ObjectType::BINARY_VALUE, 2),
                PropertyIdentifier::PRESENT_VALUE,
                Some(1),
                ErrorClass::PROPERTY,
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            ),
        ),
        (
            "a lifetime of zero, before any reference",
            zero_lifetime(subscribe(1, vec![(missing, vec![present_value()])])),
            general_error(ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE),
        ),
    ] {
        assert_eq!(wire(&fixture, SCPM, request).await, expected, "{what}");
        let mut table = fixture.table.write().await;
        assert_eq!(table.len(), 1, "{what}");
        let kept: Vec<_> = table
            .subscriptions_for(&bv)
            .into_iter()
            .map(|sub| sub.monitored_property)
            .collect();
        assert_eq!(kept, [Some(PropertyIdentifier::PRESENT_VALUE)], "{what}");
    }
}

#[tokio::test]
async fn subscribe_cov_property_multiple_policy_denial_is_the_general_choice() {
    let mut fixture = Fixture::new(None);
    fixture.config.mutation_policy = MutationPolicy::DenyAll;
    let request = subscribe(
        300,
        vec![(
            oid(ObjectType::BINARY_VALUE, 1),
            vec![reference(PropertyIdentifier::PRESENT_VALUE, None)],
        )],
    );
    assert_eq!(
        wire(&fixture, SCPM, request).await,
        general_error(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
    );
}
