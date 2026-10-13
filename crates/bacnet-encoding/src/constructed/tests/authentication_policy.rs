//! Access Point policy elements (#1325): golden bytes, round trips and
//! malformed input for `BACnetAuthenticationPolicy`.

use super::*;
use bacnet_types::constructed::{
    BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry, BACnetDeviceObjectReference,
};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn round_trip(policy: &BACnetAuthenticationPolicy, golden: &[u8]) {
    let mut encoded = BytesMut::new();
    encode_authentication_policy(&mut encoded, policy);
    assert_eq!(&encoded[..], golden, "{policy:?}");
    let (decoded, end) = decode_authentication_policy(golden, 0).unwrap();
    assert_eq!(&decoded, policy);
    assert_eq!(end, golden.len());
}

/// A card reader here at step 1, then a keypad in Device 99 at step 2,
/// in order, within 30 seconds.
fn card_then_pin() -> BACnetAuthenticationPolicy {
    BACnetAuthenticationPolicy {
        policy: vec![
            BACnetAuthenticationPolicyEntry {
                credential_data_input: oid(ObjectType::CREDENTIAL_DATA_INPUT, 1).into(),
                index: 1,
            },
            BACnetAuthenticationPolicyEntry {
                credential_data_input: BACnetDeviceObjectReference {
                    device_identifier: Some(oid(ObjectType::DEVICE, 99)),
                    object_identifier: oid(ObjectType::CREDENTIAL_DATA_INPUT, 2),
                },
                index: 2,
            },
        ],
        order_enforced: true,
        timeout: 30,
    }
}

#[test]
fn authentication_policy_golden_bytes_and_round_trip() {
    // Credential Data Input 1 is (37 << 22) | 1, Device 99 (8 << 22) | 99.
    round_trip(
        &card_then_pin(),
        &[
            0x0E, // policy [0]
            0x0E, // credential-data-input [0]
            0x1C, 0x09, 0x40, 0x00, 0x01, // object [1] CDI 1
            0x0F, //
            0x19, 0x01, // index [1] 1
            0x0E, //
            0x0C, 0x02, 0x00, 0x00, 0x63, // device [0] Device 99
            0x1C, 0x09, 0x40, 0x00, 0x02, // object [1] CDI 2
            0x0F, //
            0x19, 0x02, // index [1] 2
            0x0F, //
            0x19, 0x01, // order-enforced [1] TRUE
            0x29, 0x1E, // timeout [2] 30
        ],
    );

    // The default element of a grown array: no entries, unordered, no limit.
    round_trip(
        &BACnetAuthenticationPolicy::default(),
        &[0x0E, 0x0F, 0x19, 0x00, 0x29, 0x00],
    );
}

#[test]
fn authentication_policy_decoder_keeps_what_the_point_judges() {
    // An entry naming another object type, and indexes that skip a step,
    // decode as received.
    let kept = BACnetAuthenticationPolicy {
        policy: vec![
            BACnetAuthenticationPolicyEntry {
                credential_data_input: oid(ObjectType::ANALOG_INPUT, 1).into(),
                index: 0,
            },
            BACnetAuthenticationPolicyEntry {
                credential_data_input: oid(ObjectType::CREDENTIAL_DATA_INPUT, 1).into(),
                index: u32::MAX,
            },
        ],
        order_enforced: false,
        timeout: u32::MAX,
    };
    let mut encoded = BytesMut::new();
    encode_authentication_policy(&mut encoded, &kept);
    let (decoded, end) = decode_authentication_policy(&encoded, 0).unwrap();
    assert_eq!(decoded, kept);
    assert_eq!(end, encoded.len());
}

#[test]
fn authentication_policies_decode_one_element_at_a_time() {
    let first = card_then_pin();
    let second = BACnetAuthenticationPolicy::default();
    let mut array = BytesMut::new();
    encode_authentication_policy(&mut array, &first);
    let boundary = array.len();
    encode_authentication_policy(&mut array, &second);
    let (decoded, end) = decode_authentication_policy(&array, 0).unwrap();
    assert_eq!((decoded, end), (first, boundary));
    let (decoded, end) = decode_authentication_policy(&array, boundary).unwrap();
    assert_eq!((decoded, end), (second, array.len()));
}

#[test]
fn authentication_policy_rejects_malformed_elements() {
    let malformed: [&[u8]; 11] = [
        // Nothing at all.
        &[],
        // No entry frame.
        &[0x19, 0x00, 0x29, 0x00],
        // An entry frame that never closes.
        &[0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x0F, 0x19, 0x01],
        // A reference outside its frame.
        &[
            0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x19, 0x01, 0x0F, 0x19, 0x00, 0x29, 0x00,
        ],
        // An entry without its index.
        &[
            0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x0F, 0x0F, 0x19, 0x00, 0x29, 0x00,
        ],
        // Two objects in one reference frame.
        &[
            0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x1C, 0x09, 0x40, 0x00, 0x02, 0x0F, 0x19,
            0x01, 0x0F, 0x19, 0x00, 0x29, 0x00,
        ],
        // An index wider than 32 bits.
        &[
            0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x0F, 0x1D, 0x05, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x0F, 0x19, 0x00, 0x29, 0x00,
        ],
        // No order flag.
        &[0x0E, 0x0F, 0x29, 0x00],
        // An order flag that is neither 0 nor 1.
        &[0x0E, 0x0F, 0x19, 0x02, 0x29, 0x00],
        // No timeout.
        &[0x0E, 0x0F, 0x19, 0x00],
        // A timeout wider than 32 bits.
        &[
            0x0E, 0x0F, 0x19, 0x00, 0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00,
        ],
    ];
    for octets in malformed {
        assert!(
            decode_authentication_policy(octets, 0).is_err(),
            "{octets:02X?} decoded"
        );
    }
}
