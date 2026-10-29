use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::{EngineeringUnits, Reliability};
use bacnet_types::error::RemoteError;

fn value(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}
fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}
fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    input: PropertyValue,
    priority: Option<u8>,
) -> Result<ObjectIdentifier, Error> {
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: value(&input),
        priority,
    }
    .encode(&mut bytes)
    .unwrap();
    sourced_wp(db, &bytes)
}
fn wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    values: Vec<(PropertyIdentifier, Option<u32>, PropertyValue)>,
) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: values
                .into_iter()
                .map(|(property, index, input)| BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: index,
                    value: value(&input),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    sourced_wpm(db, &bytes)
}
fn objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(NetworkPortObject::new_bip(1, "NP", Default::default()).unwrap()),
        Box::new(AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(ScheduleObject::new(1, "Schedule", PropertyValue::Real(0.0)).unwrap()),
    ]
}
fn snapshot(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<PropertyValue> {
    [
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
        PropertyIdentifier::STATUS_FLAGS,
    ]
    .map(|p| read(db, oid, p))
    .to_vec()
}
fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error,Error::Remote(RemoteError::Error {class:actual_class,code:actual_code}) if actual_class == class && actual_code == code),
        "{error:?}"
    );
}

#[test]
fn scalar_null_wp_preserves_property_state_and_error_precedence() {
    for mut object in objects() {
        object
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("retained".into()),
                None,
            )
            .unwrap();
        if object.object_identifier().object_type() != ObjectType::NETWORK_PORT {
            object
                .set_reliability_internal(Reliability::OVER_RANGE)
                .unwrap();
        }
        object
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let before = snapshot(&db, oid);
        for property in [
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OUT_OF_SERVICE,
        ] {
            assert_eq!(
                wp(&mut db, oid, property, None, PropertyValue::Null, None).unwrap(),
                oid
            );
            assert_eq!(snapshot(&db, oid), before);
            for index in [0, 1] {
                assert_error(
                    wp(
                        &mut db,
                        oid,
                        property,
                        Some(index),
                        PropertyValue::Null,
                        None,
                    )
                    .unwrap_err(),
                    ErrorClass::PROPERTY,
                    ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                );
                assert_eq!(snapshot(&db, oid), before);
            }
        }
        for (property, expected) in [
            (
                PropertyIdentifier::OBJECT_IDENTIFIER,
                ErrorCode::WRITE_ACCESS_DENIED,
            ),
            (
                PropertyIdentifier::from_raw(5555),
                ErrorCode::UNKNOWN_PROPERTY,
            ),
        ] {
            for input in [PropertyValue::Null, PropertyValue::Unsigned(17)] {
                assert_error(
                    wp(&mut db, oid, property, None, input, None).unwrap_err(),
                    ErrorClass::PROPERTY,
                    expected,
                );
                assert_eq!(snapshot(&db, oid), before);
            }
        }
        for index in [0, 1] {
            assert_error(
                wp(
                    &mut db,
                    oid,
                    PropertyIdentifier::from_raw(5555),
                    Some(index),
                    PropertyValue::Null,
                    None,
                )
                .unwrap_err(),
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_eq!(snapshot(&db, oid), before);
        }
        let missing = ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap();
        assert_error(
            wp(
                &mut db,
                missing,
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::Null,
                None,
            )
            .unwrap_err(),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        );
    }
}

#[test]
fn scalar_null_wpm_continues_and_failing_suffix_preserves_committed_prefix() {
    for object in objects() {
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        wp(
            &mut db,
            oid,
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        assert_eq!(
            wpm(
                &mut db,
                oid,
                vec![
                    (
                        PropertyIdentifier::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Null
                    ),
                    (PropertyIdentifier::DESCRIPTION, None, PropertyValue::Null),
                    (
                        PropertyIdentifier::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("suffix".into())
                    ),
                ]
            )
            .unwrap(),
            vec![oid]
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::DESCRIPTION),
            PropertyValue::CharacterString("suffix".into())
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::OUT_OF_SERVICE),
            PropertyValue::Boolean(true)
        );
        assert_error(
            wpm(
                &mut db,
                oid,
                vec![
                    (
                        PropertyIdentifier::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("prefix".into()),
                    ),
                    (
                        PropertyIdentifier::OUT_OF_SERVICE,
                        Some(0),
                        PropertyValue::Null,
                    ),
                    (
                        PropertyIdentifier::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("unreached".into()),
                    ),
                ],
            )
            .unwrap_err(),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::DESCRIPTION),
            PropertyValue::CharacterString("prefix".into())
        );
        assert_eq!(
            read(&db, oid, PropertyIdentifier::OUT_OF_SERVICE),
            PropertyValue::Boolean(true)
        );
    }
}

#[test]
fn scalar_null_handlers_keep_commandable_nullable_and_readonly_semantics() {
    let mut db = ObjectDatabase::new();
    let output = AnalogOutputObject::new(1, "AO", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let output_id = output.object_identifier();
    db.add(Box::new(output)).unwrap();
    wp(
        &mut db,
        output_id,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(12.0),
        Some(8),
    )
    .unwrap();
    wp(
        &mut db,
        output_id,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(24.0),
        Some(4),
    )
    .unwrap();
    wp(
        &mut db,
        output_id,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(4),
    )
    .unwrap();
    assert_eq!(
        read(&db, output_id, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(12.0)
    );
    let schedule = ScheduleObject::new(1, "nullable", PropertyValue::Real(1.0)).unwrap();
    let schedule_id = schedule.object_identifier();
    db.add(Box::new(schedule)).unwrap();
    wp(
        &mut db,
        schedule_id,
        PropertyIdentifier::SCHEDULE_DEFAULT,
        None,
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        read(&db, schedule_id, PropertyIdentifier::SCHEDULE_DEFAULT),
        PropertyValue::Null
    );
    let calendar = CalendarObject::new(1, "readonly").unwrap();
    let calendar_id = calendar.object_identifier();
    db.add(Box::new(calendar)).unwrap();
    assert_error(
        wp(
            &mut db,
            calendar_id,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn unknown_writes_file_record_count_wp_presence_and_read_only() {
    use bacnet_objects::file::FileObject;
    use bacnet_types::enums::FileAccessMethod;
    for records in [false, true] {
        let mut file = FileObject::new(1, "File", "raw").unwrap();
        if records {
            file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
            file.set_read_only(true);
        }
        let oid = file.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(file)).unwrap();
        for value in [PropertyValue::Null, PropertyValue::Unsigned(17)] {
            assert_error(
                wp(
                    &mut db,
                    oid,
                    PropertyIdentifier::RECORD_COUNT,
                    None,
                    value,
                    None,
                )
                .unwrap_err(),
                ErrorClass::PROPERTY,
                if records {
                    ErrorCode::WRITE_ACCESS_DENIED
                } else {
                    ErrorCode::UNKNOWN_PROPERTY
                },
            );
        }
    }
}
