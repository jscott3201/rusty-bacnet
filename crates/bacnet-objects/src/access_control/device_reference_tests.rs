//! The access-control setters that take BACnetDeviceObjectReference values
//! refuse a device identifier that isn't a Device object (Clause 21, #1285):
//! Access Door Door_Members, Access Point Access_Doors and
//! Access_Event_Credential, and Access Credential Assigned_Access_Rights.
//! A Device identifier, or none, still goes through.

use bacnet_types::constructed::{BACnetAssignedAccessRights, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};

use super::credential_data_input_out_of_service_tests::stamp;
use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// `object` in the device `device` names, which may be of any type.
fn on(device: ObjectIdentifier, object: ObjectIdentifier) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(device),
        object_identifier: object,
    }
}

/// Device identifiers of other object types, the wildcard instance included.
fn not_devices() -> [ObjectIdentifier; 3] {
    [
        oid(ObjectType::ANALOG_VALUE, 9),
        oid(ObjectType::ACCESS_DOOR, 9),
        oid(ObjectType::ACCESS_POINT, ObjectIdentifier::MAX_INSTANCE),
    ]
}

fn assert_value_out_of_range<T: std::fmt::Debug>(result: Result<T, Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32),
        "expected PROPERTY / VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

#[test]
fn access_door_door_members_refuse_a_non_device_device_identifier() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    let device = oid(ObjectType::DEVICE, 9);
    let lock = oid(ObjectType::BINARY_OUTPUT, 1);
    door.set_door_members([lock.into(), on(device, oid(ObjectType::BINARY_INPUT, 2))])
        .unwrap();
    let members = read(&door, P::DOOR_MEMBERS);
    for other in not_devices() {
        // One bad member refuses the whole list and keeps the members.
        assert_value_out_of_range(door.set_door_members([lock.into(), on(other, lock)]));
        assert_eq!(read(&door, P::DOOR_MEMBERS), members);
    }
}

#[test]
fn access_point_access_doors_refuse_a_non_device_device_identifier() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    let device = oid(ObjectType::DEVICE, 9);
    let door = oid(ObjectType::ACCESS_DOOR, 1);
    point
        .set_access_doors([door.into(), on(device, door)])
        .unwrap();
    let doors = read(&point, P::ACCESS_DOORS);
    for other in not_devices() {
        assert_value_out_of_range(point.set_access_doors([door.into(), on(other, door)]));
        assert_eq!(read(&point, P::ACCESS_DOORS), doors);
    }
}

#[test]
fn access_point_event_credential_refuses_a_non_device_device_identifier() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    let card = oid(ObjectType::ACCESS_CREDENTIAL, 3);
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(9)),
            credential: Some(on(oid(ObjectType::DEVICE, 9), card)),
            ..AccessEventReport::new(AccessEvent::GRANTED, 1)
        })
        .unwrap();
    // Device 9, then Access Credential 3.
    let granted = PropertyValue::ApplicationData(vec![
        0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x08, 0x00, 0x00, 0x03,
    ]);
    assert_eq!(read(&point, P::ACCESS_EVENT_CREDENTIAL), granted);
    for other in not_devices() {
        assert_value_out_of_range(point.set_access_event(AccessEventReport {
            time: Some(stamp(10)),
            credential: Some(on(other, card)),
            ..AccessEventReport::new(AccessEvent::DENIED_OTHER, 2)
        }));
        // None of the event's rows moved.
        assert_eq!(read(&point, P::ACCESS_EVENT_CREDENTIAL), granted);
        assert_eq!(
            read(&point, P::ACCESS_EVENT),
            PropertyValue::Enumerated(AccessEvent::GRANTED.to_raw())
        );
        assert_eq!(
            read(&point, P::ACCESS_EVENT_TAG),
            PropertyValue::Unsigned(1)
        );
    }
}

#[test]
fn access_credential_assigned_access_rights_refuse_a_non_device_device_identifier() {
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    let rights = oid(ObjectType::ACCESS_RIGHTS, 4);
    let element = |reference| BACnetAssignedAccessRights {
        assigned_access_rights: reference,
        enable: true,
    };
    credential
        .set_assigned_access_rights(vec![
            element(rights.into()),
            element(on(oid(ObjectType::DEVICE, 9), rights)),
        ])
        .unwrap();
    let assigned = read(&credential, P::ASSIGNED_ACCESS_RIGHTS);
    for other in not_devices() {
        assert_value_out_of_range(
            credential.set_assigned_access_rights(vec![element(on(other, rights))]),
        );
        assert_eq!(read(&credential, P::ASSIGNED_ACCESS_RIGHTS), assigned);
    }
}
