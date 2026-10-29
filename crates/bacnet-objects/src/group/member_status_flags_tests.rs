//! Global Group Event_State and Member_Status_Flags (Clauses 12.50.9 and
//! 12.50.10, #1092).

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

const IN_ALARM: u8 = 0x80;
const FAULT: u8 = 0x40;
const OVERRIDDEN: u8 = 0x20;
const OUT_OF_SERVICE: u8 = 0x10;

/// A Status_Flags value as a member read would deliver it.
fn flags(octet: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![octet],
    }
}

/// A successful read of a member's Status_Flags, as the group stores it.
fn held(octet: u8) -> AccessResult {
    AccessResult::Value(flags(octet))
}

fn member(instance: u32, property: PropertyIdentifier) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property_identifier: property.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

fn read(group: &GlobalGroupObject, property: PropertyIdentifier) -> PropertyValue {
    group.read_property(property, None).unwrap()
}

fn assert_write_denied(group: &mut GlobalGroupObject, property: PropertyIdentifier) {
    let value = read(group, property);
    assert!(!group.is_writable_property(property));
    assert!(matches!(
        group.write_property(property, None, value.clone(), None),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
    assert_eq!(read(group, property), value);
}

#[test]
fn an_empty_group_reads_normal_and_all_clear() {
    let group = GlobalGroupObject::new(1, "GG-1").unwrap();
    assert_eq!(
        read(&group, PropertyIdentifier::EVENT_STATE),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(0)
    );
    assert_eq!(group.member_status_flags(), StatusFlags::empty());
}

#[test]
fn member_status_flags_is_the_or_of_the_status_flags_members() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = vec![
        member(1, PropertyIdentifier::STATUS_FLAGS),
        member(2, PropertyIdentifier::PRESENT_VALUE),
        member(3, PropertyIdentifier::STATUS_FLAGS),
        member(4, PropertyIdentifier::STATUS_FLAGS),
    ];
    group.present_value = vec![
        held(IN_ALARM),
        // A bit string held for a member that is not Status_Flags is ignored.
        held(OVERRIDDEN),
        held(FAULT | OUT_OF_SERVICE),
        held(0),
    ];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(IN_ALARM | FAULT | OUT_OF_SERVICE)
    );
    assert_eq!(
        group.member_status_flags(),
        StatusFlags::IN_ALARM | StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE
    );
}

#[test]
fn member_status_flags_follows_each_present_value_update() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = vec![
        member(1, PropertyIdentifier::STATUS_FLAGS),
        member(2, PropertyIdentifier::STATUS_FLAGS),
    ];
    group.present_value = vec![held(0), held(OVERRIDDEN)];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(OVERRIDDEN)
    );

    group.present_value[0] = held(FAULT);
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(FAULT | OVERRIDDEN)
    );

    group.present_value[1] = held(0);
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(FAULT)
    );
}

#[test]
fn values_that_are_not_status_flags_bit_strings_add_nothing() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = vec![
        member(1, PropertyIdentifier::STATUS_FLAGS),
        member(2, PropertyIdentifier::STATUS_FLAGS),
        member(3, PropertyIdentifier::STATUS_FLAGS),
    ];
    // The third member has no stored value yet, and a stored value with no
    // member behind it is not a member's Status_Flags.
    group.present_value = vec![
        AccessResult::Value(PropertyValue::Null),
        AccessResult::Value(PropertyValue::Enumerated(1)),
    ];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(0)
    );
    group.group_members.truncate(1);
    group.present_value = vec![held(0), held(IN_ALARM)];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(0)
    );
}

#[test]
fn a_status_flags_member_whose_read_failed_adds_nothing() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = vec![
        member(1, PropertyIdentifier::STATUS_FLAGS),
        member(2, PropertyIdentifier::STATUS_FLAGS),
        member(3, PropertyIdentifier::STATUS_FLAGS),
    ];
    // The second member's read failed and the third has not been read: only
    // the first member's flags count.
    group.present_value = vec![
        held(OVERRIDDEN),
        AccessResult::Error {
            class: ErrorClass::DEVICE,
            code: ErrorCode::UNKNOWN_DEVICE,
        },
    ];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(OVERRIDDEN)
    );
    group.present_value[0] = AccessResult::NOT_INITIALIZED;
    assert_eq!(group.member_status_flags(), StatusFlags::empty());
}

#[test]
fn member_alarms_leave_the_groups_own_event_state_and_status_flags_alone() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    group.group_members = vec![member(1, PropertyIdentifier::STATUS_FLAGS)];
    group.present_value = vec![held(IN_ALARM | FAULT)];
    assert_eq!(
        read(&group, PropertyIdentifier::MEMBER_STATUS_FLAGS),
        flags(IN_ALARM | FAULT)
    );
    // Without intrinsic reporting Event_State stays NORMAL, so the group's
    // own IN_ALARM stays clear; its FAULT follows its own Reliability.
    assert_eq!(
        read(&group, PropertyIdentifier::EVENT_STATE),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(read(&group, PropertyIdentifier::STATUS_FLAGS), flags(0));
}

#[test]
fn event_state_and_member_status_flags_are_read_only_scalars() {
    let mut group = GlobalGroupObject::new(1, "GG-1").unwrap();
    for property in [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::MEMBER_STATUS_FLAGS,
    ] {
        assert!(!group.is_array_property(property));
        assert!(!group.is_list_property(property));
        assert_write_denied(&mut group, property);
    }
}
