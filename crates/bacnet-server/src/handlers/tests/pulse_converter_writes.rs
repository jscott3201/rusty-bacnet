use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

fn pulse_converter_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let object = PulseConverterObject::new(1, "PC-1", 62).unwrap();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode_value(value),
        priority: None,
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property(db, &bytes)
}

fn write_multiple_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: Vec<(PropertyIdentifier, PropertyValue)>,
) -> Result<Vec<ObjectIdentifier>, Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .into_iter()
                .map(|(property_identifier, value)| BACnetPropertyValue {
                    property_identifier,
                    property_array_index: None,
                    value: encode_value(value),
                    priority: None,
                })
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    handle_write_property_multiple(db, &bytes)
}

fn assert_write_access_denied<T>(result: Result<T, Error>) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32);
        }
        Err(other) => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
        Ok(_) => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got success"),
    }
}

fn assert_state(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    present_value: f32,
    out_of_service: bool,
) {
    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(present_value)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(out_of_service)
    );
}

#[test]
fn write_property_present_value_requires_out_of_service() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(12.5),
    ));
    assert_state(&db, oid, 0.0, false);

    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write_wire(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(12.5),
    )
    .unwrap();
    assert_state(&db, oid, 12.5, true);
}

#[test]
fn write_property_multiple_present_value_before_oos_fails_without_mutation() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_multiple_wire(
        &mut db,
        oid,
        vec![
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
        ],
    ));
    assert_state(&db, oid, 0.0, false);
}

#[test]
fn write_property_multiple_oos_before_present_value_succeeds() {
    let (mut db, oid) = pulse_converter_db();

    write_multiple_wire(
        &mut db,
        oid,
        vec![
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
        ],
    )
    .unwrap();
    assert_state(&db, oid, 12.5, true);
}

#[test]
fn write_property_multiple_later_failure_keeps_present_value_and_oos_prefix() {
    let (mut db, oid) = pulse_converter_db();

    assert_write_access_denied(write_multiple_wire(
        &mut db,
        oid,
        vec![
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.5)),
            (
                PropertyIdentifier::OBJECT_TYPE,
                PropertyValue::Enumerated(ObjectType::PULSE_CONVERTER.to_raw()),
            ),
        ],
    ));
    assert_state(&db, oid, 12.5, true);
}
