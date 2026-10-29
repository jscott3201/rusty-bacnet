//! A lab site's device, built with bacnet-objects: one object of each kind a
//! technician checks out, with AI:1 out of service and in fault and AO:1
//! commanded at priority 10. Shared by the TUI's in-process tests (through
//! `#[path]`) and the one-shot integration tests.

use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// The vendor identifier the lab devices report.
pub const LAB_VENDOR: u16 = 260;

/// The lab device's identity, for the server builder.
pub fn lab_identity(instance: u32) -> DeviceIdentity {
    DeviceIdentity::new(instance, LAB_VENDOR)
        .expect("valid Device identity")
        .with_name(format!("Lab device {instance}"))
}

/// A database of the lab Device and `objects`, listed in its Object_List.
fn database(instance: u32, objects: Vec<Box<dyn BACnetObject>>) -> ObjectDatabase {
    let mut device = DeviceObject::new(DeviceConfig {
        identity: lab_identity(instance),
        vendor_name: "Rusty Lab".into(),
        model_name: "Bench-1".into(),
        firmware_revision: "1.2.3".into(),
        application_software_version: "4.5".into(),
        ..Default::default()
    })
    .expect("lab Device object");
    let mut list = vec![device.object_identifier()];
    list.extend(objects.iter().map(|object| object.object_identifier()));
    device.set_object_list(list);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    for object in objects {
        db.add(object).unwrap();
    }
    db
}

fn set(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: PropertyValue) {
    object
        .write_property(property, None, value, None)
        .unwrap_or_else(|error| panic!("lab {property}: {error}"));
}

/// The lab device: Device, AI:1 (out of service, in fault), AI:2, AO:1
/// (commanded to 55 at priority 10), AV:1, BI:1, BO:1, BV:1, MSV:1, NC:1
/// and TL:1.
pub fn lab_database(instance: u32) -> ObjectDatabase {
    let mut ai1 =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai1.set_present_value(72.5);
    set(
        &mut ai1,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    set(
        &mut ai1,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
    );
    let mut ai2 =
        AnalogInputObject::new(2, "Outside Air", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai2.set_present_value(51.0);

    let mut ao1 = AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap();
    let operator = CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        initiating_object: None,
    };
    ao1.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(55.0),
        Some(10),
        &operator,
    )
    .expect("command AO:1 at priority 10");
    database(
        instance,
        vec![
            Box::new(ai1),
            Box::new(ai2),
            Box::new(ao1),
            Box::new(
                AnalogValueObject::new(1, "Setpoint", EngineeringUnits::DEGREES_FAHRENHEIT)
                    .unwrap(),
            ),
            Box::new(BinaryInputObject::new(1, "Fan Status").unwrap()),
            Box::new(BinaryOutputObject::new(1, "Fan Command").unwrap()),
            Box::new(BinaryValueObject::new(1, "Occupied").unwrap()),
            Box::new(MultiStateValueObject::new(1, "Mode", 3).unwrap()),
            Box::new(NotificationClass::new(1, "Alarms").unwrap()),
            Box::new(TrendLogObject::new(1, "Zone Temp Log", 100).unwrap()),
        ],
    )
}

/// A device with `count` analog values besides its Device object.
pub fn large_database(instance: u32, count: u32) -> ObjectDatabase {
    let points = (1..=count)
        .map(|n| {
            Box::new(
                AnalogValueObject::new(n, format!("Point {n}"), EngineeringUnits::NO_UNITS)
                    .unwrap(),
            ) as Box<dyn BACnetObject>
        })
        .collect();
    database(instance, points)
}
