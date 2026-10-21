//! The synchronous WriteProperty and WritePropertyMultiple handlers end a
//! Channel distribution they can't make, so the Channel is never left
//! IN_PROGRESS (#1151, #1178; Clause 12.53).
//!
//! CH-1 writes AO-1's Present_Value; CH-2 has no members.
use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, WriteStatus};

fn ch(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, instance).unwrap()
}

fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            ao1(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )])
        .unwrap();
    db.add(Box::new(channel)).unwrap();
    db.add(Box::new(ChannelObject::new(2, "CH-2", 8).unwrap()))
        .unwrap();
    db
}

fn real(value: f32) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Real(value)).unwrap();
    encoded.to_vec()
}

fn write_pv(db: &mut ObjectDatabase, instance: u32, value: f32) -> ObjectIdentifier {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ch(instance),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: real(value),
        priority: Some(8),
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).unwrap()
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

fn write_status(db: &ObjectDatabase, instance: u32) -> WriteStatus {
    match read(db, ch(instance), PropertyIdentifier::WRITE_STATUS) {
        PropertyValue::Enumerated(raw) => WriteStatus::from_raw(raw),
        other => panic!("Write_Status read {other:?}"),
    }
}

fn ended_unmade(db: &ObjectDatabase) {
    assert_eq!(write_status(db, 1), WriteStatus::FAILED);
    // The member wasn't written.
    assert_eq!(
        db.get(&ao1())
            .unwrap()
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        PropertyValue::Null
    );
}

#[test]
fn bare_write_property_ends_a_channel_distribution_it_cannot_make() {
    let mut db = database();
    assert_eq!(write_pv(&mut db, 1, 3.0), ch(1));
    assert_eq!(
        read(&db, ch(1), PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(3.0)
    );
    ended_unmade(&db);
    // Nothing is left running, so the next write isn't BUSY.
    write_pv(&mut db, 1, 4.0);
    ended_unmade(&db);
    // A Channel without members starts nothing and stays IDLE.
    write_pv(&mut db, 2, 4.0);
    assert_eq!(write_status(&db, 2), WriteStatus::IDLE);
}

#[test]
fn bare_write_property_multiple_ends_the_distributions_its_writes_started() {
    let mut db = database();
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: ch(1),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: real(5.0),
                priority: Some(8),
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(&mut db, &request).unwrap();
    ended_unmade(&db);
    write_pv(&mut db, 1, 6.0);
    ended_unmade(&db);
}
