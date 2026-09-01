//! Wire-level Alert Enrollment Table 12-61 property-surface checks.

use super::*;
use bacnet_objects::event_enrollment::AlertEnrollmentObject;
use bacnet_types::enums::NotifyType;

fn make_alert_db() -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    let alert = AlertEnrollmentObject::new(1, "AE-1", source).unwrap();
    let alert_oid = alert.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(alert)).unwrap();
    (db, alert_oid, source)
}

fn read_wire(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<PropertyValue, Error> {
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    let mut response_bytes = BytesMut::new();
    handle_read_property(db, &request_bytes, &mut response_bytes)?;
    let encoded = ReadPropertyACK::decode(&response_bytes)?.property_value;
    Ok(bacnet_encoding::primitives::decode_application_value(&encoded, 0)?.0)
}

fn write_wire(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value)?;
    write_raw(db, oid, property, encoded.to_vec())
}

fn write_raw(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    property_value: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value,
        priority: None,
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property(db, &request_bytes).map(|_| ())
}

fn assert_property_error<T: std::fmt::Debug>(
    result: Result<T, Error>,
    expected: ErrorCode,
    context: &str,
) {
    match result.expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32, "{context}");
            assert_eq!(code, expected.to_raw() as u32, "{context}");
        }
        other => panic!("{context}: expected property error, got {other:?}"),
    }
}

#[test]
fn alert_present_value_and_removed_properties_have_exact_wire_access() {
    let (mut db, oid, source) = make_alert_db();
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PRESENT_VALUE).unwrap(),
        PropertyValue::ObjectIdentifier(source)
    );
    assert_property_error(
        write_wire(
            &mut db,
            oid,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::ObjectIdentifier(
                ObjectIdentifier::new(ObjectType::BINARY_INPUT, 8).unwrap(),
            ),
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
        "Present_Value write",
    );

    for property in [
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert_property_error(
            read_wire(&db, oid, property),
            ErrorCode::UNKNOWN_PROPERTY,
            &format!("{property:?} read"),
        );
        assert_property_error(
            write_wire(&mut db, oid, property, PropertyValue::Boolean(true)),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{property:?} write"),
        );
    }

    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PRESENT_VALUE).unwrap(),
        PropertyValue::ObjectIdentifier(source),
        "every refused write must preserve the source"
    );
}

#[test]
fn alert_notify_type_wire_domain_is_alarm_or_event() {
    let (mut db, oid, _) = make_alert_db();
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::NOTIFY_TYPE).unwrap(),
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw())
    );

    for accepted in [NotifyType::EVENT, NotifyType::ALARM] {
        write_wire(
            &mut db,
            oid,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(accepted.to_raw()),
        )
        .unwrap();
    }
    for refused in [NotifyType::ACK_NOTIFICATION.to_raw(), 3, 99, u32::MAX] {
        assert_property_error(
            write_wire(
                &mut db,
                oid,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyValue::Enumerated(refused),
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("Notify_Type={refused}"),
        );
    }
    assert_property_error(
        write_wire(
            &mut db,
            oid,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Unsigned(1),
        ),
        ErrorCode::INVALID_DATA_TYPE,
        "Notify_Type wrong datatype",
    );

    let mut overwide = BytesMut::new();
    bacnet_encoding::tags::encode_tag(
        &mut overwide,
        bacnet_encoding::tags::app_tag::ENUMERATED,
        bacnet_encoding::tags::TagClass::Application,
        8,
    );
    overwide.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 2]);
    assert_property_error(
        write_raw(
            &mut db,
            oid,
            PropertyIdentifier::NOTIFY_TYPE,
            overwide.to_vec(),
        ),
        ErrorCode::INVALID_DATA_ENCODING,
        "overwide Notify_Type",
    );
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::NOTIFY_TYPE).unwrap(),
        PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
        "every refused write must preserve Notify_Type"
    );
}
