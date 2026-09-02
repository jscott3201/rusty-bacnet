use super::*;
use bacnet_objects::elevator::EscalatorObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn escalator_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let escalator = EscalatorObject::new(1, "ESC-1").unwrap();
    let oid = escalator.object_identifier();
    db.add(Box::new(escalator)).unwrap();
    (db, oid)
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
        property_value: value,
        priority: None,
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property(db, &request_bytes).map(|_| ())
}

fn write_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    properties: Vec<BACnetPropertyValue>,
) -> Result<(), Error> {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: properties,
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property_multiple(db, &request_bytes).map(|_| ())
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    match error {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, expected.to_raw() as u32);
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn status_values() -> Vec<(PropertyIdentifier, PropertyValue)> {
    vec![
        (PropertyIdentifier::POWER_MODE, PropertyValue::Boolean(true)),
        (
            PropertyIdentifier::OPERATION_DIRECTION,
            PropertyValue::Enumerated(2),
        ),
        (
            PropertyIdentifier::ESCALATOR_MODE,
            PropertyValue::Enumerated(3),
        ),
        (PropertyIdentifier::ENERGY_METER, PropertyValue::Real(18.75)),
        (
            PropertyIdentifier::FAULT_SIGNALS,
            PropertyValue::List(vec![
                PropertyValue::Enumerated(0),
                PropertyValue::Enumerated(1024),
            ]),
        ),
        (
            PropertyIdentifier::PASSENGER_ALARM,
            PropertyValue::Boolean(true),
        ),
    ]
}

#[test]
fn escalator_status_family_writes_over_wp_in_service_and_oos() {
    for out_of_service in [false, true] {
        let (mut db, oid) = escalator_db();
        if out_of_service {
            write_property(
                &mut db,
                oid,
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                encode_value(PropertyValue::Boolean(true)),
            )
            .unwrap();
        }

        for (property, value) in status_values() {
            write_property(&mut db, oid, property, None, encode_value(value.clone()))
                .unwrap_or_else(|error| {
                    panic!("{property:?} must write with OOS={out_of_service}: {error:?}")
                });
            assert_eq!(read(&db, oid, property), value);
        }
    }
}

#[test]
fn escalator_fault_signals_empty_singleton_and_multi_reach_the_object_over_wp() {
    let (mut db, oid) = escalator_db();

    write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        None,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        read(&db, oid, PropertyIdentifier::FAULT_SIGNALS),
        PropertyValue::List(vec![])
    );

    write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        None,
        encode_value(PropertyValue::Enumerated(8)),
    )
    .unwrap();
    assert_eq!(
        read(&db, oid, PropertyIdentifier::FAULT_SIGNALS),
        PropertyValue::List(vec![PropertyValue::Enumerated(8)])
    );

    let multi = PropertyValue::List(vec![
        PropertyValue::Enumerated(0),
        PropertyValue::Enumerated(65535),
    ]);
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        None,
        encode_value(multi.clone()),
    )
    .unwrap();
    assert_eq!(read(&db, oid, PropertyIdentifier::FAULT_SIGNALS), multi);
}

#[test]
fn escalator_fault_signals_reject_array_index_before_empty_list_decode() {
    let (mut db, oid) = escalator_db();
    let prior = PropertyValue::List(vec![PropertyValue::Enumerated(8)]);
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        None,
        encode_value(prior.clone()),
    )
    .unwrap();

    let error = write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        Some(0),
        Vec::new(),
    )
    .unwrap_err();
    assert_property_error(error, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY);
    assert_eq!(read(&db, oid, PropertyIdentifier::FAULT_SIGNALS), prior);
}

#[test]
fn escalator_empty_fault_signals_wpm_prefix_stays_committed() {
    let (mut db, oid) = escalator_db();
    let prior = PropertyValue::List(vec![
        PropertyValue::Enumerated(0),
        PropertyValue::Enumerated(1024),
    ]);
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_SIGNALS,
        None,
        encode_value(prior.clone()),
    )
    .unwrap();

    let error = write_multiple(
        &mut db,
        oid,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::FAULT_SIGNALS,
                property_array_index: None,
                value: Vec::new(),
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PASSENGER_ALARM,
                property_array_index: None,
                value: encode_value(PropertyValue::Real(1.0)),
                priority: None,
            },
        ],
    )
    .unwrap_err();
    assert_property_error(error, ErrorCode::INVALID_DATA_TYPE);
    assert_eq!(
        read(&db, oid, PropertyIdentifier::FAULT_SIGNALS),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        read(&db, oid, PropertyIdentifier::PASSENGER_ALARM),
        PropertyValue::Boolean(false)
    );

    write_multiple(
        &mut db,
        oid,
        vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::FAULT_SIGNALS,
            property_array_index: None,
            value: Vec::new(),
            priority: None,
        }],
    )
    .unwrap();
    assert_eq!(
        read(&db, oid, PropertyIdentifier::FAULT_SIGNALS),
        PropertyValue::List(vec![])
    );
}
