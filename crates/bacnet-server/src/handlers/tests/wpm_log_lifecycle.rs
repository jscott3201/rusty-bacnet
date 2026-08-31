use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::primitives::{Date, Time};
use std::sync::Arc;

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        time: Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        },
        log_datum: LogDatum::RealValue(42.0),
        status_flags: None,
    }
}

fn failed_wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
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
    handle_write_property_multiple_with_residuals(db, &request_bytes)
}

#[derive(Clone, Copy)]
enum LogKind {
    Event,
    Trend,
    TrendMultiple,
}

fn add_full_log(db: &mut ObjectDatabase, kind: LogKind) -> ObjectIdentifier {
    match kind {
        LogKind::Event => {
            let mut object = EventLogObject::new(1, "EL-1", 3).unwrap();
            for _ in 0..3 {
                object.add_record(record());
            }
            let oid = object.object_identifier();
            db.add(Box::new(object)).unwrap();
            oid
        }
        LogKind::Trend => {
            let mut object = TrendLogObject::new(1, "TL-1", 3).unwrap();
            for _ in 0..3 {
                object.add_record(record());
            }
            let oid = object.object_identifier();
            db.add(Box::new(object)).unwrap();
            oid
        }
        LogKind::TrendMultiple => {
            let mut object = TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap();
            for _ in 0..3 {
                object.add_record(record());
            }
            let oid = object.object_identifier();
            db.add(Box::new(object)).unwrap();
            oid
        }
    }
}

#[test]
fn log_enable_and_stop_rollback_restore_exact_lifecycle() {
    for kind in [LogKind::Event, LogKind::Trend, LogKind::TrendMultiple] {
        for property in [
            PropertyIdentifier::LOG_ENABLE,
            PropertyIdentifier::STOP_WHEN_FULL,
        ] {
            let mut db = ObjectDatabase::new();
            db.set_clock_reader(Some(Arc::new(FixedClock)));
            let oid = add_full_log(&mut db, kind);
            let object = db.get(&oid).unwrap();
            let before_buffer = object
                .read_property(PropertyIdentifier::LOG_BUFFER, None)
                .unwrap();
            let before_total = object
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap();
            let before_enable = object
                .read_property(PropertyIdentifier::LOG_ENABLE, None)
                .unwrap();
            let before_stop = object
                .read_property(PropertyIdentifier::STOP_WHEN_FULL, None)
                .unwrap();
            let before_identities = object.log_record_identities_internal().unwrap();

            let mut value = BytesMut::new();
            bacnet_encoding::primitives::encode_app_boolean(
                &mut value,
                property == PropertyIdentifier::STOP_WHEN_FULL,
            );
            let (result, residual_oids) = failed_wpm(&mut db, oid, property, value.to_vec());

            assert!(result.is_err(), "{property:?} {oid:?}");
            assert!(residual_oids.is_empty(), "{property:?} {oid:?}");
            let object = db.get(&oid).unwrap();
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::LOG_BUFFER, None)
                    .unwrap(),
                before_buffer,
                "{property:?} {oid:?}"
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                    .unwrap(),
                before_total,
                "{property:?} {oid:?}"
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::LOG_ENABLE, None)
                    .unwrap(),
                before_enable,
                "{property:?} {oid:?}"
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::STOP_WHEN_FULL, None)
                    .unwrap(),
                before_stop,
                "{property:?} {oid:?}"
            );
            assert_eq!(
                object.log_record_identities_internal().unwrap(),
                before_identities,
                "{property:?} {oid:?}"
            );
        }
    }
}
