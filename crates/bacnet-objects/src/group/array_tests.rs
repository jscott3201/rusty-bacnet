//! Global Group Group_Members and Present_Value as BACnetARRAYs of their
//! Clause 21 productions (Clauses 12.50.5 and 12.50.7, #1107).
//!
//! The expected octets are written out from the production tags, not
//! produced by the codecs under test.

use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

/// Four members: two local, one indexed in another device, and one the
/// application has no result for in [`configured`].
fn members() -> Vec<BACnetDeviceObjectPropertyReference> {
    vec![
        BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_INPUT, 2),
            PropertyIdentifier::STATUS_FLAGS.to_raw(),
        ),
        BACnetDeviceObjectPropertyReference::new_remote(
            oid(ObjectType::ANALOG_VALUE, 3),
            PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
            oid(ObjectType::DEVICE, 1234),
        )
        .with_index(8),
        BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::BINARY_INPUT, 4),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
    ]
}

/// The members' references: `[0]` object, `[1]` property, and for the third
/// `[2]` index 8 and `[3]` device 1234.
const REFERENCES: [&[u8]; 4] = [
    &[0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55],
    &[0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x6F],
    &[
        0x0C, 0x00, 0x80, 0x00, 0x03, 0x19, 0x57, 0x29, 0x08, 0x3C, 0x02, 0x00, 0x04, 0xD2,
    ],
    &[0x0C, 0x00, 0xC0, 0x00, 0x04, 0x19, 0x55],
];

/// The group with [`members`], names, and results for the first three: a
/// REAL, a Status_Flags bit string with FAULT set, and a failed read.
fn configured() -> GlobalGroupObject {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = members();
    group.group_member_names = ["temp", "flags", "remote", "unread"]
        .map(String::from)
        .to_vec();
    group.present_value = vec![
        AccessResult::Value(PropertyValue::Real(21.5)),
        AccessResult::Value(PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x40],
        }),
        AccessResult::Error {
            class: ErrorClass::COMMUNICATION,
            code: ErrorCode::TIMEOUT,
        },
    ];
    group
}

/// The Present_Value elements of [`configured`]: each reference, then the
/// value inside `[4]` or the error class and code inside `[5]`. The fourth
/// member has no result, so it reads PROPERTY / VALUE_NOT_INITIALIZED.
fn configured_present_value() -> [Vec<u8>; 4] {
    let results: [&[u8]; 4] = [
        &[0x4E, 0x44, 0x41, 0xAC, 0x00, 0x00, 0x4F],
        &[0x4E, 0x82, 0x04, 0x40, 0x4F],
        &[0x5E, 0x91, 0x07, 0x91, 0x1E, 0x5F],
        UNREAD,
    ];
    std::array::from_fn(|i| [REFERENCES[i], results[i]].concat())
}

/// PROPERTY / VALUE_NOT_INITIALIZED inside `[5]`.
const UNREAD: &[u8] = &[0x5E, 0x91, 0x02, 0x91, 0x48, 0x5F];

fn wire(value: &PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    encoded.to_vec()
}

fn read_wire(
    group: &GlobalGroupObject,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Vec<u8> {
    wire(&group.read_property(property, index).unwrap())
}

fn assert_invalid_index(group: &GlobalGroupObject, property: PropertyIdentifier, index: u32) {
    assert!(
        matches!(
            group.read_property(property, Some(index)),
            Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::INVALID_ARRAY_INDEX
        ),
        "{property:?}[{index}]"
    );
}

/// Index 0 is the size, 1..=N one element each, and past N is
/// INVALID_ARRAY_INDEX.
fn assert_indexed(group: &GlobalGroupObject, property: PropertyIdentifier, elements: &[Vec<u8>]) {
    assert_eq!(
        group.read_property(property, Some(0)).unwrap(),
        PropertyValue::Unsigned(elements.len() as u64),
        "{property:?}[0]"
    );
    for (index, element) in (1..).zip(elements) {
        assert_eq!(
            &read_wire(group, property, Some(index)),
            element,
            "{property:?}[{index}]"
        );
    }
    for index in [elements.len() as u32 + 1, u32::MAX] {
        assert_invalid_index(group, property, index);
    }
}

#[test]
fn group_members_go_out_as_device_object_property_references() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = members();
    assert_eq!(
        read_wire(&group, PropertyIdentifier::GROUP_MEMBERS, None),
        REFERENCES.concat()
    );
    let references: Vec<_> = REFERENCES.iter().map(|r| r.to_vec()).collect();
    assert_indexed(&group, PropertyIdentifier::GROUP_MEMBERS, &references);
}

#[test]
fn present_value_holds_each_members_value_or_error() {
    let group = configured();
    assert_eq!(
        read_wire(&group, PropertyIdentifier::PRESENT_VALUE, None),
        configured_present_value().concat()
    );
}

#[test]
fn a_member_not_read_yet_reads_value_not_initialized() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = members();
    let unread: Vec<_> = REFERENCES
        .iter()
        .map(|reference| [*reference, UNREAD].concat())
        .collect();
    assert_eq!(
        read_wire(&group, PropertyIdentifier::PRESENT_VALUE, None),
        unread.concat()
    );
    assert_indexed(&group, PropertyIdentifier::PRESENT_VALUE, &unread);
}

#[test]
fn results_past_the_last_member_are_not_served() {
    let mut group = configured();
    group.group_members.truncate(2);
    group
        .present_value
        .push(AccessResult::Value(PropertyValue::Null));
    let expected = configured_present_value();
    assert_eq!(
        read_wire(&group, PropertyIdentifier::PRESENT_VALUE, None),
        expected[..2].concat()
    );
    assert_indexed(&group, PropertyIdentifier::PRESENT_VALUE, &expected[..2]);
}

#[test]
fn indexed_reads_serve_the_size_and_one_element() {
    let group = configured();
    assert_indexed(
        &group,
        PropertyIdentifier::PRESENT_VALUE,
        &configured_present_value(),
    );
    let names: Vec<_> = ["temp", "flags", "remote", "unread"]
        .iter()
        .map(|name| wire(&PropertyValue::CharacterString((*name).into())))
        .collect();
    assert_indexed(&group, PropertyIdentifier::GROUP_MEMBER_NAMES, &names);
}

#[test]
fn an_empty_group_serves_empty_arrays() {
    let group = GlobalGroupObject::new(1, "GG-1").unwrap();
    for property in [
        PropertyIdentifier::GROUP_MEMBERS,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::GROUP_MEMBER_NAMES,
    ] {
        assert_eq!(
            group.read_property(property, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_indexed(&group, property, &[]);
    }
}
