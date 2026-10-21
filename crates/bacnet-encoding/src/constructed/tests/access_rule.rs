//! Access Rights rule elements (#1316): golden bytes, round trips and
//! malformed input for `BACnetAccessRule`.

use super::*;
use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::enums::{AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Schedule 1's Present_Value (property 85) in this device.
fn schedule_present_value() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(oid(ObjectType::SCHEDULE, 1), 85)
}

fn round_trip(rule: &BACnetAccessRule, golden: &[u8]) {
    let mut encoded = BytesMut::new();
    encode_access_rule(&mut encoded, rule);
    assert_eq!(&encoded[..], golden, "{rule:?}");
    let (decoded, end) = decode_access_rule(golden, 0).unwrap();
    assert_eq!(&decoded, rule);
    assert_eq!(end, golden.len());
}

#[test]
fn access_rule_golden_bytes_and_round_trip() {
    // Schedule 1 is (17 << 22) | 1, Access Point 2 is (33 << 22) | 2.
    let local = BACnetAccessRule::new(
        Some(schedule_present_value()),
        Some(oid(ObjectType::ACCESS_POINT, 2).into()),
        true,
    );
    round_trip(
        &local,
        &[
            0x09, 0x00, // time-range-specifier [0] SPECIFIED
            0x1E, // time-range [1]
            0x0C, 0x04, 0x40, 0x00, 0x01, // object [0] Schedule 1
            0x19, 0x55, // property [1] Present_Value
            0x1F, //
            0x29, 0x00, // location-specifier [2] SPECIFIED
            0x3E, // location [3]
            0x1C, 0x08, 0x40, 0x00, 0x02, // object [1] Access Point 2
            0x3F, //
            0x49, 0x01, // enable [4] TRUE
        ],
    );

    // ALWAYS and ALL leave both references out.
    round_trip(
        &BACnetAccessRule::new(None, None, false),
        &[0x09, 0x01, 0x29, 0x01, 0x49, 0x00],
    );

    // References into Device 99 ((8 << 22) | 99), with an array index on
    // the time range, to Access Zone 5 ((36 << 22) | 5).
    let remote = BACnetAccessRule::new(
        Some(
            BACnetDeviceObjectPropertyReference::new_remote(
                oid(ObjectType::SCHEDULE, 1),
                85,
                oid(ObjectType::DEVICE, 99),
            )
            .with_index(3),
        ),
        Some(BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 99)),
            object_identifier: oid(ObjectType::ACCESS_ZONE, 5),
        }),
        true,
    );
    round_trip(
        &remote,
        &[
            0x09, 0x00, 0x1E, 0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x55, //
            0x29, 0x03, // property-array-index [2] 3
            0x3C, 0x02, 0x00, 0x00, 0x63, // device [3] Device 99
            0x1F, 0x29, 0x00, 0x3E, //
            0x0C, 0x02, 0x00, 0x00, 0x63, // device [0] Device 99
            0x1C, 0x09, 0x00, 0x00, 0x05, // object [1] Access Zone 5
            0x3F, 0x49, 0x01,
        ],
    );
}

#[test]
fn access_rule_decoder_keeps_what_the_object_judges() {
    // ALWAYS with an unspecified time range (instance 4194303) still
    // carries the reference, and a specifier outside the two named values
    // is kept as received.
    let unspecified = BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::SCHEDULE, ObjectIdentifier::MAX_INSTANCE),
        85,
    );
    let kept = BACnetAccessRule {
        time_range_specifier: AccessRuleTimeRangeSpecifier::ALWAYS,
        time_range: Some(unspecified),
        location_specifier: AccessRuleLocationSpecifier::from_raw(7),
        location: None,
        enable: true,
    };
    let mut encoded = BytesMut::new();
    encode_access_rule(&mut encoded, &kept);
    let (decoded, end) = decode_access_rule(&encoded, 0).unwrap();
    assert_eq!(decoded, kept);
    assert_eq!(end, encoded.len());

    // SPECIFIED without its reference is a structural fit too.
    let bare: &[u8] = &[0x09, 0x00, 0x29, 0x00, 0x49, 0x01];
    let (decoded, _) = decode_access_rule(bare, 0).unwrap();
    assert_eq!(
        decoded.time_range_specifier,
        AccessRuleTimeRangeSpecifier::SPECIFIED
    );
    assert_eq!((decoded.time_range, decoded.location), (None, None));
}

#[test]
fn access_rules_decode_one_element_at_a_time() {
    let first = BACnetAccessRule::new(None, Some(oid(ObjectType::ACCESS_POINT, 2).into()), true);
    let second = BACnetAccessRule::new(Some(schedule_present_value()), None, false);
    let mut array = BytesMut::new();
    encode_access_rule(&mut array, &first);
    let boundary = array.len();
    encode_access_rule(&mut array, &second);
    let (decoded, end) = decode_access_rule(&array, 0).unwrap();
    assert_eq!((decoded, end), (first, boundary));
    let (decoded, end) = decode_access_rule(&array, boundary).unwrap();
    assert_eq!((decoded, end), (second, array.len()));
}

#[test]
fn access_rule_rejects_malformed_elements() {
    let malformed: [&[u8]; 13] = [
        // Nothing at all.
        &[],
        // No time-range specifier.
        &[0x29, 0x01, 0x49, 0x01],
        // The specifiers out of order.
        &[0x29, 0x01, 0x09, 0x01, 0x49, 0x01],
        // No enable flag.
        &[0x09, 0x01, 0x29, 0x01],
        // A BOOLEAN must hold 0 or 1.
        &[0x09, 0x01, 0x29, 0x01, 0x49, 0x02],
        // The enable flag under the wrong tag.
        &[0x09, 0x01, 0x29, 0x01, 0x39, 0x01],
        // A specifier wider than 32 bits.
        &[
            0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x29, 0x01, 0x49, 0x01,
        ],
        // The time-range frame never closes.
        &[
            0x09, 0x00, 0x1E, 0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x55, 0x29, 0x01, 0x49, 0x01,
        ],
        // An empty time-range frame: the object identifier is required.
        &[0x09, 0x00, 0x1E, 0x1F, 0x29, 0x01, 0x49, 0x01],
        // A device-object reference where the time range needs a property.
        &[
            0x09, 0x00, 0x1E, 0x1C, 0x04, 0x40, 0x00, 0x01, 0x1F, 0x29, 0x01, 0x49, 0x01,
        ],
        // Something after the reference inside the location frame.
        &[
            0x09, 0x01, 0x29, 0x00, 0x3E, 0x1C, 0x08, 0x40, 0x00, 0x02, 0x21, 0x01, 0x3F, 0x49,
            0x01,
        ],
        // The location without its frame.
        &[
            0x09, 0x01, 0x29, 0x00, 0x1C, 0x08, 0x40, 0x00, 0x02, 0x49, 0x01,
        ],
        // Truncated inside the location.
        &[0x09, 0x01, 0x29, 0x00, 0x3E, 0x1C, 0x08, 0x40],
    ];
    for bytes in malformed {
        assert!(
            decode_access_rule(bytes, 0).is_err(),
            "{bytes:02X?} must not decode"
        );
    }

    // Contents cut short are BufferTooShort; a wrong tag is Decoding.
    let truncated: &[u8] = &[0x09, 0x01, 0x29, 0x00, 0x3E, 0x1C, 0x08, 0x40];
    assert!(
        matches!(
            decode_access_rule(truncated, 0),
            Err(Error::BufferTooShort { .. })
        ),
        "{:?}",
        decode_access_rule(truncated, 0)
    );
    let wrong_tag: &[u8] = &[0x09, 0x01, 0x29, 0x01, 0x39, 0x01];
    assert!(
        matches!(decode_access_rule(wrong_tag, 0), Err(Error::Decoding(_))),
        "{:?}",
        decode_access_rule(wrong_tag, 0)
    );
}

#[test]
fn access_rule_all_keeps_an_unspecified_location() {
    // ALL with the location present but unspecified: Access Point 4194303
    // ((33 << 22) | 0x3FFFFF) in Device 4194303 ((8 << 22) | 0x3FFFFF).
    let rule = BACnetAccessRule {
        time_range_specifier: AccessRuleTimeRangeSpecifier::ALWAYS,
        time_range: None,
        location_specifier: AccessRuleLocationSpecifier::ALL,
        location: Some(BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE)),
            object_identifier: oid(ObjectType::ACCESS_POINT, ObjectIdentifier::MAX_INSTANCE),
        }),
        enable: true,
    };
    round_trip(
        &rule,
        &[
            0x09, 0x01, // time-range-specifier [0] ALWAYS
            0x29, 0x01, // location-specifier [2] ALL
            0x3E, // location [3]
            0x0C, 0x02, 0x3F, 0xFF, 0xFF, // device [0] Device 4194303
            0x1C, 0x08, 0x7F, 0xFF, 0xFF, // object [1] Access Point 4194303
            0x3F, //
            0x49, 0x01, // enable [4] TRUE
        ],
    );
}
