use super::*;
use bacnet_objects::{
    analog::{AnalogOutputObject, AnalogValueObject},
    binary::{BinaryOutputObject, BinaryValueObject},
    multistate::{MultiStateOutputObject, MultiStateValueObject},
    traits::BACnetObject,
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;

#[test]
fn six_family_initial_command_source_wire_values_are_typed_and_paired() {
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(AnalogOutputObject::new(1, "AO", EngineeringUnits::SQUARE_METERS).unwrap()),
        Box::new(AnalogValueObject::new(1, "AV", EngineeringUnits::SQUARE_METERS).unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
        Box::new(BinaryValueObject::new(1, "BV").unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO", 2).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV", 2).unwrap()),
    ];
    let mut db = ObjectDatabase::new();
    let oids: Vec<_> = objects.iter().map(|o| o.object_identifier()).collect();
    for object in objects {
        db.add(object).unwrap();
    }
    for oid in oids {
        for (property, index, expected) in [
            (PropertyIdentifier::VALUE_SOURCE, None, vec![0x08]),
            (
                PropertyIdentifier::VALUE_SOURCE_ARRAY,
                Some(0),
                vec![0x21, 16],
            ),
            (PropertyIdentifier::VALUE_SOURCE_ARRAY, None, vec![0x08; 16]),
            (PropertyIdentifier::LAST_COMMAND_TIME, None, vec![0x19, 0]),
        ] {
            let mut request = BytesMut::new();
            ReadPropertyRequest {
                object_identifier: oid,
                property_identifier: property,
                property_array_index: index,
            }
            .encode(&mut request);
            let mut response = BytesMut::new();
            handle_read_property(&db, &request, &mut response).unwrap();
            assert_eq!(
                ReadPropertyACK::decode(&response).unwrap().property_value,
                expected,
                "{oid:?} {property:?} {index:?}"
            );
        }
    }
}

#[test]
fn command_source_context_free_handlers_cannot_invent_a_writer() {
    let mut db = ObjectDatabase::new();
    let value = AnalogValueObject::new(
        824,
        "standalone command target",
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    let oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: vec![0x44, 0x42, 0x28, 0, 0], // application REAL 42
        priority: Some(8),
    }
    .encode(&mut bytes)
    .unwrap();
    assert!(
        matches!(handle_write_property(&mut db, &bytes), Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::VALUE_SOURCE, None)
            .unwrap(),
        PropertyValue::ApplicationData(vec![0x08])
    );
}
