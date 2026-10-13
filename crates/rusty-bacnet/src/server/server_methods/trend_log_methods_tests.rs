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
            if c == class.to_raw() as u32 && e == code.to_raw() as u32),
        "{error:?}"
    );
}

#[test]
fn python_add_trend_log_multiple_settings_reach_every_row() {
    let log = trend_log_multiple(
        1,
        "TLM-1",
        50,
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
    let plain = trend_log_multiple(4, "TLM-4", 50, TrendLogMultipleSettings::default()).unwrap();
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
        trend_log_multiple(1, "TLM-1", 50, settings)
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
