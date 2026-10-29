//! A Trend Log Multiple's Logging_Type, Trigger, Start_Time / Stop_Time and
//! clock-alignment rows over WriteProperty and ReadProperty (#1235).

use super::*;
use bacnet_encoding::constructed::decode_log_multiple_record;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::trend::TrendLogMultipleObject;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, LogData, LogValue};
use bacnet_types::error::RemoteError;
use std::sync::Arc;

use PropertyIdentifier as P;
use bacnet_types::enums::EngineeringUnits;

fn tlm1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, 1).unwrap()
}

/// AV-1 at 21.5 and a Trend Log Multiple of its Present_Value, with both
/// clocks bound.
fn database() -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
    let mut av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    av.set_relinquish_default(21.5).unwrap();
    db.add(Box::new(av)).unwrap();
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    tlm.add_property_reference(BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    ))
    .unwrap();
    db.add(Box::new(tlm)).unwrap();
    db.set_monotonic_clock_internal(Some(Arc::new(|| std::time::Duration::ZERO)));
    db
}

/// A WriteProperty of `value`, application-tagged bytes, to TLM-1.
fn wp(db: &mut ObjectDatabase, property: P, value: &[u8]) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: tlm1(),
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

/// The value bytes a ReadProperty of TLM-1's `property` returns.
fn rp(db: &ObjectDatabase, property: P) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: tlm1(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack).unwrap();
    ReadPropertyACK::decode(&ack).unwrap().property_value
}

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

fn log_data(db: &ObjectDatabase) -> Vec<LogData> {
    let records = db.get(&tlm1()).unwrap().log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut bytes = BytesMut::new();
            records.encode_record(index, &mut bytes);
            decode_log_multiple_record(&bytes, 0).unwrap().0.log_data
        })
        .collect()
}

const POLLED: [u8; 2] = [0x91, 0x00];
const COV: [u8; 2] = [0x91, 0x01];
const TRIGGERED: [u8; 2] = [0x91, 0x02];
const TRUE: [u8; 1] = [0x11];
const FALSE: [u8; 1] = [0x10];

#[test]
fn logging_type_writes_refuse_cov_and_steer_log_interval() {
    let mut db = database();
    wp(&mut db, P::LOG_INTERVAL, &[0x21, 0x64]).unwrap();
    // A Trend Log Multiple never logs by COV (Clause 12.30.12).
    assert_refused(
        wp(&mut db, P::LOGGING_TYPE, &COV),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(rp(&db, P::LOGGING_TYPE), POLLED);
    assert_eq!(rp(&db, P::LOG_INTERVAL), [0x21, 0x64]);

    // TRIGGERED zeroes Log_Interval and makes it read-only.
    wp(&mut db, P::LOGGING_TYPE, &TRIGGERED).unwrap();
    assert_eq!(rp(&db, P::LOG_INTERVAL), [0x21, 0x00]);
    assert_refused(
        wp(&mut db, P::LOG_INTERVAL, &[0x21, 0x64]),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // POLLED again with a zero interval takes the one-minute default.
    wp(&mut db, P::LOGGING_TYPE, &POLLED).unwrap();
    assert_eq!(rp(&db, P::LOG_INTERVAL), [0x22, 0x17, 0x70]);
}

#[test]
fn a_trigger_written_over_the_wire_is_served_by_the_next_poll() {
    let mut db = database();
    assert_refused(
        wp(&mut db, P::TRIGGER, &TRUE),
        ErrorClass::PROPERTY,
        ErrorCode::NOT_CONFIGURED_FOR_TRIGGERED_LOGGING,
    );
    wp(&mut db, P::LOGGING_TYPE, &TRIGGERED).unwrap();
    db.poll_trend_logs();
    assert!(log_data(&db).is_empty());
    wp(&mut db, P::TRIGGER, &TRUE).unwrap();
    assert_eq!(rp(&db, P::TRIGGER), TRUE);
    db.poll_trend_logs();
    assert_eq!(
        log_data(&db),
        [LogData::Values(vec![LogValue::RealValue(21.5)])]
    );
    assert_eq!(rp(&db, P::TRIGGER), FALSE);
}

#[test]
fn start_and_stop_time_take_a_date_and_time_and_alignment_reads_back() {
    let mut db = database();
    // 2150-01-01 (a Thursday) at 08:30: long after this test runs, so the
    // window closes now and says so.
    let start = [
        0xA4, 250, 1, 1, 4, // Date
        0xB4, 8, 30, 0, 0, // Time
    ];
    wp(&mut db, P::START_TIME, &start).unwrap();
    assert_eq!(rp(&db, P::START_TIME), start);
    assert_eq!(log_data(&db), [LogData::LogStatus(LogStatus::LOG_DISABLED)]);
    let unspecified = [0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(rp(&db, P::STOP_TIME), unspecified);
    // A date with an unspecified year names no moment.
    let partly = [0xA4, 0xFF, 1, 1, 4, 0xB4, 8, 30, 0, 0];
    assert_refused(
        wp(&mut db, P::STOP_TIME, &partly),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_refused(
        wp(&mut db, P::STOP_TIME, &start[..5]),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    // Back to unspecified: the window opens again.
    wp(&mut db, P::START_TIME, &unspecified).unwrap();
    assert_eq!(log_data(&db)[1..], [LogData::LogStatus(LogStatus::empty())]);

    wp(&mut db, P::ALIGN_INTERVALS, &TRUE).unwrap();
    wp(&mut db, P::INTERVAL_OFFSET, &[0x21, 0x1F]).unwrap();
    assert_eq!(rp(&db, P::ALIGN_INTERVALS), TRUE);
    assert_eq!(rp(&db, P::INTERVAL_OFFSET), [0x21, 0x1F]);
}
