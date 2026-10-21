use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_types::constructed::{BACnetObjectPropertyReference, PropertyReference};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};

fn structured<T: std::fmt::Debug>(
    result: Result<T, Error>,
    class: ErrorClass,
    code: ErrorCode,
) -> ErrorDetail {
    match result {
        Err(Error::Structured {
            class: c,
            code: k,
            detail,
        }) if c == class && k == code => *detail,
        other => panic!("expected a structured {class:?} / {code:?} error, got {other:?}"),
    }
}

/// CreateObject and SubscribeCOVPropertyMultiple errors end to end over
/// loopback UDP (#1047): the server sends each service's Clause 21 body and
/// the client reports its fields.
#[tokio::test]
async fn create_object_and_cov_multiple_errors_reach_the_client_with_their_detail() {
    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();
    let objects = server.database().read().await.len();

    // The second initial value, an out-of-range Present_Value, is refused.
    let result = client
        .create_object(
            &server_mac,
            ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
            vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: vec![0x72, 0, b'd'],
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x91, 9],
                    priority: None,
                },
            ],
        )
        .await;
    assert_eq!(
        structured(result, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE),
        ErrorDetail::FirstFailedElementNumber(2)
    );
    // An identifier in use is a refusal of the object, element 0.
    let result = client
        .create_object(
            &server_mac,
            ObjectSpecifier::Identifier(
                ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap(),
            ),
            vec![],
        )
        .await;
    assert_eq!(
        structured(
            result,
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS
        ),
        ErrorDetail::FirstFailedElementNumber(0)
    );
    assert_eq!(server.database().read().await.len(), objects);

    // The AnalogInput's second COV reference names a property it lacks.
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let missing = PropertyIdentifier::from_raw(200);
    let mut request = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 9,
        issue_confirmed_notifications: false,
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ai,
            list_of_cov_references: [PropertyIdentifier::PRESENT_VALUE, missing]
                .into_iter()
                .map(|property_identifier| COVReference {
                    monitored_property: PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let result = client
        .confirmed_request(
            &server_mac,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            &request,
        )
        .await;
    assert_eq!(
        structured(result, ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        ErrorDetail::FirstFailedSubscription(BACnetObjectPropertyReference::new(
            ai,
            missing.to_raw()
        ))
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
