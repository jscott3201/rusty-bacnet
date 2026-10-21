//! ReadRange over a polled Trend Log Multiple: each item is one framed
//! multi-value record (#1203).

use std::sync::Mutex;
use std::time::Duration;

use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::EngineeringUnits;

/// Hands out the time last stored, on [`DATE`].
struct SettableClock(Mutex<Time>);

impl ClockReader for SettableClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: *self.0.lock().unwrap(),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn reference(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    device: Option<u32>,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: object,
        property_identifier: property.to_raw(),
        property_array_index: None,
        device_identifier: device
            .map(|instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()),
    }
}

/// A three-record Trend Log Multiple polled at hours 1 to 4, so record 1 has
/// made way and records 2 to 4 (hours 2 to 4) remain. Its members are AV-1's
/// Present_Value (42.5) and Units (95), a missing AV-9, and AV-1 qualified by
/// a device that isn't this one.
fn polled_log() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let monotonic = Arc::new(Mutex::new(Duration::ZERO));
    let source = monotonic.clone();
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    let clock = Arc::new(SettableClock(Mutex::new(time(1))));
    db.set_clock_reader(Some(clock.clone()));
    let mut av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    av.set_relinquish_default(42.5).unwrap();
    let av = {
        let oid = av.object_identifier();
        db.add(Box::new(av)).unwrap();
        oid
    };
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap();
    let mut log = TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap();
    for member in [
        reference(av, PropertyIdentifier::PRESENT_VALUE, None),
        reference(av, PropertyIdentifier::UNITS, None),
        reference(missing, PropertyIdentifier::PRESENT_VALUE, None),
        reference(av, PropertyIdentifier::PRESENT_VALUE, Some(200)),
    ] {
        log.add_property_reference(member).unwrap();
    }
    log.write_property(
        PropertyIdentifier::LOG_INTERVAL,
        None,
        PropertyValue::Unsigned(1),
        None,
    )
    .unwrap();
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    for hour in 1..=4u8 {
        *clock.0.lock().unwrap() = time(hour);
        *monotonic.lock().unwrap() = Duration::from_millis(u64::from(hour) * 10);
        db.poll_trend_logs();
    }
    (db, oid)
}

/// One polled record's exact bytes, written out by hand.
fn record_bytes(hour: u8) -> Vec<u8> {
    vec![
        0x0E, 0xA4, 0x7E, 0x08, 0x1F, 0x01, 0xB4, hour, 0x02, 0x03, 0x04,
        0x0F, // timestamp [0]
        0x1E, 0x1E, // log-data [1], member list [1]
        0x1C, 0x42, 0x2A, 0x00, 0x00, // real-value 42.5
        0x29, 0x5F, // enumerated-value 95
        0x7E, 0x91, 0x01, 0x91, 0x1F, 0x7F, // failure OBJECT / UNKNOWN_OBJECT
        0x7E, 0x91, 0x02, 0x91, 0x2D,
        0x7F, // failure PROPERTY / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        0x1F, 0x1F,
    ]
}

fn records_bytes(hours: &[u8]) -> Vec<u8> {
    hours.iter().flat_map(|&hour| record_bytes(hour)).collect()
}

#[test]
fn polled_multiple_records_read_range_with_exact_bytes() {
    let (db, oid) = polled_log();
    let cases = [
        (
            RangeSpec::ByPosition {
                reference_index: 1,
                count: 2,
            },
            vec![2, 3],
            (true, false, false),
            None,
        ),
        (
            RangeSpec::BySequenceNumber {
                reference_seq: 3,
                count: 2,
            },
            vec![3, 4],
            (false, true, false),
            Some(3),
        ),
        (
            RangeSpec::ByTime {
                reference_time: (DATE, time(2)),
                count: 2,
            },
            vec![3, 4],
            (false, true, false),
            Some(3),
        ),
        (
            RangeSpec::BySequenceNumber {
                reference_seq: 4,
                count: -3,
            },
            vec![2, 3, 4],
            (true, true, false),
            Some(2),
        ),
    ];
    for (range, hours, flags, first_sequence_number) in cases {
        let ack = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(range.clone()),
        )
        .unwrap();
        assert_eq!(ack.item_count, hours.len() as u32, "{range:?}");
        assert_eq!(ack.item_data, records_bytes(&hours), "{range:?}");
        assert_eq!(ack.result_flags, flags, "{range:?}");
        assert_eq!(
            ack.first_sequence_number, first_sequence_number,
            "{range:?}"
        );
    }
}

#[test]
fn read_range_multiple_records_decode_back_to_the_polled_values() {
    use bacnet_encoding::constructed::decode_log_multiple_record;
    use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogValue};

    let (db, oid) = polled_log();
    let ack = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    let failure = |class: ErrorClass, code: ErrorCode| LogValue::Failure {
        error_class: class,
        error_code: code,
    };
    let mut offset = 0;
    for hour in 2..=4 {
        let (record, next) = decode_log_multiple_record(&ack.item_data, offset).unwrap();
        assert_eq!(
            record,
            BACnetLogMultipleRecord {
                date: DATE,
                time: time(hour),
                log_data: LogData::Values(vec![
                    LogValue::RealValue(42.5),
                    LogValue::EnumValue(95),
                    failure(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                    failure(
                        ErrorClass::PROPERTY,
                        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
                    ),
                ]),
            }
        );
        // The decoded record encodes back to the bytes served.
        let mut encoded = BytesMut::new();
        bacnet_encoding::constructed::encode_log_multiple_record(&record, &mut encoded).unwrap();
        assert_eq!(&encoded[..], &ack.item_data[offset..next]);
        offset = next;
    }
    assert_eq!(offset, ack.item_data.len());
    assert_eq!(ack.item_count, 3);
}
