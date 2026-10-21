use super::*;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::ObjectIdentifier;

fn read(log: &TrendLogMultipleObject, property: P) -> PropertyValue {
    log.read_property(property, None).unwrap()
}

fn member(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

fn noon() -> (Date, Time) {
    (
        SpecificDate::new(2026, 10, 3).unwrap().to_date(),
        Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
    )
}

fn assert_refused(result: Result<TrendLogMultipleObject, Error>, code: ErrorCode) {
    let error = result.err().unwrap();
    let class = if code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY {
        ErrorClass::RESOURCES
    } else {
        ErrorClass::PROPERTY
    };
    assert!(
        matches!(error, Error::Protocol { class: c, code: e }
            if c == class && e == code),
        "{error:?}"
    );
}

#[test]
fn python_add_trend_log_multiple_settings_reach_every_row() {
    let log = trend_log_multiple(
        1,
        "TLM-1",
        50,
        None,
        TrendLogMultipleSettings {
            members: vec![member(1), member(2)],
            log_interval: Some(1_500),
            logging_type: Some(LoggingType::POLLED),
            start_time: Some(noon()),
            stop_time: None,
            align_intervals: Some(true),
            interval_offset: Some(300),
            total_record_count: u32::MAX - 1,
        },
    )
    .unwrap();
    assert_eq!(log.total_record_count(), u32::MAX - 1);
    let mut expected = TrendLogMultipleObject::new(9, "expected", 1).unwrap();
    for instance in [1, 2] {
        expected.add_property_reference(member(instance)).unwrap();
    }
    assert_eq!(
        read(&log, P::LOG_DEVICE_OBJECT_PROPERTY),
        read(&expected, P::LOG_DEVICE_OBJECT_PROPERTY)
    );
    assert_eq!(read(&log, P::LOG_INTERVAL), PropertyValue::Unsigned(1_500));
    assert_eq!(read(&log, P::LOGGING_TYPE), PropertyValue::Enumerated(0));
    assert_eq!(
        read(&log, P::START_TIME),
        PropertyValue::List(vec![
            PropertyValue::Date(noon().0),
            PropertyValue::Time(noon().1)
        ])
    );
    assert_eq!(read(&log, P::ALIGN_INTERVALS), PropertyValue::Boolean(true));
    assert_eq!(read(&log, P::INTERVAL_OFFSET), PropertyValue::Unsigned(300));

    // POLLED alone takes the default interval; TRIGGERED zeroes it.
    let polled = trend_log_multiple(
        2,
        "TLM-2",
        50,
        None,
        TrendLogMultipleSettings {
            logging_type: Some(LoggingType::POLLED),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&polled, P::LOG_INTERVAL),
        PropertyValue::Unsigned(bacnet_objects::trend::DEFAULT_LOG_INTERVAL.into())
    );
    let triggered = trend_log_multiple(
        3,
        "TLM-3",
        50,
        None,
        TrendLogMultipleSettings {
            logging_type: Some(LoggingType::TRIGGERED),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read(&triggered, P::LOGGING_TYPE),
        PropertyValue::Enumerated(2)
    );
    assert_eq!(
        read(&triggered, P::LOG_INTERVAL),
        PropertyValue::Unsigned(0)
    );
    // Omitted arguments keep the object's defaults.
    let plain =
        trend_log_multiple(4, "TLM-4", 50, None, TrendLogMultipleSettings::default()).unwrap();
    assert_eq!(read(&plain, P::LOG_INTERVAL), PropertyValue::Unsigned(0));
    assert_eq!(
        read(&plain, P::LOG_DEVICE_OBJECT_PROPERTY),
        PropertyValue::List(Vec::new())
    );
}

#[test]
fn python_add_trend_log_multiple_settings_are_checked_like_the_rust_setters() {
    let settings = |set: fn(&mut TrendLogMultipleSettings)| {
        let mut settings = TrendLogMultipleSettings::default();
        set(&mut settings);
        trend_log_multiple(1, "TLM-1", 50, None, settings)
    };
    assert_refused(
        settings(|s| s.logging_type = Some(LoggingType::COV)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_refused(
        settings(|s| {
            s.logging_type = Some(LoggingType::TRIGGERED);
            s.log_interval = Some(100);
        }),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_refused(
        settings(|s| {
            s.stop_time = Some((
                Date {
                    year: Date::UNSPECIFIED,
                    ..noon().0
                },
                noon().1,
            ))
        }),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_refused(
        settings(|s| s.members = vec![member(1); 65]),
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert!(logging_type_from_py("cov").is_ok());
    Python::initialize();
    Python::attach(|py| {
        let error = logging_type_from_py("COV").unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py), "{error}");
    });
}

fn read_tl(log: &TrendLogObject, property: P) -> PropertyValue {
    log.read_property(property, None).unwrap()
}

#[test]
fn python_add_trend_log_settings_reach_every_row() {
    let log = trend_log(
        1,
        "TL-1",
        50,
        None,
        TrendLogSettings {
            reference: Some(member(1)),
            logging_type: Some(LoggingType::COV),
            client_cov_increment: Some(0.5),
            cov_resubscription_interval: Some(120),
            start_time: Some(noon()),
            align_intervals: Some(true),
            interval_offset: Some(300),
            total_record_count: 7,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(log.total_record_count(), 7);
    let mut expected = TrendLogObject::new(9, "expected", 1).unwrap();
    expected
        .set_log_device_object_property(Some(member(1)))
        .unwrap();
    assert_eq!(
        read_tl(&log, P::LOG_DEVICE_OBJECT_PROPERTY),
        read_tl(&expected, P::LOG_DEVICE_OBJECT_PROPERTY)
    );
    assert_eq!(read_tl(&log, P::LOGGING_TYPE), PropertyValue::Enumerated(1));
    assert_eq!(read_tl(&log, P::LOG_INTERVAL), PropertyValue::Unsigned(0));
    assert_eq!(
        read_tl(&log, P::CLIENT_COV_INCREMENT),
        PropertyValue::Real(0.5)
    );
    assert_eq!(
        read_tl(&log, P::COV_RESUBSCRIPTION_INTERVAL),
        PropertyValue::Unsigned(120)
    );
    assert_eq!(
        read_tl(&log, P::START_TIME),
        PropertyValue::List(vec![
            PropertyValue::Date(noon().0),
            PropertyValue::Time(noon().1)
        ])
    );
    assert_eq!(
        read_tl(&log, P::ALIGN_INTERVALS),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        read_tl(&log, P::INTERVAL_OFFSET),
        PropertyValue::Unsigned(300)
    );

    // COV with a nonzero interval polls at it (Clause 12.25.9).
    let polled = trend_log(
        2,
        "TL-2",
        50,
        None,
        TrendLogSettings {
            logging_type: Some(LoggingType::COV),
            log_interval: Some(500),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read_tl(&polled, P::LOGGING_TYPE),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        read_tl(&polled, P::LOG_INTERVAL),
        PropertyValue::Unsigned(500)
    );
    // Omitted arguments keep the object's defaults: no reference, a NULL
    // increment.
    let plain = trend_log(3, "TL-3", 50, None, TrendLogSettings::default()).unwrap();
    let fresh = TrendLogObject::new(9, "fresh", 1).unwrap();
    assert_eq!(
        read_tl(&plain, P::LOG_DEVICE_OBJECT_PROPERTY),
        read_tl(&fresh, P::LOG_DEVICE_OBJECT_PROPERTY)
    );
    assert_eq!(
        read_tl(&plain, P::CLIENT_COV_INCREMENT),
        PropertyValue::Null
    );
}

#[test]
fn python_add_trend_log_settings_are_checked_like_the_rust_setters() {
    let refused = |settings: TrendLogSettings, code: ErrorCode| {
        let error = trend_log(1, "TL-1", 50, None, settings).err().unwrap();
        assert!(
            matches!(error, Error::Protocol { class: ErrorClass::PROPERTY, code: e } if e == code),
            "{error:?}"
        );
    };
    refused(
        TrendLogSettings {
            logging_type: Some(LoggingType::TRIGGERED),
            log_interval: Some(100),
            ..Default::default()
        },
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    refused(
        TrendLogSettings {
            client_cov_increment: Some(-1.0),
            ..Default::default()
        },
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    refused(
        TrendLogSettings {
            cov_resubscription_interval: Some(0),
            ..Default::default()
        },
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn python_storage_path_keeps_saved_settings_over_the_keywords() {
    let dir =
        std::env::temp_dir().join(format!("rusty-bacnet-py-trend-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tl-1");
    let path = path.to_str().unwrap();
    let first = TrendLogSettings {
        logging_type: Some(LoggingType::POLLED),
        log_interval: Some(1_500),
        total_record_count: 40,
        ..Default::default()
    };
    let log = trend_log(1, "TL-1", 50, Some(path), first).unwrap();
    assert!(!log.reopened());
    assert_eq!(log.total_record_count(), 40);
    drop(log);

    let second = TrendLogSettings {
        log_interval: Some(9_000),
        client_cov_increment: Some(2.5),
        total_record_count: 7,
        ..Default::default()
    };
    let log = trend_log(1, "TL-1", 50, Some(path), second).unwrap();
    assert!(log.reopened());
    // Saved: the interval and the count. Not saved: the COV increment.
    assert_eq!(
        read_tl(&log, P::LOG_INTERVAL),
        PropertyValue::Unsigned(1_500)
    );
    assert_eq!(log.total_record_count(), 40);
    assert_eq!(
        read_tl(&log, P::CLIENT_COV_INCREMENT),
        PropertyValue::Real(2.5)
    );
    drop(log);
    let _ = std::fs::remove_dir_all(&dir);
}
