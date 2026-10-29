use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn assert_property_error(error: Error, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code: c })
            if class == ErrorClass::PROPERTY && c == code),
        "expected PROPERTY / {code:?}, got {error:?}"
    );
}

/// AV 7 Present_Value at index 2 in Device 9: every member of the production.
const FULL: [u8; 14] = [
    0x0C, 0x00, 0x80, 0x00, 0x07, // [0] analog-value 7
    0x19, 0x55, // [1] present-value
    0x29, 0x02, // [2] index 2
    0x3C, 0x02, 0x00, 0x00, 0x09, // [3] device 9
];

fn full() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: oid(ObjectType::ANALOG_VALUE, 7),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: Some(2),
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
    }
}

#[test]
fn property_reference_value_is_the_context_tagged_members() {
    assert_eq!(
        reference_value(&full()),
        PropertyValue::ApplicationData(FULL.to_vec())
    );
    let local = BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::ANALOG_VALUE, 7),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    // The optional index and Device members are left out, not sent as Null.
    assert_eq!(
        reference_value(&local),
        PropertyValue::ApplicationData(FULL[..7].to_vec())
    );
}

#[test]
fn object_reference_list_is_one_encoding_per_element() {
    let remote = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(ObjectType::LIFE_SAFETY_ZONE, 3),
    };
    let local = BACnetDeviceObjectReference::from(oid(ObjectType::LIFE_SAFETY_ZONE, 4));
    assert_eq!(
        reference_list(&[remote, local]),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(vec![
                0x0C, 0x02, 0x00, 0x00, 0x09, // [0] device 9
                0x1C, 0x05, 0x80, 0x00, 0x03, // [1] life-safety-zone 3
            ]),
            PropertyValue::ApplicationData(vec![0x1C, 0x05, 0x80, 0x00, 0x04]),
        ])
    );
}

#[test]
fn decode_accepts_raw_bytes_and_the_read_shape() {
    let mut two = FULL.to_vec();
    two.extend_from_slice(&FULL[..7]);
    let expected = vec![
        full(),
        BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_VALUE, 7),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
    ];
    assert_eq!(
        decode_references::<BACnetDeviceObjectPropertyReference>(&PropertyValue::ApplicationData(
            two
        ))
        .unwrap(),
        expected
    );
    let read_shape = PropertyValue::List(vec![
        PropertyValue::ApplicationData(FULL.to_vec()),
        PropertyValue::ApplicationData(FULL[..7].to_vec()),
    ]);
    assert_eq!(
        decode_references::<BACnetDeviceObjectPropertyReference>(&read_shape).unwrap(),
        expected
    );
    assert_eq!(
        decode_references::<BACnetDeviceObjectPropertyReference>(&PropertyValue::List(Vec::new()))
            .unwrap(),
        Vec::new()
    );
    assert_eq!(
        decode_reference::<BACnetDeviceObjectPropertyReference>(&PropertyValue::ApplicationData(
            FULL.to_vec()
        ))
        .unwrap(),
        full()
    );
}

#[test]
fn decode_refuses_other_shapes_with_the_matching_error() {
    for value in [
        PropertyValue::Null,
        PropertyValue::ObjectIdentifier(oid(ObjectType::ANALOG_VALUE, 7)),
        // The flat application-tagged form reads used to serve.
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(oid(ObjectType::ANALOG_VALUE, 7)),
            PropertyValue::Unsigned(85),
        ]),
        // Application-tagged object identifier where [0] belongs.
        PropertyValue::ApplicationData(vec![0xC4, 0x00, 0x80, 0x00, 0x07]),
    ] {
        assert_property_error(
            decode_references::<BACnetDeviceObjectPropertyReference>(&value).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    for bytes in [
        FULL[..5].to_vec(),           // no [1]
        FULL[..12].to_vec(),          // [3] cut short
        vec![0x0C, 0x00, 0x80, 0x00], // [0] cut short
    ] {
        assert_property_error(
            decode_references::<BACnetDeviceObjectPropertyReference>(
                &PropertyValue::ApplicationData(bytes),
            )
            .unwrap_err(),
            ErrorCode::INVALID_DATA_ENCODING,
        );
    }
    let mut trailing = FULL.to_vec();
    trailing.extend_from_slice(&[0x49, 0x01]); // [4] is not a member
    assert_property_error(
        decode_references::<BACnetDeviceObjectPropertyReference>(&PropertyValue::ApplicationData(
            trailing,
        ))
        .unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // A single-reference property holds exactly one: once the value opens as
    // a reference, anything after it is an encoding fault, whatever its tag.
    let mut two = FULL.to_vec();
    two.extend_from_slice(&FULL);
    let after = |trailing: &[u8]| PropertyValue::ApplicationData([&FULL[..], trailing].concat());
    for value in [
        PropertyValue::ApplicationData(two),
        PropertyValue::ApplicationData(Vec::new()),
        PropertyValue::List(Vec::new()),
        after(&[0x49, 0x01]), // [4]
        after(&[0x21, 0x01]), // application Unsigned
        after(&[0x09, 0x01]), // [0], too short to be an object identifier
    ] {
        assert_property_error(
            decode_reference::<BACnetDeviceObjectPropertyReference>(&value).unwrap_err(),
            ErrorCode::INVALID_DATA_ENCODING,
        );
    }
    // A value that doesn't open as a reference is another datatype.
    for value in [
        PropertyValue::Null,
        PropertyValue::ApplicationData(vec![0xC4, 0x00, 0x80, 0x00, 0x07]),
        PropertyValue::ApplicationData(vec![0x19, 0x55]),
    ] {
        assert_property_error(
            decode_reference::<BACnetDeviceObjectPropertyReference>(&value).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // The object reference opens with its Device member [0] or, without one,
    // the object [1]; the same rule holds after it.
    let object = [0x1C, 0x05, 0x80, 0x00, 0x03]; // [1] life-safety-zone 3
    assert!(
        decode_reference::<BACnetDeviceObjectReference>(&PropertyValue::ApplicationData(
            object.to_vec()
        ))
        .is_ok()
    );
    assert_property_error(
        decode_reference::<BACnetDeviceObjectReference>(&PropertyValue::ApplicationData(
            [&object[..], &[0x49, 0x01]].concat(),
        ))
        .unwrap_err(),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_property_error(
        decode_reference::<BACnetDeviceObjectReference>(&PropertyValue::ApplicationData(vec![
            0x29, 0x01,
        ]))
        .unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
}

#[test]
fn device_member_must_be_a_device() {
    check_device_member(None).unwrap();
    check_device_member(Some(oid(ObjectType::DEVICE, 9))).unwrap();
    assert_property_error(
        check_device_member(Some(oid(ObjectType::ANALOG_INPUT, 9))).unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}
