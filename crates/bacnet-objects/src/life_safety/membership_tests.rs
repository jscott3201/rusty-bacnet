//! Member_Of and Zone_Members as BACnetDeviceObjectReference lists (#1182).

use super::*;
use bacnet_types::enums::ErrorCode;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn remote(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(object_type, instance),
    }
}

fn assert_out_of_range(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "expected PROPERTY / VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

#[test]
fn point_member_of_keeps_a_remote_zone_and_lists_each_zone_once() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    point
        .add_member(remote(ObjectType::LIFE_SAFETY_ZONE, 4))
        .unwrap();
    point
        .add_member(remote(ObjectType::LIFE_SAFETY_ZONE, 4))
        .unwrap();
    assert_eq!(
        point
            .read_property(PropertyIdentifier::MEMBER_OF, None)
            .unwrap(),
        PropertyValue::List(vec![PropertyValue::ApplicationData(vec![
            0x0C, 0x02, 0x00, 0x00, 0x09, // [0] device 9
            0x1C, 0x05, 0x80, 0x00, 0x04, // [1] life-safety-zone 4
        ])])
    );
}

#[test]
fn membership_refuses_other_object_types_and_device_members() {
    let mut point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let mut zone = LifeSafetyZoneObject::new(2, "LSZ-2").unwrap();
    // Member_Of names zones only; Zone_Members names points and zones.
    assert_out_of_range(point.add_member(oid(ObjectType::LIFE_SAFETY_POINT, 3)));
    assert_out_of_range(zone.add_member(oid(ObjectType::LIFE_SAFETY_POINT, 3)));
    assert_out_of_range(zone.add_zone_member(oid(ObjectType::ANALOG_INPUT, 3)));
    // A Device member must name a Device.
    let not_a_device = BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::ANALOG_INPUT, 9)),
        object_identifier: oid(ObjectType::LIFE_SAFETY_ZONE, 4),
    };
    assert_out_of_range(point.add_member(not_a_device.clone()));
    assert_out_of_range(zone.add_member(not_a_device));
    for (object, property) in [
        (&point as &dyn BACnetObject, PropertyIdentifier::MEMBER_OF),
        (&zone, PropertyIdentifier::MEMBER_OF),
        (&zone, PropertyIdentifier::ZONE_MEMBERS),
    ] {
        assert_eq!(
            object.read_property(property, None).unwrap(),
            PropertyValue::List(Vec::new())
        );
    }
    zone.add_zone_member(oid(ObjectType::LIFE_SAFETY_ZONE, 5))
        .unwrap();
    zone.add_zone_member(remote(ObjectType::LIFE_SAFETY_POINT, 6))
        .unwrap();
    let PropertyValue::List(members) = zone
        .read_property(PropertyIdentifier::ZONE_MEMBERS, None)
        .unwrap()
    else {
        panic!("Zone_Members is a list");
    };
    assert_eq!(members.len(), 2);
}

#[test]
fn member_lists_stay_read_only_over_the_network() {
    let served = PropertyValue::List(vec![PropertyValue::ApplicationData(vec![
        0x1C, 0x05, 0x80, 0x00, 0x04,
    ])]);
    for (mut object, property) in [
        (
            Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()) as Box<dyn BACnetObject>,
            PropertyIdentifier::MEMBER_OF,
        ),
        (
            Box::new(LifeSafetyZoneObject::new(2, "LSZ-2").unwrap()),
            PropertyIdentifier::MEMBER_OF,
        ),
        (
            Box::new(LifeSafetyZoneObject::new(2, "LSZ-2").unwrap()),
            PropertyIdentifier::ZONE_MEMBERS,
        ),
    ] {
        assert!(!object.is_writable_property(property));
        assert!(object.is_list_property(property));
        assert!(matches!(
            object.write_property(property, None, served.clone(), None),
            Err(Error::Protocol { code, .. })
                if code == ErrorCode::WRITE_ACCESS_DENIED
        ));
    }
}
