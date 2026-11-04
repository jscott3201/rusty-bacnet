//! Each service production that carries a property value, with
//! Event_Parameters values in the pre-framing `[255]` form (#2376): what its
//! encoder writes decodes back to the same values, with two such values back
//! to back wherever the production has a list, and its encoder refuses a
//! value that wouldn't, leaving the buffer as it was.

use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::cov::COVNotificationRequest;
use crate::object_mgmt::{CreateObjectRequest, ObjectSpecifier};
use crate::read_property::ReadPropertyACK;
use crate::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use crate::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use crate::write_property::WritePropertyRequest;

const EVENT_PARAMETERS: PropertyIdentifier = PropertyIdentifier::EVENT_PARAMETERS;

fn enrollment(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance).unwrap()
}

/// Legacy values every production takes: the shortest, one with payload,
/// and one whose inner pair is followed by another tag's closing octet.
const TAKEN: [&[u8]; 3] = [
    &[0xfe, 0xff, 0xff, 0xff],
    &[0xfe, 0xff, 1, 0xff, 0xff],
    &[0xfe, 0xff, 3, 0xff, 0xff, 0x6f, 0xff, 0xff],
];

/// Legacy values a production whose value closes with `close` refuses:
/// cut short, without its closing pair, and with the pair and `close`
/// before its end.
fn refused(close: u8) -> [Vec<u8>; 3] {
    [
        vec![0xfe, 0xff],
        vec![0xfe, 0xff, 1, 2],
        vec![0xfe, 0xff, 1, 0xff, 0xff, close, 2, 0xff, 0xff],
    ]
}

fn legacy(value: &[u8]) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: EVENT_PARAMETERS,
        property_array_index: None,
        value: value.to_vec(),
        priority: None,
    }
}

fn with_priority(value: &[u8], priority: u8) -> BACnetPropertyValue {
    BACnetPropertyValue {
        priority: Some(priority),
        ..legacy(value)
    }
}

/// An Octet String ending in `ff ff`, as a following value.
fn octet_string() -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        value: vec![0x62, 0xff, 0xff],
        priority: None,
    }
}

/// Property values for the list productions: two legacy values back to
/// back, the second with a priority, then the Octet String.
fn listed() -> Vec<BACnetPropertyValue> {
    let mut values: Vec<_> = TAKEN.iter().map(|value| legacy(value)).collect();
    values[1].priority = Some(8);
    values.push(octet_string());
    values
}

/// `encode` into a buffer holding one octet already: whether it took the
/// value, with the buffer left as it was when it didn't.
fn encodes(encode: impl FnOnce(&mut BytesMut) -> Result<(), bacnet_types::error::Error>) -> bool {
    let mut buf = BytesMut::from(&[0xaa][..]);
    let taken = encode(&mut buf).is_ok();
    if !taken {
        assert_eq!(buf.as_ref(), [0xaa]);
    }
    taken
}

#[test]
fn read_property_ack() {
    let ack = |value: &[u8]| ReadPropertyACK {
        object_identifier: enrollment(1),
        property_identifier: EVENT_PARAMETERS,
        property_array_index: None,
        property_value: value.to_vec(),
    };
    for value in TAKEN {
        let mut buf = BytesMut::new();
        ack(value).encode(&mut buf).unwrap();
        assert_eq!(ReadPropertyACK::decode(&buf).unwrap(), ack(value));
    }
    for value in refused(0x3f) {
        assert!(!encodes(|buf| ack(&value).encode(buf)), "{value:02x?}");
    }
}

#[test]
fn write_property() {
    let request = |value: &[u8], priority| WritePropertyRequest {
        object_identifier: enrollment(1),
        property_identifier: EVENT_PARAMETERS,
        property_array_index: Some(0),
        property_value: value.to_vec(),
        priority,
    };
    for value in TAKEN {
        for priority in [None, Some(8)] {
            let mut buf = BytesMut::new();
            request(value, priority).encode(&mut buf).unwrap();
            assert_eq!(
                WritePropertyRequest::decode(&buf).unwrap(),
                request(value, priority)
            );
        }
    }
    for value in refused(0x3f) {
        assert!(
            !encodes(|buf| request(&value, None).encode(buf)),
            "{value:02x?}"
        );
    }
}

fn result_element(value: &BACnetPropertyValue) -> ReadResultElement {
    ReadResultElement {
        property_identifier: value.property_identifier,
        property_array_index: value.property_array_index,
        property_value: Some(value.value.clone()),
        error: None,
    }
}

#[test]
fn read_property_multiple_ack() {
    let ack = |last: &[u8]| ReadPropertyMultipleACK {
        list_of_read_access_results: vec![
            ReadAccessResult {
                object_identifier: enrollment(1),
                list_of_results: listed().iter().map(result_element).collect(),
            },
            ReadAccessResult {
                object_identifier: enrollment(2),
                list_of_results: vec![
                    result_element(&legacy(TAKEN[1])),
                    result_element(&legacy(last)),
                ],
            },
        ],
    };
    for last in TAKEN {
        let mut buf = BytesMut::new();
        ack(last).encode(&mut buf).unwrap();
        assert_eq!(ReadPropertyMultipleACK::decode(&buf).unwrap(), ack(last));
    }
    // A refused value in the last result leaves out the results before it
    // too.
    for value in refused(0x4f) {
        assert!(!encodes(|buf| ack(&value).encode(buf)), "{value:02x?}");
        let result = &ack(&value).list_of_read_access_results[1];
        assert!(!encodes(|buf| result.encode(buf)));
        assert!(!encodes(|buf| result.list_of_results[1].encode(buf)));
    }
}

#[test]
fn write_property_multiple() {
    let request = |last: BACnetPropertyValue| WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            WriteAccessSpecification {
                object_identifier: enrollment(1),
                list_of_properties: listed(),
            },
            WriteAccessSpecification {
                object_identifier: enrollment(2),
                list_of_properties: vec![legacy(TAKEN[1]), last],
            },
        ],
    };
    for last in [
        legacy(TAKEN[0]),
        with_priority(TAKEN[2], 16),
        octet_string(),
    ] {
        let mut buf = BytesMut::new();
        request(last.clone()).encode(&mut buf).unwrap();
        assert_eq!(
            WritePropertyMultipleRequest::decode(&buf).unwrap(),
            request(last)
        );
    }
    for value in refused(0x2f) {
        assert!(
            !encodes(|buf| request(legacy(&value)).encode(buf)),
            "{value:02x?}"
        );
    }
}

#[test]
fn create_object() {
    let request = |values| CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(enrollment(1)),
        list_of_initial_values: values,
    };
    let mut buf = BytesMut::new();
    request(listed()).encode(&mut buf).unwrap();
    assert_eq!(
        CreateObjectRequest::decode(&buf).unwrap(),
        request(listed())
    );
    for value in refused(0x2f) {
        let mut values = listed();
        values.push(legacy(&value));
        assert!(!encodes(|buf| request(values).encode(buf)), "{value:02x?}");
    }
}

#[test]
fn cov_notification() {
    let notification = |values| COVNotificationRequest {
        subscriber_process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        monitored_object_identifier: enrollment(1),
        time_remaining: 60,
        list_of_values: values,
    };
    let mut buf = BytesMut::new();
    notification(listed()).encode(&mut buf).unwrap();
    assert_eq!(
        COVNotificationRequest::decode(&buf).unwrap(),
        notification(listed())
    );
    for value in refused(0x2f) {
        let mut values = listed();
        values.push(legacy(&value));
        assert!(
            !encodes(|buf| notification(values).encode(buf)),
            "{value:02x?}"
        );
    }
}
