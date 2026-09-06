use super::*;
use bacnet_types::constructed::{
    BACnetAddress, BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
    BACnetRecipientProcess,
};

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
