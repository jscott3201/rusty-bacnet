//! A Trend Log's Logging_Type, Log_Interval and Trigger (#1354), its COV
//! acquisition and COV rows (#1480), and the
//! Start_Time / Stop_Time window of a Trend Log and an Event Log (#1353),
//! over WriteProperty and ReadProperty.

use super::*;
use bacnet_encoding::constructed::{decode_event_log_record, decode_log_record};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, EventLogDatum, LogDatum};
use bacnet_types::error::RemoteError;
use std::sync::Arc;

use PropertyIdentifier as P;
use bacnet_types::enums::EngineeringUnits;

fn tl1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

fn el1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_LOG, 1).unwrap()
}

/// AV-1 at 21.5, a Trend Log of its Present_Value and an Event Log, with
/// both clocks bound.
fn database() -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
    let mut av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    av.set_relinquish_default(21.5).unwrap();
    db.add(Box::new(av)).unwrap();
    let mut tl = TrendLogObject::new(1, "TL-1", 8).unwrap();
    tl.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    db.add(Box::new(tl)).unwrap();
    db.add(Box::new(EventLogObject::new(1, "EL-1", 8).unwrap()))
        .unwrap();
    db.set_monotonic_clock_internal(Some(Arc::new(|| std::time::Duration::ZERO)));
    db
}

/// A WriteProperty of `value`, application-tagged bytes, to `oid`.
fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

/// The value bytes a ReadProperty of `oid`'s `property` returns.
fn rp(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
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

/// Each record of `oid` as ReadRange serves it: its status, or `None` for
/// an ordinary record.
fn statuses(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<Option<LogStatus>> {
    let records = db.get(&oid).unwrap().log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut bytes = BytesMut::new();
            records.encode_record(index, &mut bytes);
            if oid.object_type() == ObjectType::TREND_LOG {
                match decode_log_record(&bytes, 0).unwrap().0.log_datum {
                    LogDatum::LogStatus(status) => Some(status),
                    _ => None,
                }
            } else {
                match decode_event_log_record(&bytes, 0).unwrap().0.log_datum {
                    EventLogDatum::LogStatus(status) => Some(status),
                    _ => None,
                }
            }
        })
        .collect()
}

const POLLED: [u8; 2] = [0x91, 0x00];
const COV: [u8; 2] = [0x91, 0x01];
const TRIGGERED: [u8; 2] = [0x91, 0x02];
const TRUE: [u8; 1] = [0x11];
const FALSE: [u8; 1] = [0x10];

#[test]
fn trend_log_logging_type_takes_cov_and_steers_log_interval() {
    let mut db = database();
    wp(&mut db, tl1(), P::LOG_INTERVAL, &[0x21, 0x64]).unwrap();
    // A value outside the enumeration gets the Clause 12.25.26 answer.
    assert_refused(
        wp(&mut db, tl1(), P::LOGGING_TYPE, &[0x91, 0x03]),
        ErrorClass::PROPERTY,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(rp(&db, tl1(), P::LOGGING_TYPE), POLLED);
    // COV zeroes Log_Interval, which stays writable; a nonzero interval
    // turns the log back to POLLED, and zero to COV again (Clause 12.25.9,
    // #1480).
    wp(&mut db, tl1(), P::LOGGING_TYPE, &COV).unwrap();
    assert_eq!(rp(&db, tl1(), P::LOG_INTERVAL), [0x21, 0x00]);
    wp(&mut db, tl1(), P::LOG_INTERVAL, &[0x21, 0x64]).unwrap();
    assert_eq!(rp(&db, tl1(), P::LOGGING_TYPE), POLLED);
    wp(&mut db, tl1(), P::LOG_INTERVAL, &[0x21, 0x00]).unwrap();
    assert_eq!(rp(&db, tl1(), P::LOGGING_TYPE), COV);
    // The two COV rows go over the wire too: NULL and a REAL increment, and
    // a nonzero resubscription interval.
    assert_eq!(rp(&db, tl1(), P::CLIENT_COV_INCREMENT), [0x00]);
    wp(
        &mut db,
        tl1(),
        P::CLIENT_COV_INCREMENT,
        &[0x44, 0x40, 0x00, 0x00, 0x00],
    )
    .unwrap();
    assert_eq!(
        rp(&db, tl1(), P::CLIENT_COV_INCREMENT),
        [0x44, 0x40, 0x00, 0x00, 0x00]
    );
    wp(
        &mut db,
        tl1(),
        P::COV_RESUBSCRIPTION_INTERVAL,
        &[0x21, 0x3C],
    )
    .unwrap();
    assert_eq!(rp(&db, tl1(), P::COV_RESUBSCRIPTION_INTERVAL), [0x21, 0x3C]);
    assert_refused(
        wp(
            &mut db,
            tl1(),
            P::COV_RESUBSCRIPTION_INTERVAL,
            &[0x21, 0x00],
        ),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    // The poller logs AV-1's value once, then only a change of at least the
    // increment, 2.0.
    db.poll_trend_logs();
    assert_eq!(statuses(&db, tl1()), [None]);
    for (value, records) in [
        ([0x44, 0x41, 0xB0, 0x00, 0x00], 1),
        ([0x44, 0x41, 0xD0, 0x00, 0x00], 2),
    ] {
        // 22.0 is 0.5 past 21.5; 26.0 is 4.0 past it.
        wp(&mut db, av1(), P::PRESENT_VALUE, &value).unwrap();
        db.poll_trend_logs();
        assert_eq!(statuses(&db, tl1()).len(), records);
    }
    wp(&mut db, tl1(), P::LOG_INTERVAL, &[0x21, 0x64]).unwrap();

    // TRIGGERED zeroes Log_Interval and makes it read-only.
    wp(&mut db, tl1(), P::LOGGING_TYPE, &TRIGGERED).unwrap();
    assert_eq!(rp(&db, tl1(), P::LOG_INTERVAL), [0x21, 0x00]);
    assert_refused(
        wp(&mut db, tl1(), P::LOG_INTERVAL, &[0x21, 0x64]),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // A Trigger written over the wire is served by the next poll, after
    // the two COV records.
    db.poll_trend_logs();
    assert_eq!(statuses(&db, tl1()), [None, None]);
    wp(&mut db, tl1(), P::TRIGGER, &TRUE).unwrap();
    assert_eq!(rp(&db, tl1(), P::TRIGGER), TRUE);
    db.poll_trend_logs();
    assert_eq!(statuses(&db, tl1()), [None, None, None]);
    assert_eq!(rp(&db, tl1(), P::TRIGGER), FALSE);
}

#[test]
fn trend_log_and_event_log_windows_take_a_date_and_time() {
    let mut db = database();
    for oid in [tl1(), el1()] {
        // 2150-01-01 (a Thursday) at 08:30: long after this test runs, so
        // the window shuts now and says so.
        let start = [
            0xA4, 250, 1, 1, 4, // Date
            0xB4, 8, 30, 0, 0, // Time
        ];
        wp(&mut db, oid, P::START_TIME, &start).unwrap();
        assert_eq!(rp(&db, oid, P::START_TIME), start);
        assert_eq!(statuses(&db, oid), [Some(LogStatus::LOG_DISABLED)]);
        let unspecified = [0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(rp(&db, oid, P::STOP_TIME), unspecified);
        // A date with an unspecified year names no moment.
        let partly = [0xA4, 0xFF, 1, 1, 4, 0xB4, 8, 30, 0, 0];
        assert_refused(
            wp(&mut db, oid, P::STOP_TIME, &partly),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_refused(
            wp(&mut db, oid, P::STOP_TIME, &start[..5]),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Back to unspecified: the window opens again.
        wp(&mut db, oid, P::START_TIME, &unspecified).unwrap();
        assert_eq!(
            statuses(&db, oid),
            [Some(LogStatus::LOG_DISABLED), Some(LogStatus::empty())]
        );
    }
}
