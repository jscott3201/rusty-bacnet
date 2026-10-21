//! Access Zone Entry_Points and Exit_Points: BACnetLISTs of
//! BACnetDeviceObjectReference naming Access Points (Clauses 12.32.23 and
//! 12.32.24, #1306).

use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};

use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Access Point `instance` in Device 9.
fn remote_point(instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(ObjectType::ACCESS_POINT, instance),
    }
}

fn read(zone: &AccessZoneObject, property: P) -> PropertyValue {
    zone.read_property(property, None).unwrap()
}

fn assert_value_out_of_range(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "expected PROPERTY / VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

#[test]
fn access_zone_entry_and_exit_points_serve_device_object_references() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_entry_points([oid(ObjectType::ACCESS_POINT, 1).into(), remote_point(4)])
        .unwrap();
    zone.set_exit_points([oid(ObjectType::ACCESS_POINT, 2)])
        .unwrap();
    // Object identifier [1] alone for a point in this device; device
    // identifier [0] first for one in Device 9.
    assert_eq!(
        read(&zone, P::ENTRY_POINTS),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(vec![0x1C, 0x08, 0x40, 0x00, 0x01]),
            PropertyValue::ApplicationData(vec![
                0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x08, 0x40, 0x00, 0x04,
            ]),
        ])
    );
    assert_eq!(
        read(&zone, P::EXIT_POINTS),
        PropertyValue::List(vec![PropertyValue::ApplicationData(vec![
            0x1C, 0x08, 0x40, 0x00, 0x02,
        ])])
    );
    for property in [P::ENTRY_POINTS, P::EXIT_POINTS] {
        assert!(zone.is_list_property(property));
        assert!(!zone.is_array_property(property));
        assert!(!zone.is_writable_property(property));
        // Table 12-37 codes both R, so a client's write is refused.
        let served = read(&zone, property);
        let result = zone.write_property(property, None, served.clone(), None);
        assert!(
            matches!(result, Err(Error::Protocol { code, .. })
                if code == ErrorCode::WRITE_ACCESS_DENIED),
            "{property:?}: {result:?}"
        );
        assert_eq!(read(&zone, property), served);
    }
    // An empty list clears them.
    zone.set_exit_points(Vec::<BACnetDeviceObjectReference>::new())
        .unwrap();
    assert_eq!(read(&zone, P::EXIT_POINTS), PropertyValue::List(vec![]));
}

#[test]
fn access_zone_points_refuse_other_objects_and_non_device_devices() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_entry_points([remote_point(4)]).unwrap();
    zone.set_exit_points([remote_point(5)]).unwrap();
    let kept = [P::ENTRY_POINTS, P::EXIT_POINTS].map(|p| read(&zone, p));
    let not_a_point = [
        oid(ObjectType::ACCESS_DOOR, 1).into(),
        oid(ObjectType::ACCESS_ZONE, 2).into(),
        BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 9)),
            object_identifier: oid(ObjectType::ANALOG_VALUE, 3),
        },
    ];
    let not_a_device = [
        oid(ObjectType::ANALOG_VALUE, 9),
        oid(ObjectType::ACCESS_POINT, 9),
        oid(ObjectType::ACCESS_ZONE, ObjectIdentifier::MAX_INSTANCE),
    ];
    for bad in
        not_a_point
            .into_iter()
            .chain(not_a_device.map(|device| BACnetDeviceObjectReference {
                device_identifier: Some(device),
                object_identifier: oid(ObjectType::ACCESS_POINT, 1),
            }))
    {
        // One bad reference refuses the whole list and keeps the points.
        let list = [oid(ObjectType::ACCESS_POINT, 1).into(), bad.clone()];
        assert_value_out_of_range(zone.set_entry_points(list.clone()));
        assert_value_out_of_range(zone.set_exit_points(list));
        assert_eq!(
            [P::ENTRY_POINTS, P::EXIT_POINTS].map(|p| read(&zone, p)),
            kept,
            "{bad:?}"
        );
    }
}
