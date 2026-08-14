use super::*;
use bacnet_objects::access_control::AccessDoorObject;
use bacnet_objects::audit::{AuditLogObject, AuditRecord};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::lighting::ChannelObject;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::traits::WritePropertyRollback;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::primitives::{Date, Time};
use std::borrow::Cow;

struct TokenBackedNameObject {
    oid: ObjectIdentifier,
    name: String,
    write_only_state: String,
}

impl TokenBackedNameObject {
    fn new() -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 98).unwrap(),
            name: "original".into(),
            write_only_state: "before".into(),
        }
    }
}

impl BACnetObject for TokenBackedNameObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::CharacterString(
                self.write_only_state.clone(),
            )),
            _ => Err(Error::Encoding("test property is not readable".into())),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::OBJECT_NAME
            && value == PropertyValue::CharacterString("renamed".into())
        {
            self.name = "renamed".into();
            return Ok(());
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(value) = value {
                self.write_only_state = value;
                return Ok(());
            }
        }
        Err(Error::Encoding("test property is not writable".into()))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::DESCRIPTION,
        ])
    }

    fn capture_write_property_rollback(
        &mut self,
        property: PropertyIdentifier,
        _value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        (property == PropertyIdentifier::OBJECT_NAME)
            .then(|| WritePropertyRollback::new(self.name.clone()))
    }

    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        self.name = rollback.downcast::<String>()?;
        Ok(())
    }
}

fn failed_wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
    priority: Option<u8>,
) -> (Result<Vec<ObjectIdentifier>, Error>, Vec<ObjectIdentifier>) {
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value,
                    priority,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property_multiple_with_residuals(db, &request_bytes)
}

fn successful_wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) -> (Result<Vec<ObjectIdentifier>, Error>, Vec<ObjectIdentifier>) {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: property,
                property_array_index: None,
                value,
                priority: None,
            }],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property_multiple_with_residuals(db, &request_bytes)
}

fn log_record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 13,
            day_of_week: 4,
        },
        time: Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(42.0),
        status_flags: None,
    }
}

#[test]
fn token_backed_name_rollback_restores_property_and_database_index() {
    let mut db = ObjectDatabase::new();
    let object = TokenBackedNameObject::new();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    let failure = BinaryValueObject::new(1, "failure").unwrap();
    let failure_oid = failure.object_identifier();
    db.add(Box::new(failure)).unwrap();

    let mut renamed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut renamed, "renamed").unwrap();
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: renamed.to_vec(),
                    priority: None,
                }],
            },
            WriteAccessSpecification {
                object_identifier: failure_oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                }],
            },
        ],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    assert!(handle_write_property_multiple(&mut db, &request_bytes).is_err());
    assert_eq!(db.get(&oid).unwrap().object_name(), "original");
    assert_eq!(
        db.find_by_name("original")
            .map(BACnetObject::object_identifier),
        Some(oid)
    );
    assert!(db.find_by_name("renamed").is_none());
}

#[test]
fn unreadable_successful_write_reports_failed_rollback_and_residual_object() {
    let mut db = ObjectDatabase::new();
    let object = TokenBackedNameObject::new();
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();

    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut changed, "changed").unwrap();
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: changed.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    let (result, residual_oids) =
        handle_write_property_multiple_with_residuals(&mut db, &request_bytes);
    assert!(result.unwrap_err().to_string().contains("rollback failed"));
    assert_eq!(residual_oids, vec![oid]);
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::CharacterString("changed".into())
    );
}

#[test]
fn channel_rollback_restores_last_priority() {
    let mut db = ObjectDatabase::new();
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(10),
            Some(3),
        )
        .unwrap();
    let oid = channel.object_identifier();
    db.add(Box::new(channel)).unwrap();

    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut changed, 20);
    let (result, residual_oids) = failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        changed.to_vec(),
        Some(8),
    );

    assert!(result.is_err());
    assert!(residual_oids.is_empty());
    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(10)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::LAST_PRIORITY, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
}

#[test]
fn network_port_rollback_restores_changes_pending() {
    let mut octets = BytesMut::new();
    bacnet_encoding::primitives::encode_app_octet_string(&mut octets, &[192, 168, 1, 10]);
    let mut udp_port = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut udp_port, 47809);
    let cases = [
        (PropertyIdentifier::IP_ADDRESS, octets.to_vec()),
        (PropertyIdentifier::IP_DEFAULT_GATEWAY, octets.to_vec()),
        (PropertyIdentifier::IP_SUBNET_MASK, octets.to_vec()),
        (PropertyIdentifier::BACNET_IP_UDP_PORT, udp_port.to_vec()),
    ];

    for (instance, (property, value)) in cases.into_iter().enumerate() {
        let mut db = ObjectDatabase::new();
        let object = NetworkPortObject::new(instance as u32, format!("NP-{instance}"), 0).unwrap();
        let oid = object.object_identifier();
        let original = object.read_property(property, None).unwrap();
        db.add(Box::new(object)).unwrap();

        let (result, residual_oids) = failed_wpm(&mut db, oid, property, value, None);

        assert!(result.is_err(), "{property:?}");
        assert!(residual_oids.is_empty(), "{property:?}");
        let object = db.get(&oid).unwrap();
        assert_eq!(object.read_property(property, None).unwrap(), original);
        assert_eq!(
            object
                .read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(false),
            "{property:?}"
        );
    }
}

#[test]
fn access_door_rollback_restores_priority_array() {
    let mut db = ObjectDatabase::new();
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(3),
    )
    .unwrap();
    let oid = door.object_identifier();
    db.add(Box::new(door)).unwrap();

    let mut changed = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut changed, 2);
    let (result, residual_oids) = failed_wpm(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        changed.to_vec(),
        Some(8),
    );

    assert!(result.is_err());
    assert!(residual_oids.is_empty());
    let object = db.get(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(3))
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        PropertyValue::Null
    );
}

#[test]
fn log_record_count_rollback_restores_cleared_buffers() {
    let mut db = ObjectDatabase::new();
    let mut trend = TrendLogObject::new(1, "TL-1", 10).unwrap();
    trend.add_record(log_record());
    let trend_oid = trend.object_identifier();
    db.add(Box::new(trend)).unwrap();

    let mut trend_multiple = TrendLogMultipleObject::new(1, "TLM-1", 10).unwrap();
    trend_multiple.add_record(log_record());
    let trend_multiple_oid = trend_multiple.object_identifier();
    db.add(Box::new(trend_multiple)).unwrap();

    let mut event = EventLogObject::new(1, "EL-1", 10).unwrap();
    event.add_record(log_record());
    let event_oid = event.object_identifier();
    db.add(Box::new(event)).unwrap();

    let mut audit = AuditLogObject::new(1, "AL-1", 10).unwrap();
    audit.add_record(AuditRecord {
        timestamp_secs: 1,
        description: "record".into(),
    });
    let audit_oid = audit.object_identifier();
    db.add(Box::new(audit)).unwrap();

    for oid in [trend_oid, trend_multiple_oid, event_oid, audit_oid] {
        let object = db.get(&oid).unwrap();
        let before = object
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap();
        let before_buffer = object
            .read_property(PropertyIdentifier::LOG_BUFFER, None)
            .ok();
        let before_total = object
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap();
        let mut clear = BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut clear, 0);

        let (result, residual_oids) = failed_wpm(
            &mut db,
            oid,
            PropertyIdentifier::RECORD_COUNT,
            clear.to_vec(),
            None,
        );

        assert!(result.is_err(), "{oid:?}");
        assert!(residual_oids.is_empty(), "{oid:?}");
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            before,
            "{oid:?}"
        );
        if let Some(before_buffer) = before_buffer {
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::LOG_BUFFER, None)
                    .unwrap(),
                before_buffer,
                "{oid:?}"
            );
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            before_total,
            "{oid:?}"
        );

        let mut invalid = BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut invalid, 1);
        let (invalid_result, invalid_residual_oids) = failed_wpm(
            &mut db,
            oid,
            PropertyIdentifier::RECORD_COUNT,
            invalid.to_vec(),
            None,
        );
        assert!(invalid_result.is_err(), "{oid:?}");
        assert!(invalid_residual_oids.is_empty(), "{oid:?}");
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            before,
            "{oid:?}"
        );

        let mut clear = BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut clear, 0);
        let (success, residual_oids) = successful_wpm(
            &mut db,
            oid,
            PropertyIdentifier::RECORD_COUNT,
            clear.to_vec(),
        );
        assert_eq!(success.unwrap(), vec![oid]);
        assert!(residual_oids.is_empty());
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(0),
            "{oid:?}"
        );
    }
}
