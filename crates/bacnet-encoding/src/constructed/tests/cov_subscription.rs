use super::*;
use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVMultipleSubscription, BACnetCOVReference, BACnetCOVSubscription,
    BACnetCOVSubscriptionSpecification, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetRecipientProcess,
};
use bacnet_types::enums::PropertyIdentifier;

const DEVICE_SUBSCRIPTION: &[u8] = &[
    0x0E, 0x0E, 0x0C, 0x02, 0x00, 0x00, 0x07, 0x0F, 0x19, 0x07, 0x0F, 0x1E, 0x0C, 0x00, 0x40, 0x00,
    0x03, 0x19, 0x57, 0x29, 0x02, 0x1F, 0x29, 0x01, 0x3A, 0x01, 0x2C, 0x4C, 0x3F, 0x00, 0x00, 0x00,
];

const ADDRESS_SUBSCRIPTION: &[u8] = &[
    0x0E, 0x0E, 0x1E, 0x22, 0x12, 0x34, 0x62, 0xAA, 0xBB, 0x1F, 0x0F, 0x19, 0x09, 0x0F, 0x1E, 0x0C,
    0x01, 0x40, 0x00, 0x03, 0x19, 0x6F, 0x1F, 0x29, 0x00, 0x39, 0x00,
];

fn device_subscription() -> BACnetCOVSubscription {
    BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
            ),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new_indexed(
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
            87,
            2,
        ),
        issue_confirmed_notifications: true,
        time_remaining: 300,
        cov_increment: Some(0.5),
    }
}

fn address_subscription() -> BACnetCOVSubscription {
    BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: 0x1234,
                mac_address: bacnet_types::MacAddr::from_slice(&[0xAA, 0xBB]),
            }),
            process_identifier: 9,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap(),
            111,
        ),
        issue_confirmed_notifications: false,
        time_remaining: 0,
        cov_increment: None,
    }
}

#[test]
fn cov_subscription_device_recipient_golden() {
    let mut buf = BytesMut::new();
    encode_cov_subscription(&mut buf, &device_subscription());
    assert_eq!(buf.as_ref(), DEVICE_SUBSCRIPTION);
}

#[test]
fn cov_subscription_address_recipient_golden() {
    let mut buf = BytesMut::new();
    encode_cov_subscription(&mut buf, &address_subscription());
    assert_eq!(buf.as_ref(), ADDRESS_SUBSCRIPTION);
}

#[test]
fn cov_subscription_list_is_bare_concatenation() {
    let mut buf = BytesMut::new();
    encode_cov_subscription_list(&mut buf, &[device_subscription(), address_subscription()]);

    let expected = [DEVICE_SUBSCRIPTION, ADDRESS_SUBSCRIPTION].concat();
    assert_eq!(buf.as_ref(), expected);
}

#[test]
fn cov_subscription_list_empty_encodes_to_nothing() {
    let mut buf = BytesMut::new();
    encode_cov_subscription_list(&mut buf, &[]);
    assert!(buf.is_empty());
}

// Hand-assembled from the Clause 21 BACnetCOVMultipleSubscription production
// and Clause 20.2 tag rules, independently of the encoder under test.
#[rustfmt::skip]
const DEVICE_MULTIPLE: &[u8] = &[
    // [0] recipient process: [0] device 7, [1] process 7
    0x0E, 0x0E, 0x0C, 0x02, 0x00, 0x00, 0x07, 0x0F, 0x19, 0x07, 0x0F,
    // [1] confirmed, [2] time remaining 300, [3] max notification delay 10
    0x19, 0x01, 0x2A, 0x01, 0x2C, 0x39, 0x0A,
    0x4E,
    // AI:1 -> Present_Value (increment 0.5, timestamped), Priority_Array[8]
    0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E,
    0x0E, 0x09, 0x55, 0x0F, 0x1C, 0x3F, 0x00, 0x00, 0x00, 0x29, 0x01,
    0x0E, 0x09, 0x57, 0x19, 0x08, 0x0F, 0x29, 0x00,
    0x1F,
    // AV:3 -> Status_Flags
    0x0C, 0x00, 0x80, 0x00, 0x03, 0x1E, 0x0E, 0x09, 0x6F, 0x0F, 0x29, 0x00, 0x1F,
    0x4F,
];

#[rustfmt::skip]
const ADDRESS_MULTIPLE: &[u8] = &[
    // [0] recipient process: [0] recipient = [1] address (network 0x1234,
    // MAC AA BB), [1] process 9
    0x0E, 0x0E, 0x1E, 0x22, 0x12, 0x34, 0x62, 0xAA, 0xBB, 0x1F, 0x0F, 0x19, 0x09, 0x0F,
    // unconfirmed, one second remaining, zero delay, no specifications
    0x19, 0x00, 0x29, 0x01, 0x39, 0x00, 0x4E, 0x4F,
];

fn cov_reference(
    property: PropertyIdentifier,
    index: Option<u32>,
    cov_increment: Option<f32>,
    timestamped: bool,
) -> BACnetCOVReference {
    BACnetCOVReference {
        property_identifier: property,
        property_array_index: index,
        cov_increment,
        timestamped,
    }
}

fn device_multiple() -> BACnetCOVMultipleSubscription {
    BACnetCOVMultipleSubscription {
        recipient: device_subscription().recipient,
        issue_confirmed_notifications: true,
        time_remaining: 300,
        max_notification_delay: 10,
        list_of_cov_subscription_specifications: vec![
            BACnetCOVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                list_of_cov_references: vec![
                    cov_reference(PropertyIdentifier::PRESENT_VALUE, None, Some(0.5), true),
                    cov_reference(PropertyIdentifier::PRIORITY_ARRAY, Some(8), None, false),
                ],
            },
            BACnetCOVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3)
                    .unwrap(),
                list_of_cov_references: vec![cov_reference(
                    PropertyIdentifier::STATUS_FLAGS,
                    None,
                    None,
                    false,
                )],
            },
        ],
    }
}

fn address_multiple() -> BACnetCOVMultipleSubscription {
    BACnetCOVMultipleSubscription {
        recipient: address_subscription().recipient,
        issue_confirmed_notifications: false,
        time_remaining: 1,
        max_notification_delay: 0,
        list_of_cov_subscription_specifications: Vec::new(),
    }
}

#[test]
fn cov_multiple_subscription_nested_specifications_golden() {
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription(&mut buf, &device_multiple());
    assert_eq!(buf.as_ref(), DEVICE_MULTIPLE);
}

#[test]
fn cov_multiple_subscription_address_recipient_empty_specifications_golden() {
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription(&mut buf, &address_multiple());
    assert_eq!(buf.as_ref(), ADDRESS_MULTIPLE);
}

#[test]
fn cov_multiple_subscription_list_is_bare_concatenation() {
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription_list(&mut buf, &[address_multiple(), device_multiple()]);
    assert_eq!(buf.as_ref(), [ADDRESS_MULTIPLE, DEVICE_MULTIPLE].concat());

    let mut empty = BytesMut::new();
    encode_cov_multiple_subscription_list(&mut empty, &[]);
    assert!(empty.is_empty());
}
