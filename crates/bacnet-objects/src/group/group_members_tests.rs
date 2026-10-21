//! Group (type 11) List_Of_Group_Members and Present_Value (#1134).

use super::*;
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn member(
    object_identifier: ObjectIdentifier,
    references: &[(PropertyIdentifier, Option<u32>)],
) -> ReadAccessSpecification {
    ReadAccessSpecification {
        object_identifier,
        list_of_property_references: references
            .iter()
            .map(
                |&(property_identifier, property_array_index)| PropertyReference {
                    property_identifier,
                    property_array_index,
                },
            )
            .collect(),
    }
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

// AI-1 Present_Value and Status_Flags, then AV-2 Priority_Array[16], written
// out from the Clause 21 tags: the object in [0], then the references inside
// [1], each a property in [0] and an optional index in [1].
const MEMBER_1: &[u8] = &[0x0C, 0, 0, 0, 1, 0x1E, 0x09, 85, 0x09, 111, 0x1F];
const MEMBER_2: &[u8] = &[0x0C, 0, 0x80, 0, 2, 0x1E, 0x09, 87, 0x19, 16, 0x1F];

fn configured() -> GroupObject {
    let mut group = GroupObject::new(1, "G-1").unwrap();
    group
        .add_member(member(
            oid(ObjectType::ANALOG_INPUT, 1),
            &[
                (PropertyIdentifier::PRESENT_VALUE, None),
                (PropertyIdentifier::STATUS_FLAGS, None),
            ],
        ))
        .unwrap();
    group
        .add_member(member(
            oid(ObjectType::ANALOG_VALUE, 2),
            &[(PropertyIdentifier::PRIORITY_ARRAY, Some(16))],
        ))
        .unwrap();
    group
}

#[test]
fn list_of_group_members_goes_out_as_read_access_specifications() {
    let group = configured();
    assert_eq!(
        group
            .read_property(PropertyIdentifier::LIST_OF_GROUP_MEMBERS, None)
            .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(MEMBER_1.to_vec()),
            PropertyValue::ApplicationData(MEMBER_2.to_vec()),
        ])
    );
    assert_eq!(group.members().len(), 2);
    assert_eq!(
        group.members()[1].object_identifier,
        oid(ObjectType::ANALOG_VALUE, 2)
    );
}

#[test]
fn group_lists_refuse_an_array_index() {
    let group = configured();
    for property in [
        PropertyIdentifier::LIST_OF_GROUP_MEMBERS,
        PropertyIdentifier::PRESENT_VALUE,
    ] {
        assert!(!group.is_array_property(property));
        assert!(group.is_list_property(property));
        for index in [0, 1, u32::MAX] {
            assert_property_error(
                group.read_property(property, Some(index)).unwrap_err(),
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            );
        }
    }
}

#[test]
fn the_object_alone_serves_an_empty_present_value() {
    // The server rebuilds Present_Value from the members; the object stores
    // none to serve.
    assert_eq!(
        configured()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn add_member_refuses_a_member_that_reports_a_group_present_value() {
    let mut group = configured();
    let before = group.members().to_vec();
    for nested in [ObjectType::GROUP, ObjectType::GLOBAL_GROUP] {
        for selector in [
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::ALL,
            PropertyIdentifier::REQUIRED,
        ] {
            let refused = member(
                oid(nested, 3),
                &[(PropertyIdentifier::OBJECT_NAME, None), (selector, None)],
            );
            let refusal = group.add_member(refused).unwrap_err();
            assert_eq!(refusal, GroupMemberRefusal::NestsGroupPresentValue);
            assert_property_error(refusal.into(), ErrorCode::VALUE_OUT_OF_RANGE);
            assert_eq!(group.members(), before);
        }
    }
}

#[test]
fn add_member_refuses_a_member_without_properties() {
    let mut group = configured();
    let before = group.members().to_vec();
    let refusal = group
        .add_member(member(oid(ObjectType::ANALOG_INPUT, 3), &[]))
        .unwrap_err();
    assert_eq!(refusal, GroupMemberRefusal::NoProperties);
    assert_property_error(refusal.into(), ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(group.members(), before);
}

#[test]
fn add_member_takes_property_identifiers_past_22_bits() {
    // ASHRAE assigns identifiers from 4194304 up (Table 23-1), such as
    // Default_Color, so a member may name any of them (#887).
    let mut group = configured();
    let color = oid(ObjectType::COLOR, 3);
    for property in [
        PropertyIdentifier::from_raw(4_194_303),
        PropertyIdentifier::from_raw(4_194_304),
        PropertyIdentifier::DEFAULT_COLOR,
        PropertyIdentifier::from_raw(u32::MAX),
    ] {
        group
            .add_member(member(color, &[(property, Some(u32::MAX))]))
            .unwrap();
        assert_eq!(
            group.members().last().unwrap().list_of_property_references[0].property_identifier,
            property
        );
    }
}

#[test]
fn add_member_accepts_other_properties_of_a_group() {
    // Only Present_Value is barred: OPTIONAL and named properties of another
    // group, and any object's Present_Value, are ordinary members.
    let mut group = GroupObject::new(1, "G-1").unwrap();
    for accepted in [
        member(
            oid(ObjectType::GROUP, 2),
            &[(PropertyIdentifier::OPTIONAL, None)],
        ),
        member(
            oid(ObjectType::GLOBAL_GROUP, 2),
            &[(PropertyIdentifier::MEMBER_STATUS_FLAGS, None)],
        ),
        member(
            oid(ObjectType::ANALOG_INPUT, 1),
            &[(PropertyIdentifier::ALL, None)],
        ),
    ] {
        group.add_member(accepted).unwrap();
    }
    assert_eq!(group.members().len(), 3);
    group.clear_members();
    assert!(group.members().is_empty());
    assert_eq!(
        group
            .read_property(PropertyIdentifier::LIST_OF_GROUP_MEMBERS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}
