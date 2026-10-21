//! COV acquisition for a Trend Log (#1480): each change of AV-1's
//! Present_Value that meets the increment, and each change of its
//! Status_Flags, logs one record carrying those flags; Enable and the
//! Start_Time / Stop_Time window gate the records as they gate polled ones.
//! The monotonic clock and the Device clock are both set by hand.

use super::acquisition_tests::{at, set_clock};
use super::*;
use bacnet_encoding::constructed::decode_log_record;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::primitives::StatusFlags;

fn records(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<BACnetLogRecord> {
    let records = db.get(&oid).unwrap().log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut bytes = BytesMut::new();
            records.encode_record(index, &mut bytes);
            decode_log_record(&bytes, 0).unwrap().0
        })
        .collect()
}

/// Each record of `oid` as (datum, Status_Flags).
fn logged(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<(LogDatum, Option<StatusFlags>)> {
    records(db, oid)
        .into_iter()
        .map(|record| (record.log_datum, record.status_flags))
        .collect()
}

/// Set AV-1's Present_Value as the application does.
fn set_value(db: &mut ObjectDatabase, value: f32) {
    db.get_mut(&target())
        .unwrap()
        .set_present_value_internal(PropertyValue::Real(value))
        .unwrap();
}

const NORMAL: Option<StatusFlags> = Some(StatusFlags::empty());

fn real(value: f32) -> (LogDatum, Option<StatusFlags>) {
    (LogDatum::RealValue(value), NORMAL)
}

fn status(status: LogStatus) -> (LogDatum, Option<StatusFlags>) {
    (LogDatum::LogStatus(status), None)
}

/// The fixture's Trend Log of AV-1 (at 0.0), put in COV with `increment`.
fn cov_fixture(increment: PropertyValue) -> (ObjectDatabase, ObjectIdentifier, Arc<WallClock>) {
    let (mut db, oid, _, clock) = fixture(0);
    // An AV-1 the application writes directly.
    db.remove(&target()).unwrap();
    db.add(Box::new(
        AnalogValueObject::with_access(
            1,
            "AV",
            EngineeringUnits::NO_UNITS,
            crate::present_value_access::PresentValueAccess::Writable,
        )
        .unwrap(),
    ))
    .unwrap();
    write(
        &mut db,
        oid,
        P::LOGGING_TYPE,
        PropertyValue::Enumerated(LoggingType::COV.to_raw()),
    );
    write(&mut db, oid, P::CLIENT_COV_INCREMENT, increment);
    (db, oid, clock)
}

#[test]
fn a_change_meeting_the_increment_logs_one_record_with_status_flags() {
    let (mut db, oid, _) = cov_fixture(PropertyValue::Real(1.0));
    // The first look logs the value, as a first notification reports it.
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(logged(&db, oid), [real(0.0)]);
    assert_eq!(records(&db, oid)[0].time, frame().local_time);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0)]);
    // 0.5 is short of the increment; 1.5 is 1.5 past the last logged 0.0.
    set_value(&mut db, 0.5);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0)]);
    set_value(&mut db, 1.5);
    db.poll_trend_logs();
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0), real(1.5)]);
    // Measured from the last logged value, not the last one seen: 0.9 down
    // is short, 1.0 down meets it.
    set_value(&mut db, 0.6);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid).len(), 2);
    set_value(&mut db, 0.5);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0), real(1.5), real(0.5)]);
    // A Status_Flags change logs whatever the value did.
    write(
        &mut db,
        target(),
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid)[3],
        (LogDatum::RealValue(0.5), Some(StatusFlags::OUT_OF_SERVICE))
    );
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid).len(), 4);
}

#[test]
fn a_null_increment_uses_the_targets_cov_increment() {
    let (mut db, oid, _) = cov_fixture(PropertyValue::Null);
    write(
        &mut db,
        target(),
        P::COV_INCREMENT,
        PropertyValue::Real(2.0),
    );
    db.poll_trend_logs();
    set_value(&mut db, 1.0);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0)]);
    set_value(&mut db, 2.0);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0), real(2.0)]);
    // Client_COV_Increment wins once set: 0.25 is enough.
    write(
        &mut db,
        oid,
        P::CLIENT_COV_INCREMENT,
        PropertyValue::Real(0.25),
    );
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid).len(), 2);
    set_value(&mut db, 2.25);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0), real(2.0), real(2.25)]);
}

#[test]
fn an_increment_change_alone_keeps_the_baseline() {
    let (mut db, oid, _) = cov_fixture(PropertyValue::Real(1.0));
    db.poll_trend_logs();
    // The new increment is measured from the last logged value: nothing is
    // logged again for the change itself.
    for increment in [PropertyValue::Real(5.0), PropertyValue::Null] {
        write(&mut db, oid, P::CLIENT_COV_INCREMENT, increment);
        db.poll_trend_logs();
        assert_eq!(logged(&db, oid), [real(0.0)]);
    }
    set_value(&mut db, 0.5);
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0), real(0.5)]);
}

#[test]
fn enable_gates_cov_records_and_restarts_from_the_current_value() {
    let (mut db, oid, _) = cov_fixture(PropertyValue::Real(1.0));
    db.poll_trend_logs();
    write(&mut db, oid, P::LOG_ENABLE, PropertyValue::Boolean(false));
    set_value(&mut db, 5.0);
    db.poll_trend_logs();
    // Back within the increment of the last logged 0.0.
    set_value(&mut db, 0.5);
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid),
        [real(0.0), status(LogStatus::LOG_DISABLED)]
    );
    // Collection resumes with the value as it stands, though it is short
    // of the increment from the last one logged.
    write(&mut db, oid, P::LOG_ENABLE, PropertyValue::Boolean(true));
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid),
        [
            real(0.0),
            status(LogStatus::LOG_DISABLED),
            status(LogStatus::empty()),
            real(0.5),
        ]
    );
}

#[test]
fn the_window_gates_cov_records_and_restarts_from_the_current_value() {
    let (mut db, oid, clock) = cov_fixture(PropertyValue::Real(1.0));
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0)]);
    // The window now opens at 12:30; the clock reads 12:00.
    let start = at(12, 30, 0, 0);
    write(
        &mut db,
        oid,
        P::START_TIME,
        PropertyValue::List(vec![
            PropertyValue::Date(start.local_date),
            PropertyValue::Time(start.local_time),
        ]),
    );
    db.poll_trend_logs();
    set_value(&mut db, 3.0);
    db.poll_trend_logs();
    set_value(&mut db, 0.5);
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid),
        [real(0.0), status(LogStatus::LOG_DISABLED)]
    );
    // It opens: the value is logged as it stands, short of the increment.
    set_clock(&clock, Some(at(12, 30, 0, 0)));
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid),
        [
            real(0.0),
            status(LogStatus::LOG_DISABLED),
            status(LogStatus::empty()),
            real(0.5),
        ]
    );
    assert_eq!(records(&db, oid)[3].time, start.local_time);
}

#[test]
fn another_devices_reference_logs_its_failure_once() {
    let (mut db, _, _) = cov_fixture(PropertyValue::Null);
    // The setter takes a reference to another device; a client's write
    // would be held to this one.
    let mut log = TrendLogObject::new(2, "Remote", 8).unwrap();
    log.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: target(),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap()),
    }))
    .unwrap();
    log.set_logging_type(LoggingType::COV).unwrap();
    let remote = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    for value in [1.0, 2.0, 3.0] {
        db.poll_trend_logs();
        set_value(&mut db, value);
    }
    assert_eq!(
        logged(&db, remote),
        [(
            LogDatum::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            },
            None,
        )]
    );
}

#[test]
fn a_cov_log_without_a_clock_looks_again_after_the_retry_delay() {
    let (mut db, oid, time, clock) = fixture(0);
    write(
        &mut db,
        oid,
        P::LOGGING_TYPE,
        PropertyValue::Enumerated(LoggingType::COV.to_raw()),
    );
    set_clock(&clock, None);
    db.poll_trend_logs();
    assert!(logged(&db, oid).is_empty());
    set_clock(&clock, Some(frame()));
    db.poll_trend_logs();
    assert!(logged(&db, oid).is_empty());
    *time.lock().unwrap() = RECONCILE;
    db.poll_trend_logs();
    assert_eq!(logged(&db, oid), [real(0.0)]);
}

/// A second COV Trend Log, instance 2, of `object`'s Present_Value.
fn cov_log_of(db: &mut ObjectDatabase, object: ObjectIdentifier) -> ObjectIdentifier {
    let mut log = TrendLogObject::new(2, "Second", 16).unwrap();
    log.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference::new_local(
        object,
        P::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    log.set_logging_type(LoggingType::COV).unwrap();
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    oid
}

fn set_present_value(db: &mut ObjectDatabase, object: ObjectIdentifier, value: PropertyValue) {
    db.get_mut(&object)
        .unwrap()
        .set_present_value_internal(value)
        .unwrap();
}

#[test]
fn a_binary_value_logs_every_change() {
    let (mut db, _, _) = cov_fixture(PropertyValue::Real(1.0));
    let bv = crate::binary::BinaryValueObject::with_access(
        1,
        "BV",
        crate::present_value_access::PresentValueAccess::Writable,
    )
    .unwrap();
    let bv_oid = bv.object_identifier();
    db.add(Box::new(bv)).unwrap();
    let oid = cov_log_of(&mut db, bv_oid);
    db.poll_trend_logs();
    db.poll_trend_logs();
    // Not numeric, so the increment doesn't apply: each change logs.
    for value in [1, 0, 1] {
        set_present_value(&mut db, bv_oid, PropertyValue::Enumerated(value));
        db.poll_trend_logs();
        db.poll_trend_logs();
    }
    let values: Vec<_> = logged(&db, oid)
        .into_iter()
        .map(|(datum, _)| datum)
        .collect();
    assert_eq!(values, [0, 1, 0, 1].map(LogDatum::EnumValue));
}

#[test]
fn a_target_without_a_cov_increment_logs_every_change_under_a_null_increment() {
    let (mut db, _, _) = cov_fixture(PropertyValue::Real(1.0));
    let msv = crate::multistate::MultiStateValueObject::with_access(
        1,
        "MSV",
        4,
        crate::present_value_access::PresentValueAccess::Writable,
    )
    .unwrap();
    let msv_oid = msv.object_identifier();
    assert_eq!(msv.cov_increment(), None);
    db.add(Box::new(msv)).unwrap();
    // The log's own Client_COV_Increment is NULL by default.
    let oid = cov_log_of(&mut db, msv_oid);
    db.poll_trend_logs();
    for value in [2, 3] {
        set_present_value(&mut db, msv_oid, PropertyValue::Unsigned(value));
        db.poll_trend_logs();
    }
    let values: Vec<_> = logged(&db, oid)
        .into_iter()
        .map(|(datum, _)| datum)
        .collect();
    assert_eq!(values, [1, 2, 3].map(LogDatum::UnsignedValue));
}

#[test]
fn a_target_removed_logs_its_failure_once_then_its_value_again() {
    let (mut db, oid, _) = cov_fixture(PropertyValue::Real(1.0));
    db.poll_trend_logs();
    let removed = db.remove(&target()).unwrap().unwrap();
    db.poll_trend_logs();
    db.poll_trend_logs();
    db.add(removed).unwrap();
    db.poll_trend_logs();
    db.poll_trend_logs();
    assert_eq!(
        logged(&db, oid),
        [
            real(0.0),
            (
                LogDatum::Failure {
                    error_class: ErrorClass::OBJECT,
                    error_code: ErrorCode::UNKNOWN_OBJECT,
                },
                None,
            ),
            real(0.0),
        ]
    );
}
