use super::*;
use bacnet_types::enums::PropertyIdentifier;

const DATE_TIME: &[u8] = &[0xa4, 0x7c, 0x06, 0x0f, 0x03, 0xb4, 0x0a, 0x1e, 0x00, 0x00];
const TIMER_REQUIRED_BODY: &[u8] = &[
    0xfe, 0x16, 0x09, 0x01, 0x1a, 0x04, 0x80, 0x2e, 0xa4, 0x7c, 0x06, 0x0f, 0x03, 0xb4, 0x0a, 0x1e,
    0x00, 0x00, 0x2f,
];
const LOCAL_ACCESS: &[u8] = &[
    0xde, 0x09, 0x05, 0x1a, 0x04, 0x80, 0x29, 0x0a, 0x3e, 0x2e, 0xa4, 0x7c, 0x06, 0x0f, 0x03, 0xb4,
    0x0a, 0x1e, 0x00, 0x00, 0x2f, 0x3f, 0x4e, 0x1c, 0x08, 0x00, 0x00, 0x01, 0x4f, 0xdf,
];
const REMOTE_ACCESS_WITH_FACTOR: &[u8] = &[
    0xde, 0x09, 0x05, 0x1a, 0x04, 0x80, 0x29, 0x0a, 0x3e, 0x2e, 0xa4, 0x7c, 0x06, 0x0f, 0x03, 0xb4,
    0x0a, 0x1e, 0x00, 0x00, 0x2f, 0x3f, 0x4e, 0x0c, 0x02, 0x00, 0x00, 0x02, 0x1c, 0x08, 0x00, 0x00,
    0x01, 0x4f, 0x5e, 0x09, 0x01, 0x19, 0x02, 0x2a, 0xab, 0xcd, 0x5f, 0xdf,
];

fn date_time() -> (Date, Time) {
    (
        Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 3,
        },
        Time {
            hour: 10,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
    )
}

fn property_value(
    property_identifier: PropertyIdentifier,
    property_array_index: Option<u32>,
    value: &[u8],
    priority: Option<u8>,
) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier,
        property_array_index,
        value: value.to_vec(),
        priority,
    }
}

fn exact_round_trip(expected: NotificationParameters, literal: &[u8]) {
    let mut encoded = BytesMut::new();
    expected.encode(&mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), literal);
    assert_eq!(
        NotificationParameters::decode(literal, 0).unwrap(),
        expected
    );
}

fn access_credential(remote: bool) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: remote.then(|| ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap(),
    }
}

fn access_event(remote: bool, authentication_factor: Option<Vec<u8>>) -> NotificationParameters {
    NotificationParameters::AccessEvent {
        access_event: 5,
        status_flags: 0b1000,
        access_event_tag: 10,
        access_event_time: date_time(),
        access_credential: access_credential(remote),
        authentication_factor,
    }
}

fn raw_access_event(credential_fields: &[u8], authentication_factor: Option<&[u8]>) -> Vec<u8> {
    let mut wire = vec![
        0xde, 0x09, 0x05, 0x1a, 0x04, 0x80, 0x29, 0x0a, 0x3e, 0x2e, 0xa4, 0x7c, 0x06, 0x0f, 0x03,
        0xb4, 0x0a, 0x1e, 0x00, 0x00, 0x2f, 0x3f, 0x4e,
    ];
    wire.extend_from_slice(credential_fields);
    wire.push(0x4f);
    if let Some(authentication_factor) = authentication_factor {
        wire.push(0x5e);
        wire.extend_from_slice(authentication_factor);
        wire.push(0x5f);
    }
    wire.push(0xdf);
    wire
}

fn timer(mask: u8) -> NotificationParameters {
    NotificationParameters::ChangeOfTimer {
        new_state: 1,
        status_flags: 0b1000,
        update_time: date_time(),
        last_state_change: (mask & 1 != 0).then_some(2),
        initial_timeout: (mask & 2 != 0).then_some(3),
        expiration_time: (mask & 4 != 0).then_some(date_time()),
    }
}

fn literal_timer(mask: u8) -> Vec<u8> {
    let mut wire = TIMER_REQUIRED_BODY.to_vec();
    if mask & 1 != 0 {
        wire.extend_from_slice(&[0x39, 0x02]);
    }
    if mask & 2 != 0 {
        wire.extend_from_slice(&[0x49, 0x03]);
    }
    if mask & 4 != 0 {
        wire.push(0x5e);
        wire.extend_from_slice(DATE_TIME);
        wire.push(0x5f);
    }
    wire.extend_from_slice(&[0xff, 0x16]);
    wire
}

#[test]
fn choice_20_is_rejected() {
    let encoded = [0xfe, 0x14, 0xff, 0x14];
    assert!(NotificationParameters::decode(&encoded, 0).is_err());
}

#[test]
fn complex_event_type_literal_vectors_round_trip() {
    exact_round_trip(
        NotificationParameters::ComplexEventType {
            property_values: Vec::new(),
        },
        &[0x6e, 0x6f],
    );

    exact_round_trip(
        NotificationParameters::ComplexEventType {
            property_values: vec![property_value(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                &[0x00],
                None,
            )],
        },
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x6f],
    );

    exact_round_trip(
        NotificationParameters::ComplexEventType {
            property_values: vec![
                property_value(PropertyIdentifier::PRESENT_VALUE, None, &[0x00], None),
                property_value(
                    PropertyIdentifier::STATUS_FLAGS,
                    Some(2),
                    &[0x21, 0x07],
                    Some(16),
                ),
            ],
        },
        &[
            0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x09, 0x6f, 0x19, 0x02, 0x2e, 0x21, 0x07, 0x2f,
            0x39, 0x10, 0x6f,
        ],
    );
}

#[test]
fn complex_event_type_preserves_raw_property_values() {
    let expected = NotificationParameters::ComplexEventType {
        property_values: vec![property_value(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &[0x6e, 0x00, 0x6f],
            None,
        )],
    };
    exact_round_trip(
        expected,
        &[0x6e, 0x09, 0x55, 0x2e, 0x6e, 0x00, 0x6f, 0x2f, 0x6f],
    );
}

#[test]
fn complex_event_type_rejects_malformed_property_fields() {
    let malformed = [
        &[0x6e, 0x09, 0x55, 0x09, 0x6f, 0x2e, 0x00, 0x2f, 0x6f][..],
        &[
            0x6e, 0x09, 0x55, 0x19, 0x01, 0x19, 0x02, 0x2e, 0x00, 0x2f, 0x6f,
        ],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x2e, 0x00, 0x2f, 0x6f],
        &[
            0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x39, 0x01, 0x39, 0x02, 0x6f,
        ],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x39, 0x00, 0x6f],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x39, 0x11, 0x6f],
        &[0x6e, 0x09, 0x55, 0x49, 0x01, 0x6f],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x49, 0x01, 0x6f],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x6f],
        &[0x6e, 0x09, 0x55, 0x2e, 0x00, 0x2f, 0x5f],
    ];
    for wire in malformed {
        assert!(
            NotificationParameters::decode(wire, 0).is_err(),
            "{wire:02x?}"
        );
    }
}

#[test]
fn complex_event_type_enforces_count_cap_on_encode_and_decode() {
    let value = property_value(PropertyIdentifier::PRESENT_VALUE, None, &[0x00], None);
    let accepted = NotificationParameters::ComplexEventType {
        property_values: vec![value.clone(); MAX_DECODED_ITEMS],
    };
    let mut encoded = BytesMut::new();
    accepted.encode(&mut encoded).unwrap();
    let NotificationParameters::ComplexEventType { property_values } =
        NotificationParameters::decode(&encoded, 0).unwrap()
    else {
        panic!("expected ComplexEventType");
    };
    assert_eq!(property_values.len(), MAX_DECODED_ITEMS);

    let over_cap = NotificationParameters::ComplexEventType {
        property_values: vec![value; MAX_DECODED_ITEMS + 1],
    };
    let mut untouched = BytesMut::from(&[0xaa, 0xbb][..]);
    assert!(over_cap.encode(&mut untouched).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa, 0xbb]);

    let mut literal = BytesMut::with_capacity((MAX_DECODED_ITEMS + 1) * 6 + 2);
    literal.extend_from_slice(&[0x6e]);
    for _ in 0..=MAX_DECODED_ITEMS {
        literal.extend_from_slice(&[0x09, 0x55, 0x2e, 0x00, 0x2f]);
    }
    literal.extend_from_slice(&[0x6f]);
    assert!(NotificationParameters::decode(&literal, 0).is_err());
}

#[test]
fn complex_event_type_failed_encodes_are_atomic() {
    for property_value in [
        property_value(PropertyIdentifier::PRESENT_VALUE, None, &[0x9f], None),
        property_value(PropertyIdentifier::PRESENT_VALUE, None, &[0x00], Some(17)),
    ] {
        let invalid = NotificationParameters::ComplexEventType {
            property_values: vec![property_value],
        };
        let mut untouched = BytesMut::from(&[0xaa, 0xbb][..]);
        assert!(invalid.encode(&mut untouched).is_err());
        assert_eq!(untouched.as_ref(), &[0xaa, 0xbb]);
    }
}

#[test]
fn complex_event_type_enforces_total_nesting() {
    let nested_value = |depth| {
        let mut value = vec![0x0e; depth];
        value.extend(vec![0x0f; depth]);
        NotificationParameters::ComplexEventType {
            property_values: vec![property_value(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                &value,
                None,
            )],
        }
    };

    let accepted = nested_value(tags::MAX_CONTEXT_NESTING_DEPTH - 2);
    let mut encoded = BytesMut::new();
    accepted.encode(&mut encoded).unwrap();
    assert_eq!(
        NotificationParameters::decode(&encoded, 0).unwrap(),
        accepted
    );

    let too_deep = nested_value(tags::MAX_CONTEXT_NESTING_DEPTH - 1);
    let mut untouched = BytesMut::from(&[0xaa, 0xbb][..]);
    assert!(too_deep.encode(&mut untouched).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa, 0xbb]);

    let mut raw = BytesMut::new();
    tags::encode_opening_tag(&mut raw, 6);
    primitives::encode_ctx_unsigned(&mut raw, 0, 85);
    tags::encode_opening_tag(&mut raw, 2);
    for _ in 0..tags::MAX_CONTEXT_NESTING_DEPTH - 1 {
        tags::encode_opening_tag(&mut raw, 0);
    }
    for _ in 0..tags::MAX_CONTEXT_NESTING_DEPTH - 1 {
        tags::encode_closing_tag(&mut raw, 0);
    }
    tags::encode_closing_tag(&mut raw, 2);
    tags::encode_closing_tag(&mut raw, 6);
    assert!(NotificationParameters::decode(&raw, 0).is_err());
}

#[test]
fn access_event_literal_vectors_round_trip() {
    exact_round_trip(access_event(false, None), LOCAL_ACCESS);
    exact_round_trip(
        access_event(true, Some(vec![0x09, 0x01, 0x19, 0x02, 0x2a, 0xab, 0xcd])),
        REMOTE_ACCESS_WITH_FACTOR,
    );

    let empty_value = access_event(false, Some(vec![0x09, 0x01, 0x19, 0x02, 0x28]));
    let mut empty_literal = LOCAL_ACCESS[..LOCAL_ACCESS.len() - 1].to_vec();
    empty_literal.extend_from_slice(&[0x5e, 0x09, 0x01, 0x19, 0x02, 0x28, 0x5f, 0xdf]);
    exact_round_trip(empty_value, &empty_literal);
}

#[test]
fn access_event_rejects_invalid_device_object_references() {
    let object = [0x1c, 0x08, 0x00, 0x00, 0x01];
    let device = [0x0c, 0x02, 0x00, 0x00, 0x02];
    let malformed = [
        &[][..],
        &[0x0c, 0x08, 0x00, 0x00, 0x01, 0x19, 0x55],
        &[object.as_slice(), object.as_slice()].concat(),
        &[device.as_slice(), device.as_slice(), object.as_slice()].concat(),
        &[object.as_slice(), device.as_slice()].concat(),
        &[object.as_slice(), &[0x29, 0x01]].concat(),
    ];
    for credential_fields in malformed {
        let wire = raw_access_event(credential_fields, None);
        assert!(NotificationParameters::decode(&wire, 0).is_err());
    }
}

#[test]
fn access_event_rejects_malformed_authentication_factors_atomically() {
    let malformed: &[&[u8]] = &[
        &[],
        &[0x19, 0x02, 0x28],
        &[0x09, 0x01, 0x28],
        &[0x09, 0x01, 0x19, 0x02],
        &[0x09, 0x01, 0x09, 0x02, 0x19, 0x02, 0x28],
        &[0x09, 0x01, 0x19, 0x02, 0x19, 0x03, 0x28],
        &[0x09, 0x01, 0x19, 0x02, 0x28, 0x28],
        &[0x19, 0x02, 0x09, 0x01, 0x28],
        &[0x09, 0x01, 0x28, 0x19, 0x02],
        &[0x91, 0x01, 0x19, 0x02, 0x28],
        &[0x09, 0x01, 0x21, 0x02, 0x28],
        &[0x09, 0x01, 0x19, 0x02, 0x60],
        &[0x09, 0x01, 0x29, 0x02, 0x28],
        &[0x0a, 0x01],
        &[0x09, 0x01, 0x19, 0x02, 0x2a, 0xab],
        &[0x0d, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x19, 0x02, 0x28],
        &[0x09, 0x01, 0x18, 0x28],
        &[0x09, 0x01, 0x19, 0x02, 0x28, 0x39, 0x01],
    ];
    let credential = [0x1c, 0x08, 0x00, 0x00, 0x01];
    for factor in malformed {
        let wire = raw_access_event(&credential, Some(factor));
        assert!(
            NotificationParameters::decode(&wire, 0).is_err(),
            "{factor:02x?}"
        );

        let invalid = access_event(false, Some(factor.to_vec()));
        let mut untouched = BytesMut::from(&[0xaa, 0xbb][..]);
        assert!(invalid.encode(&mut untouched).is_err(), "{factor:02x?}");
        assert_eq!(untouched.as_ref(), &[0xaa, 0xbb]);
    }
}

#[test]
fn change_of_timer_all_optional_combinations_have_exact_wire_bytes() {
    for mask in 0..8 {
        exact_round_trip(timer(mask), &literal_timer(mask));
    }
}

#[test]
fn change_of_timer_rejects_noncanonical_optional_fields() {
    let mut malformed = Vec::new();
    for suffix in [
        &[0x39, 0x02, 0x39, 0x03][..],
        &[0x49, 0x03, 0x39, 0x02],
        &[0x91, 0x02],
        &[0x3e, 0x3f],
        &[0x49, 0x03, 0x49, 0x04],
        &[0x69, 0x01],
    ] {
        let mut wire = TIMER_REQUIRED_BODY.to_vec();
        wire.extend_from_slice(suffix);
        wire.extend_from_slice(&[0xff, 0x16]);
        malformed.push(wire);
    }

    let mut reversed_date_time = TIMER_REQUIRED_BODY.to_vec();
    reversed_date_time.extend_from_slice(&[
        0x5e, 0xb4, 0x0a, 0x1e, 0x00, 0x00, 0xa4, 0x7c, 0x06, 0x0f, 0x03, 0x5f, 0xff, 0x16,
    ]);
    malformed.push(reversed_date_time);

    let mut wrong_date_length = TIMER_REQUIRED_BODY.to_vec();
    wrong_date_length.extend_from_slice(&[
        0x5e, 0xa3, 0x7c, 0x06, 0x0f, 0xb4, 0x0a, 0x1e, 0x00, 0x00, 0x5f, 0xff, 0x16,
    ]);
    malformed.push(wrong_date_length);

    let mut duplicate_expiration = TIMER_REQUIRED_BODY.to_vec();
    for _ in 0..2 {
        duplicate_expiration.push(0x5e);
        duplicate_expiration.extend_from_slice(DATE_TIME);
        duplicate_expiration.push(0x5f);
    }
    duplicate_expiration.extend_from_slice(&[0xff, 0x16]);
    malformed.push(duplicate_expiration);

    let mut wrong_close = literal_timer(0);
    *wrong_close.last_mut().unwrap() = 0x15;
    malformed.push(wrong_close);

    for wire in malformed {
        assert!(
            NotificationParameters::decode(&wire, 0).is_err(),
            "{wire:02x?}"
        );
    }
}

fn event_request(event_values: Option<NotificationParameters>) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 6,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values,
    }
}

#[test]
fn corrected_variants_preserve_event_values_framing() {
    let corrected = [
        NotificationParameters::ComplexEventType {
            property_values: vec![property_value(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                &[0x00],
                None,
            )],
        },
        access_event(true, Some(vec![0x09, 0x01, 0x19, 0x02, 0x2a, 0xab, 0xcd])),
        timer(5),
    ];

    for expected in corrected {
        let mut direct = BytesMut::new();
        expected.encode(&mut direct).unwrap();
        assert_eq!(
            NotificationParameters::decode(&direct, 0).unwrap(),
            expected
        );

        let mut wrapped = BytesMut::from(&[0xce][..]);
        wrapped.extend_from_slice(&direct);
        wrapped.extend_from_slice(&[0xcf]);
        assert_eq!(
            NotificationParameters::decode(&wrapped, 1).unwrap(),
            expected
        );

        let mut service = BytesMut::new();
        event_request(Some(expected.clone()))
            .encode(&mut service)
            .unwrap();
        assert_eq!(
            EventNotificationRequest::decode(&service)
                .unwrap()
                .event_values,
            Some(expected)
        );
    }

    let mut without_values = BytesMut::new();
    event_request(None).encode(&mut without_values).unwrap();
    assert!(EventNotificationRequest::decode(&without_values)
        .unwrap()
        .event_values
        .is_none());
}
