//! Energy_Meter_Ref on the Lift (Clause 12.59, Table 12-77) and the
//! Escalator (Clause 12.60, Table 12-78; #1036): an optional
//! BACnetDeviceObjectReference that the application sets and that is
//! read-only over the network. While it names a meter object, Energy_Meter
//! reads 0.0.

use super::super::*;
use super::{assert_invalid_data_type, assert_value_out_of_range};
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

const ENERGY_METER: PropertyIdentifier = PropertyIdentifier::ENERGY_METER;
const ENERGY_METER_REF: PropertyIdentifier = PropertyIdentifier::ENERGY_METER_REF;

/// The Energy_Meter_Ref API both objects share, so each test runs on both.
trait Metered: BACnetObject {
    fn energy_meter_ref(&self) -> &BACnetDeviceObjectReference;
    fn set_energy_meter_ref(&mut self, reference: BACnetDeviceObjectReference)
    -> Result<(), Error>;
}

macro_rules! impl_metered {
    ($object:ty) => {
        impl Metered for $object {
            fn energy_meter_ref(&self) -> &BACnetDeviceObjectReference {
                <$object>::energy_meter_ref(self)
            }
            fn set_energy_meter_ref(
                &mut self,
                reference: BACnetDeviceObjectReference,
            ) -> Result<(), Error> {
                <$object>::set_energy_meter_ref(self, reference)
            }
        }
    };
}
impl_metered!(LiftObject);
impl_metered!(EscalatorObject);

fn objects() -> [Box<dyn Metered>; 2] {
    [
        Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
        Box::new(EscalatorObject::new(1, "ESC-1").unwrap()),
    ]
}

fn reference(
    device: Option<u32>,
    object_type: ObjectType,
    instance: u32,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: device
            .map(|instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()),
        object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
    }
}

fn uninitialized() -> BACnetDeviceObjectReference {
    reference(
        None,
        ObjectType::ACCUMULATOR,
        ObjectIdentifier::MAX_INSTANCE,
    )
}

fn write(
    object: &mut dyn Metered,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn read(object: &dyn Metered, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn assert_write_access_denied(result: Result<(), Error>, context: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED, "{context}");
        }
        other => panic!("{context}: expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
    }
}

#[test]
fn energy_meter_ref_starts_uninitialized_and_is_read_only_over_the_network() {
    for mut object in objects() {
        let name = object.object_name().to_owned();
        assert_eq!(*object.energy_meter_ref(), uninitialized(), "{name}");
        // object [1] Accumulator (type 23) instance 4194303, no device [0].
        let unset = PropertyValue::ApplicationData(vec![0x1C, 0x05, 0xFF, 0xFF, 0xFF]);
        assert_eq!(read(&*object, ENERGY_METER_REF), unset, "{name}");
        assert!(object.property_list().contains(&ENERGY_METER_REF), "{name}");
        assert!(!object.required_properties().contains(&ENERGY_METER_REF));
        assert!(!object.is_writable_property(ENERGY_METER_REF), "{name}");
        object
            .set_energy_meter_ref(reference(None, ObjectType::ACCUMULATOR, 7))
            .unwrap();
        let set = read(&*object, ENERGY_METER_REF);
        for value in [
            set.clone(),
            unset,
            PropertyValue::ObjectIdentifier(
                ObjectIdentifier::new(ObjectType::ACCUMULATOR, 8).unwrap(),
            ),
        ] {
            for out_of_service in [false, true] {
                write(
                    &mut *object,
                    PropertyIdentifier::OUT_OF_SERVICE,
                    PropertyValue::Boolean(out_of_service),
                )
                .unwrap();
                assert_write_access_denied(
                    write(&mut *object, ENERGY_METER_REF, value.clone()),
                    &format!("{name} OOS={out_of_service} {value:?}"),
                );
            }
        }
        assert_eq!(read(&*object, ENERGY_METER_REF), set, "{name}");
    }
}

#[test]
fn set_energy_meter_ref_takes_local_and_remote_meter_objects() {
    for mut object in objects() {
        let name = object.object_name().to_owned();
        // object [1] holds the type in its top 10 bits and the instance 5.
        for (object_type, wire_type) in [
            (ObjectType::ACCUMULATOR, [0x05, 0xC0]),
            (ObjectType::PULSE_CONVERTER, [0x06, 0x00]),
            (ObjectType::ANALOG_INPUT, [0x00, 0x00]),
            (ObjectType::ANALOG_VALUE, [0x00, 0x80]),
            (ObjectType::INTEGER_VALUE, [0x0B, 0x40]),
            (ObjectType::LARGE_ANALOG_VALUE, [0x0B, 0x80]),
            (ObjectType::POSITIVE_INTEGER_VALUE, [0x0C, 0x00]),
            // Proprietary object types: vendor meters.
            (ObjectType::from_raw(128), [0x20, 0x00]),
            (ObjectType::from_raw(1023), [0xFF, 0xC0]),
        ] {
            let meter = reference(None, object_type, 5);
            object.set_energy_meter_ref(meter.clone()).unwrap();
            assert_eq!(*object.energy_meter_ref(), meter, "{name}");
            assert_eq!(
                read(&*object, ENERGY_METER_REF),
                PropertyValue::ApplicationData(vec![0x1C, wire_type[0], wire_type[1], 0x00, 0x05]),
                "{name} {object_type:?}"
            );
        }
        // A meter in another device: device [0] Device 9, then object [1]
        // Analog Input 3.
        let remote = reference(Some(9), ObjectType::ANALOG_INPUT, 3);
        object.set_energy_meter_ref(remote).unwrap();
        assert_eq!(
            read(&*object, ENERGY_METER_REF),
            PropertyValue::ApplicationData(vec![
                0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x00, 0x00, 0x00, 0x03
            ]),
            "{name}"
        );
        // The uninitialized reference clears it.
        object.set_energy_meter_ref(uninitialized()).unwrap();
        assert_eq!(*object.energy_meter_ref(), uninitialized(), "{name}");
    }
}

#[test]
fn set_energy_meter_ref_refuses_objects_that_are_not_meters_atomically() {
    for mut object in objects() {
        let name = object.object_name().to_owned();
        let meter = reference(Some(4), ObjectType::ACCUMULATOR, 7);
        object.set_energy_meter_ref(meter.clone()).unwrap();
        let wire = read(&*object, ENERGY_METER_REF);
        for refused in [
            reference(None, ObjectType::DEVICE, 4),
            reference(None, ObjectType::ANALOG_OUTPUT, 1),
            reference(None, ObjectType::BINARY_INPUT, 1),
            reference(None, ObjectType::MULTI_STATE_VALUE, 1),
            reference(None, ObjectType::TREND_LOG, 1),
            reference(None, ObjectType::ELEVATOR_GROUP, 1),
            reference(None, ObjectType::LIFT, 1),
            // Not a meter type even with the no-object instance.
            reference(None, ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE),
            // The device of a reference must be a Device object.
            BACnetDeviceObjectReference {
                device_identifier: Some(
                    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 4).unwrap(),
                ),
                object_identifier: ObjectIdentifier::new(ObjectType::ACCUMULATOR, 7).unwrap(),
            },
        ] {
            assert_value_out_of_range(
                object.set_energy_meter_ref(refused.clone()),
                &format!("{name} {refused:?}"),
            );
            assert_eq!(*object.energy_meter_ref(), meter, "{name}");
            assert_eq!(read(&*object, ENERGY_METER_REF), wire, "{name}");
        }
    }
}

#[test]
fn energy_meter_reads_zero_while_a_reference_is_set() {
    for mut object in objects() {
        let name = object.object_name().to_owned();
        write(&mut *object, ENERGY_METER, PropertyValue::Real(42.5)).unwrap();
        assert_eq!(read(&*object, ENERGY_METER), PropertyValue::Real(42.5));

        // Naming a meter zeroes the local reading.
        object
            .set_energy_meter_ref(reference(None, ObjectType::ACCUMULATOR, 7))
            .unwrap();
        assert_eq!(read(&*object, ENERGY_METER), PropertyValue::Real(0.0));
        for out_of_service in [false, true] {
            write(
                &mut *object,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(out_of_service),
            )
            .unwrap();
            for value in [5.0f32, -1.5, f32::MIN_POSITIVE, f32::NAN, f32::INFINITY] {
                assert_value_out_of_range(
                    write(&mut *object, ENERGY_METER, PropertyValue::Real(value)),
                    &format!("{name} OOS={out_of_service} Energy_Meter {value}"),
                );
            }
            assert_invalid_data_type(
                write(&mut *object, ENERGY_METER, PropertyValue::Unsigned(0)),
                &format!("{name} Unsigned Energy_Meter"),
            );
            // 0.0, either sign, is the one value in range; it reads back as
            // positive zero.
            for zero in [0.0f32, -0.0] {
                write(&mut *object, ENERGY_METER, PropertyValue::Real(zero)).unwrap();
                assert!(
                    matches!(read(&*object, ENERGY_METER), PropertyValue::Real(v) if v.to_bits() == 0),
                    "{name} {zero}"
                );
            }
        }

        // Another meter keeps the rule; a reference to instance 4194303 of
        // any meter type clears it, and the reading restarts from 0.0.
        object
            .set_energy_meter_ref(reference(Some(9), ObjectType::ANALOG_INPUT, 3))
            .unwrap();
        assert_value_out_of_range(
            write(&mut *object, ENERGY_METER, PropertyValue::Real(1.0)),
            &format!("{name} remote meter"),
        );
        object
            .set_energy_meter_ref(reference(
                None,
                ObjectType::ANALOG_INPUT,
                ObjectIdentifier::MAX_INSTANCE,
            ))
            .unwrap();
        assert_eq!(read(&*object, ENERGY_METER), PropertyValue::Real(0.0));
        write(&mut *object, ENERGY_METER, PropertyValue::Real(12.5)).unwrap();
        assert_eq!(
            read(&*object, ENERGY_METER),
            PropertyValue::Real(12.5),
            "{name}"
        );
    }
}
