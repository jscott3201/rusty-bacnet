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
    encode_cov_subscription(&mut buf, &device_subscription()).unwrap();
    assert_eq!(buf.as_ref(), DEVICE_SUBSCRIPTION);
}

#[test]
fn cov_subscription_address_recipient_golden() {
    let mut buf = BytesMut::new();
    encode_cov_subscription(&mut buf, &address_subscription()).unwrap();
    assert_eq!(buf.as_ref(), ADDRESS_SUBSCRIPTION);
}

#[test]
fn cov_subscription_list_is_bare_concatenation() {
    let mut buf = BytesMut::new();
    encode_cov_subscription_list(&mut buf, &[device_subscription(), address_subscription()])
        .unwrap();

    let expected = [DEVICE_SUBSCRIPTION, ADDRESS_SUBSCRIPTION].concat();
    assert_eq!(buf.as_ref(), expected);
}

#[test]
fn cov_subscription_list_empty_encodes_to_nothing() {
    let mut buf = BytesMut::new();
    encode_cov_subscription_list(&mut buf, &[]).unwrap();
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
    encode_cov_multiple_subscription(&mut buf, &device_multiple()).unwrap();
    assert_eq!(buf.as_ref(), DEVICE_MULTIPLE);
}

#[test]
fn cov_multiple_subscription_address_recipient_empty_specifications_golden() {
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription(&mut buf, &address_multiple()).unwrap();
    assert_eq!(buf.as_ref(), ADDRESS_MULTIPLE);
}

#[test]
fn cov_multiple_subscription_list_is_bare_concatenation() {
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription_list(&mut buf, &[address_multiple(), device_multiple()])
        .unwrap();
    assert_eq!(buf.as_ref(), [ADDRESS_MULTIPLE, DEVICE_MULTIPLE].concat());

    let mut empty = BytesMut::new();
    encode_cov_multiple_subscription_list(&mut empty, &[]).unwrap();
    assert!(empty.is_empty());
}

// --- Element walkers (#1046): each golden vector above is one list element.

#[test]
fn cov_subscription_decodes_golden_elements() {
    for (bytes, expected) in [
        (DEVICE_SUBSCRIPTION, device_subscription()),
        (ADDRESS_SUBSCRIPTION, address_subscription()),
    ] {
        assert_eq!(
            decode_cov_subscription(bytes, 0).unwrap(),
            (expected, bytes.len())
        );
    }
}

#[test]
fn cov_subscription_list_walks_one_element_at_a_time() {
    let list = [
        DEVICE_SUBSCRIPTION,
        ADDRESS_SUBSCRIPTION,
        DEVICE_SUBSCRIPTION,
    ]
    .concat();
    let mut offset = 0;
    let mut decoded = Vec::new();
    while offset < list.len() {
        let (subscription, end) = decode_cov_subscription(&list, offset).unwrap();
        decoded.push(subscription);
        offset = end;
    }
    assert_eq!(
        decoded,
        vec![
            device_subscription(),
            address_subscription(),
            device_subscription()
        ]
    );
    assert_eq!(offset, list.len());
}

#[test]
fn cov_subscription_truncations_fail_except_before_the_optional_increment() {
    // Without its trailing [4] REAL the device element is still complete.
    let without_increment = DEVICE_SUBSCRIPTION.len() - 5;
    for len in 0..DEVICE_SUBSCRIPTION.len() {
        let result = decode_cov_subscription(&DEVICE_SUBSCRIPTION[..len], 0);
        if len == without_increment {
            let (subscription, end) = result.unwrap();
            assert_eq!((subscription.cov_increment, end), (None, len));
        } else {
            assert!(result.is_err(), "prefix {len}: {result:?}");
        }
    }
    for len in 0..ADDRESS_SUBSCRIPTION.len() {
        let result = decode_cov_subscription(&ADDRESS_SUBSCRIPTION[..len], 0);
        assert!(result.is_err(), "prefix {len}: {result:?}");
    }
}

#[test]
fn cov_subscription_rejects_malformed_members() {
    // A BOOLEAN contents octet other than 0 or 1.
    let mut form = DEVICE_SUBSCRIPTION.to_vec();
    assert_eq!(form[22..24], [0x29, 0x01]);
    form[23] = 0x02;
    // BACnetObjectPropertyReference has no [3] device member.
    #[rustfmt::skip]
    let device_reference = [
        &DEVICE_SUBSCRIPTION[..11],
        &[0x1E, 0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x57, 0x3C, 0x02, 0x00, 0x00, 0x07, 0x1F],
        &[0x29, 0x00, 0x39, 0x00],
    ]
    .concat();
    // An application-tagged value where the [0] recipient process opens.
    let untagged = [0x21, 0x01];
    for bytes in [&form[..], &device_reference, &untagged] {
        let result = decode_cov_subscription(bytes, 0);
        assert!(result.is_err(), "{bytes:02X?}: {result:?}");
    }
}

#[test]
fn cov_multiple_subscription_decodes_golden_elements_and_walks_a_list() {
    for (bytes, expected) in [
        (DEVICE_MULTIPLE, device_multiple()),
        (ADDRESS_MULTIPLE, address_multiple()),
    ] {
        assert_eq!(
            decode_cov_multiple_subscription(bytes, 0).unwrap(),
            (expected, bytes.len())
        );
    }
    let list = [ADDRESS_MULTIPLE, DEVICE_MULTIPLE].concat();
    let (first, next) = decode_cov_multiple_subscription(&list, 0).unwrap();
    let (second, end) = decode_cov_multiple_subscription(&list, next).unwrap();
    assert_eq!((first, next), (address_multiple(), ADDRESS_MULTIPLE.len()));
    assert_eq!((second, end), (device_multiple(), list.len()));
}

#[test]
fn cov_multiple_subscription_rejects_truncation_and_malformed_members() {
    // Every element ends with its closing [4] tag, so no prefix is complete.
    for bytes in [DEVICE_MULTIPLE, ADDRESS_MULTIPLE] {
        for len in 0..bytes.len() {
            let result = decode_cov_multiple_subscription(&bytes[..len], 0);
            assert!(result.is_err(), "prefix {len}: {result:?}");
        }
    }
    let header = &ADDRESS_MULTIPLE[..20];
    // A one-octet BOOLEAN form flag whose contents are neither 0 nor 1.
    let mut form = ADDRESS_MULTIPLE.to_vec();
    assert_eq!(form[14..16], [0x19, 0x00]);
    form[15] = 0x05;
    // A specification must open with its [0] monitored object identifier.
    let stray = [header, &[0x4E, 0x29, 0x00, 0x4F]].concat();
    // A COV reference must end with its [2] timestamped flag.
    #[rustfmt::skip]
    let untimestamped = [
        header,
        &[0x4E, 0x0C, 0x00, 0x80, 0x00, 0x03, 0x1E, 0x0E, 0x09, 0x6F, 0x0F, 0x1F, 0x4F],
    ]
    .concat();
    for bytes in [&form, &stray, &untimestamped] {
        let result = decode_cov_multiple_subscription(bytes, 0);
        assert!(result.is_err(), "{bytes:02X?}: {result:?}");
    }
}

/// `golden`, an address-recipient element whose two-octet MAC sits at
/// octets 6..9, with a `len`-octet MAC of 0xA5 instead.
fn with_mac(golden: &[u8], len: usize) -> Vec<u8> {
    let mut wire = golden[..6].to_vec();
    wire.extend([0x65, len as u8]);
    wire.extend(std::iter::repeat_n(0xA5, len));
    wire.extend_from_slice(&golden[9..]);
    wire
}

#[test]
fn cov_subscription_recipient_mac_holds_to_the_bacnet_address_bound() {
    // #1156: a subscription's recipient is a BACnetRecipient, so its address
    // MAC is at most BACnetAddress::MAX_MAC_LEN (18) octets in both forms.
    let recipient = |len: usize| BACnetRecipientProcess {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0x1234,
            mac_address: bacnet_types::MacAddr::from_slice(&vec![0xA5; len]),
        }),
        process_identifier: 9,
    };
    let single = |len| BACnetCOVSubscription {
        recipient: recipient(len),
        ..address_subscription()
    };
    let multiple = |len| BACnetCOVMultipleSubscription {
        recipient: recipient(len),
        ..address_multiple()
    };
    let longest = BACnetAddress::MAX_MAC_LEN;
    let (single_wire, multiple_wire) = (
        with_mac(ADDRESS_SUBSCRIPTION, longest),
        with_mac(ADDRESS_MULTIPLE, longest),
    );
    let mut buf = BytesMut::new();
    encode_cov_subscription_list(&mut buf, &[single(longest)]).unwrap();
    assert_eq!(buf.as_ref(), single_wire);
    let mut buf = BytesMut::new();
    encode_cov_multiple_subscription_list(&mut buf, &[multiple(longest)]).unwrap();
    assert_eq!(buf.as_ref(), multiple_wire);
    assert_eq!(
        decode_cov_subscription(&single_wire, 0).unwrap(),
        (single(longest), single_wire.len())
    );
    assert_eq!(
        decode_cov_multiple_subscription(&multiple_wire, 0).unwrap(),
        (multiple(longest), multiple_wire.len())
    );

    for len in [longest + 1, 255] {
        let result = decode_cov_subscription(&with_mac(ADDRESS_SUBSCRIPTION, len), 0);
        assert!(result.is_err(), "{len}-octet MAC: {result:?}");
        let result = decode_cov_multiple_subscription(&with_mac(ADDRESS_MULTIPLE, len), 0);
        assert!(result.is_err(), "{len}-octet MAC: {result:?}");
        // Each encoder refuses before writing, a list included.
        let mut buf = BytesMut::new();
        assert!(encode_cov_subscription(&mut buf, &single(len)).is_err());
        assert!(encode_cov_multiple_subscription(&mut buf, &multiple(len)).is_err());
        assert!(
            encode_cov_subscription_list(&mut buf, &[device_subscription(), single(len)]).is_err()
        );
        assert!(
            encode_cov_multiple_subscription_list(&mut buf, &[device_multiple(), multiple(len)])
                .is_err()
        );
        assert!(buf.is_empty(), "{len}-octet MAC left output behind");
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for octets in [DEVICE_SUBSCRIPTION, ADDRESS_SUBSCRIPTION] {
        framed += assert_members_cut_short("BACnetCOVSubscription", octets, |data| {
            decode_cov_subscription(data, 0)
        });
    }
    for octets in [DEVICE_MULTIPLE, ADDRESS_MULTIPLE] {
        framed += assert_members_cut_short("BACnetCOVMultipleSubscription", octets, |data| {
            decode_cov_multiple_subscription(data, 0)
        });
    }
    assert!(framed > 0);
}
