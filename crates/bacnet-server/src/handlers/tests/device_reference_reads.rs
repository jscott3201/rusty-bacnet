//! The reference properties #1182 moved to their Clause 21 forms, as
//! ReadProperty and ReadPropertyMultiple serve them: Event Enrollment's and
//! Averaging's Object_Property_Reference, one
//! BACnetDeviceObjectPropertyReference each, and the Life Safety Member_Of
//! and Zone_Members lists of BACnetDeviceObjectReference. The Access Zone
//! (#1306) and Access User (#1394) lists followed.

use super::*;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::error::RemoteError;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// The value bytes ReadProperty serves for `property`, after checking that
/// ReadPropertyMultiple serves the same ones.
fn read(db: &ObjectDatabase, object: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack).unwrap();
    let value = ReadPropertyACK::decode(&ack).unwrap().property_value;

    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: object,
            list_of_property_references: vec![PropertyReference {
                property_identifier: property,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut ack = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut ack).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&ack).unwrap();
    assert_eq!(
        ack.list_of_read_access_results[0].list_of_results[0].property_value,
        Some(value.clone())
    );
    value
}

#[test]
fn event_enrollment_object_property_reference_is_served_framed() {
    let mut db = ObjectDatabase::new();
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    let ee = enrollment.object_identifier();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: oid(ObjectType::ANALOG_INPUT, 5),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: Some(1),
            device_identifier: Some(oid(ObjectType::DEVICE, 260)),
        }))
        .unwrap();
    db.add(Box::new(enrollment)).unwrap();
    assert_eq!(
        read(&db, ee, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE),
        [
            0x0C, 0x00, 0x00, 0x00, 0x05, // [0] analog-input 5
            0x19, 0x55, // [1] present-value
            0x29, 0x01, // [2] index 1
            0x3C, 0x02, 0x00, 0x01, 0x04, // [3] device 260
        ]
    );
    // Table 12-14 codes the property R, and it stays read-only here.
    let served = read(&db, ee, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE);
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ee,
        property_identifier: PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        property_array_index: None,
        property_value: served.clone(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert!(matches!(
        sourced_wp(&mut db, &request),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
    assert_eq!(
        read(&db, ee, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE),
        served
    );
}

#[test]
fn averaging_object_property_reference_is_served_framed() {
    let mut db = ObjectDatabase::new();
    let mut averaging = AveragingObject::new(1, "AVG-1").unwrap();
    let avg = averaging.object_identifier();
    averaging.set_object_property_reference(Some(BACnetObjectPropertyReference::new_indexed(
        oid(ObjectType::ANALOG_VALUE, 3),
        PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
        8,
    )));
    db.add(Box::new(averaging)).unwrap();
    assert_eq!(
        read(&db, avg, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE),
        [
            0x0C, 0x00, 0x80, 0x00, 0x03, // [0] analog-value 3
            0x19, 0x57, // [1] priority-array
            0x29, 0x08, // [2] slot 8
        ]
    );
}

#[test]
fn life_safety_member_lists_are_served_framed() {
    let mut db = ObjectDatabase::new();
    let remote_zone = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(ObjectType::LIFE_SAFETY_ZONE, 4),
    };
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point
        .add_member(oid(ObjectType::LIFE_SAFETY_ZONE, 2))
        .unwrap();
    point.add_member(remote_zone.clone()).unwrap();
    let lsp = point.object_identifier();
    let mut zone = LifeSafetyZoneObject::new(2, "LSZ-2").unwrap();
    zone.add_zone_member(lsp).unwrap();
    zone.add_member(remote_zone).unwrap();
    let lsz = zone.object_identifier();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();

    // [1] alone for a local member, [0] then [1] for one in Device 9.
    let remote = [0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x05, 0x80, 0x00, 0x04];
    assert_eq!(
        read(&db, lsp, PropertyIdentifier::MEMBER_OF),
        [&[0x1C, 0x05, 0x80, 0x00, 0x02][..], &remote].concat()
    );
    assert_eq!(
        read(&db, lsz, PropertyIdentifier::ZONE_MEMBERS),
        [0x1C, 0x05, 0x40, 0x00, 0x01]
    );
    assert_eq!(read(&db, lsz, PropertyIdentifier::MEMBER_OF), remote);
}

#[test]
fn access_zone_entry_and_exit_points_are_served_framed() {
    use bacnet_objects::access_control::AccessZoneObject;

    let mut db = ObjectDatabase::new();
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_entry_points([
        BACnetDeviceObjectReference::from(oid(ObjectType::ACCESS_POINT, 1)),
        BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 9)),
            object_identifier: oid(ObjectType::ACCESS_POINT, 4),
        },
    ])
    .unwrap();
    zone.set_exit_points([oid(ObjectType::ACCESS_POINT, 2)])
        .unwrap();
    let az = zone.object_identifier();
    db.add(Box::new(zone)).unwrap();

    // Access Point 1 here, [1] alone; Access Point 4 in Device 9, [0] then
    // [1] (#1306).
    let entry = read(&db, az, PropertyIdentifier::ENTRY_POINTS);
    assert_eq!(
        entry,
        [
            0x1C, 0x08, 0x40, 0x00, 0x01, // [1] access-point 1
            0x0C, 0x02, 0x00, 0x00, 0x09, // [0] device 9
            0x1C, 0x08, 0x40, 0x00, 0x04, // [1] access-point 4
        ]
    );
    assert_eq!(
        read(&db, az, PropertyIdentifier::EXIT_POINTS),
        [0x1C, 0x08, 0x40, 0x00, 0x02]
    );
    // Table 12-37 codes both R: a client's write of the served bytes is
    // refused and changes nothing.
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: az,
        property_identifier: PropertyIdentifier::ENTRY_POINTS,
        property_array_index: None,
        property_value: entry.clone(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert!(matches!(
        sourced_wp(&mut db, &request),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
    assert_eq!(read(&db, az, PropertyIdentifier::ENTRY_POINTS), entry);
}

#[test]
fn access_user_reference_lists_are_served_framed() {
    use bacnet_objects::access_control::AccessUserObject;

    let mut db = ObjectDatabase::new();
    let mut user = AccessUserObject::new(1, "USER-1").unwrap();
    user.set_credentials([
        BACnetDeviceObjectReference::from(oid(ObjectType::ACCESS_CREDENTIAL, 1)),
        BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 9)),
            object_identifier: oid(ObjectType::ACCESS_CREDENTIAL, 4),
        },
    ])
    .unwrap();
    user.set_members([oid(ObjectType::ACCESS_USER, 2)]).unwrap();
    user.set_member_of([BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(ObjectType::ACCESS_USER, 5),
    }])
    .unwrap();
    let au = user.object_identifier();
    db.add(Box::new(user)).unwrap();

    // Access Credential 1 here, [1] alone; Access Credential 4 in Device 9,
    // [0] then [1] (#1394). A bare application-tagged identifier couldn't
    // carry the device.
    let credentials = read(&db, au, PropertyIdentifier::CREDENTIALS);
    assert_eq!(
        credentials,
        [
            0x1C, 0x08, 0x00, 0x00, 0x01, // [1] access-credential 1
            0x0C, 0x02, 0x00, 0x00, 0x09, // [0] device 9
            0x1C, 0x08, 0x00, 0x00, 0x04, // [1] access-credential 4
        ]
    );
    assert_eq!(
        read(&db, au, PropertyIdentifier::MEMBERS),
        [0x1C, 0x08, 0xC0, 0x00, 0x02] // [1] access-user 2
    );
    assert_eq!(
        read(&db, au, PropertyIdentifier::MEMBER_OF),
        [
            0x0C, 0x02, 0x00, 0x00, 0x09, // [0] device 9
            0x1C, 0x08, 0xC0, 0x00, 0x05, // [1] access-user 5
        ]
    );
    // Credentials is a BACnetLIST, so an index is refused as on the other
    // lists, and a client's write of the served bytes changes nothing.
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: au,
        property_identifier: PropertyIdentifier::CREDENTIALS,
        property_array_index: Some(1),
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    assert!(matches!(
        handle_read_property(&db, &request, &mut ack),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
    ));
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: au,
        property_identifier: PropertyIdentifier::CREDENTIALS,
        property_array_index: None,
        property_value: credentials.clone(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert!(matches!(
        sourced_wp(&mut db, &request),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
    assert_eq!(read(&db, au, PropertyIdentifier::CREDENTIALS), credentials);
}
