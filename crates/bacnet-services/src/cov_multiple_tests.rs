use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::Time;

#[test]
fn subscribe_cov_property_multiple_round_trip() {
    let req = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 42,
        issue_confirmed_notifications: true,
        lifetime: Some(60),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(1.0),
                    timestamped: true,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    let decoded = SubscribeCOVPropertyMultipleRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn subscribe_cov_property_multiple_uses_standard_tags() {
    let req = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 42,
        issue_confirmed_notifications: true,
        lifetime: Some(60),
        max_notification_delay: Some(5),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10)
                .unwrap(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: Some(1.0),
                timestamped: true,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut offset = 0;
    for tag_number in [0, 1, 2, 3] {
        let (tag, pos) = tags::decode_tag(&buf, offset).unwrap();
        assert!(tag.is_context(tag_number));
        offset = pos + tag.length as usize;
    }
    let (tag, _) = tags::decode_tag(&buf, offset).unwrap();
    assert!(tag.is_opening_tag(4));
}

#[test]
fn subscribe_cov_property_multiple_rejects_malformed_header_fields() {
    let mut malformed_bool = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut malformed_bool, 0, 1);
    tags::encode_tag(&mut malformed_bool, 1, tags::TagClass::Context, 0);
    tags::encode_opening_tag(&mut malformed_bool, 4);
    tags::encode_closing_tag(&mut malformed_bool, 4);
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&malformed_bool).is_err());

    let mut oversized_lifetime = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut oversized_lifetime, 0, 1);
    primitives::encode_ctx_boolean(&mut oversized_lifetime, 1, true);
    tags::encode_tag(&mut oversized_lifetime, 2, tags::TagClass::Context, 5);
    oversized_lifetime.extend_from_slice(&[1, 0, 0, 0, 0]);
    tags::encode_opening_tag(&mut oversized_lifetime, 4);
    tags::encode_closing_tag(&mut oversized_lifetime, 4);
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&oversized_lifetime).is_err());
}

#[test]
fn subscribe_cov_property_multiple_minimal() {
    let req = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_INPUT, 5)
                .unwrap(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    let decoded = SubscribeCOVPropertyMultipleRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn cov_notification_multiple_round_trip() {
    let req = COVNotificationMultipleRequest {
        subscriber_process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        time_remaining: 60,
        timestamp: Some((
            bacnet_types::primitives::Date {
                year: 126,
                month: 8,
                day: 20,
                day_of_week: 4,
            },
            Time {
                hour: 12,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
        )),
        list_of_cov_notifications: vec![COVNotificationItem {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                .unwrap(),
            list_of_values: vec![
                COVNotificationValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    time_of_change: None,
                },
                COVNotificationValue {
                    property_identifier: PropertyIdentifier::STATUS_FLAGS,
                    property_array_index: None,
                    value: vec![0x82, 0x04, 0x00],
                    time_of_change: Some(Time {
                        hour: 12,
                        minute: 30,
                        second: 0,
                        hundredths: 0,
                    }),
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = COVNotificationMultipleRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn cov_notification_multiple_with_time_timestamp() {
    let req = COVNotificationMultipleRequest {
        subscriber_process_identifier: 5,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap(),
        time_remaining: 0,
        timestamp: None,
        list_of_cov_notifications: vec![COVNotificationItem {
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3)
                .unwrap(),
            list_of_values: vec![COVNotificationValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x91, 0x01],
                time_of_change: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    let decoded = COVNotificationMultipleRequest::decode(&buf).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn subscribe_cov_property_multiple_empty_input() {
    assert!(SubscribeCOVPropertyMultipleRequest::decode(&[]).is_err());
}

#[test]
fn cov_notification_multiple_empty_input() {
    assert!(COVNotificationMultipleRequest::decode(&[]).is_err());
}
