//! The Elevator Group serves the rows of its table (Clause 12.58,
//! Table 12-76; #997): no Status_Flags, Out_Of_Service or Reliability, a
//! read-only Machine_Room_ID, a Group_ID held to Unsigned8, and a
//! Group_Members array that takes an array index (#1034).

use super::super::*;
use super::assert_value_out_of_range;
use bacnet_types::enums::{ErrorClass, ErrorCode, Reliability};

const MACHINE_ROOM_ID: PropertyIdentifier = PropertyIdentifier::MACHINE_ROOM_ID;
const GROUP_ID: PropertyIdentifier = PropertyIdentifier::GROUP_ID;

fn assert_property_error(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn piv(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::POSITIVE_INTEGER_VALUE, instance).unwrap()
}

#[test]
fn elevator_group_has_no_status_flags_out_of_service_or_reliability() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    for (property, value) in [
        (
            PropertyIdentifier::STATUS_FLAGS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            },
        ),
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        ),
    ] {
        assert!(!group.property_list().contains(&property), "{property:?}");
        assert!(!group.is_writable_property(property), "{property:?}");
        assert_property_error(
            group.read_property(property, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_property_error(
            group.write_property(property, None, value, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
}

#[test]
fn elevator_group_machine_room_id_starts_as_the_no_number_reference() {
    let group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let none = piv(ObjectIdentifier::MAX_INSTANCE);
    assert_eq!(group.machine_room_id(), none);
    assert_eq!(
        group.read_property(MACHINE_ROOM_ID, None).unwrap(),
        PropertyValue::ObjectIdentifier(none)
    );
    assert!(group.property_list().contains(&MACHINE_ROOM_ID));
    assert!(group.required_properties().contains(&MACHINE_ROOM_ID));
}

#[test]
fn elevator_group_machine_room_id_is_read_only_over_the_network() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group.set_machine_room_id(piv(7)).unwrap();
    assert!(!group.is_writable_property(MACHINE_ROOM_ID));
    for value in [
        PropertyValue::ObjectIdentifier(piv(8)),
        PropertyValue::Unsigned(8),
    ] {
        assert_property_error(
            group.write_property(MACHINE_ROOM_ID, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(
        group.read_property(MACHINE_ROOM_ID, None).unwrap(),
        PropertyValue::ObjectIdentifier(piv(7))
    );
}

#[test]
fn elevator_group_set_machine_room_id_takes_only_positive_integer_values() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group.set_machine_room_id(piv(3)).unwrap();
    for oid in [
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3).unwrap(),
        ObjectIdentifier::new(ObjectType::INTEGER_VALUE, 3).unwrap(),
    ] {
        assert_value_out_of_range(
            group.set_machine_room_id(oid),
            &format!("{:?}", oid.object_type()),
        );
        assert_eq!(group.machine_room_id(), piv(3));
    }
}

#[test]
fn elevator_group_group_id_accepts_the_unsigned8_range() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    for raw in [0u64, 1, 254, 255] {
        group
            .write_property(GROUP_ID, None, PropertyValue::Unsigned(raw), None)
            .unwrap_or_else(|e| panic!("Group_ID {raw} must be accepted: {e:?}"));
        assert_eq!(
            group.read_property(GROUP_ID, None).unwrap(),
            PropertyValue::Unsigned(raw)
        );
    }
}

#[test]
fn elevator_group_group_id_refuses_values_above_255_atomically() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group
        .write_property(GROUP_ID, None, PropertyValue::Unsigned(47), None)
        .unwrap();
    for raw in [256u64, 65_535, u64::from(u32::MAX), u64::MAX] {
        assert_value_out_of_range(
            group.write_property(GROUP_ID, None, PropertyValue::Unsigned(raw), None),
            &format!("Group_ID {raw}"),
        );
        assert_eq!(
            group.read_property(GROUP_ID, None).unwrap(),
            PropertyValue::Unsigned(47)
        );
    }
}

#[test]
fn elevator_group_group_members_reads_by_array_index() {
    // Group_Members is a BACnetARRAY (Table 12-76; #1034): index 0 is the
    // member count and index n the n-th member.
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let members = PropertyIdentifier::GROUP_MEMBERS;
    assert!(group.is_array_property(members));
    assert_eq!(
        group.read_property(members, Some(0)).unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_property_error(
        group.read_property(members, Some(1)),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    let lift = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
    let escalator = ObjectIdentifier::new(ObjectType::ESCALATOR, 2).unwrap();
    group.add_member(lift);
    group.add_member(escalator);
    for (index, expected) in [
        (0, PropertyValue::Unsigned(2)),
        (1, PropertyValue::ObjectIdentifier(lift)),
        (2, PropertyValue::ObjectIdentifier(escalator)),
    ] {
        assert_eq!(group.read_property(members, Some(index)).unwrap(), expected);
    }
    for index in [3, u32::MAX] {
        assert_property_error(
            group.read_property(members, Some(index)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
    assert_eq!(
        group.read_property(members, None).unwrap(),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(lift),
            PropertyValue::ObjectIdentifier(escalator),
        ])
    );
}
