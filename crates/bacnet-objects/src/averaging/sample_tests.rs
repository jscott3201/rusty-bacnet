//! The application route that feeds a stored Averaging object (#1083).
//!
//! `add_averaging_sample_internal` takes each sample the application took and
//! converts it to REAL as Clause 12.5 describes, or records an attempt that
//! produced no value, so a running server can feed the object without
//! reaching its concrete type.
use super::*;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn read(avg: &AveragingObject, property: PropertyIdentifier) -> PropertyValue {
    avg.read_property(property, None).unwrap()
}

/// `(minimum, maximum, average, attempted, valid)` as read.
fn statistics(avg: &AveragingObject) -> [PropertyValue; 5] {
    [
        read(avg, PropertyIdentifier::MINIMUM_VALUE),
        read(avg, PropertyIdentifier::MAXIMUM_VALUE),
        read(avg, PropertyIdentifier::AVERAGE_VALUE),
        read(avg, PropertyIdentifier::ATTEMPTED_SAMPLES),
        read(avg, PropertyIdentifier::VALID_SAMPLES),
    ]
}

fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
            if c == class && e == code),
        "expected {class:?}/{code:?}, got {error:?}"
    );
}

#[test]
fn averaging_sample_hook_converts_each_sampled_datatype_to_real() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    for sample in [
        PropertyValue::Real(2.5),
        PropertyValue::Unsigned(9),
        PropertyValue::Signed(-4),
        PropertyValue::Enumerated(3),
        PropertyValue::Boolean(true),
        PropertyValue::Boolean(false),
    ] {
        avg.add_averaging_sample_internal(Some(sample)).unwrap();
    }
    // 2.5 + 9 - 4 + 3 + 1 + 0 = 11.5 over six samples.
    let [minimum, maximum, average, attempted, valid] = statistics(&avg);
    assert_eq!(minimum, PropertyValue::Real(-4.0));
    assert_eq!(maximum, PropertyValue::Real(9.0));
    let PropertyValue::Real(average) = average else {
        panic!("Average_Value is a REAL: {average:?}");
    };
    assert!((average - 11.5 / 6.0).abs() < 1e-5, "{average}");
    assert_eq!(attempted, PropertyValue::Unsigned(6));
    assert_eq!(valid, PropertyValue::Unsigned(6));
}

#[test]
fn averaging_sample_hook_refuses_other_datatypes_and_non_finite_reals() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_averaging_sample_internal(Some(PropertyValue::Real(20.0)))
        .unwrap();
    let before = statistics(&avg);
    for (value, code) in [
        (PropertyValue::Double(1.0), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::CharacterString("21".into()),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PropertyValue::List(vec![PropertyValue::Real(1.0)]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Real(f32::NEG_INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        let error = avg.add_averaging_sample_internal(Some(value)).unwrap_err();
        assert_error(error, ErrorClass::PROPERTY, code);
    }
    // The direct setter applies the same finite check.
    let error = avg.add_sample(f32::NAN).unwrap_err();
    assert_error(error, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(
        statistics(&avg),
        before,
        "a refused sample counts as neither attempted nor valid"
    );
}

#[test]
fn averaging_sample_hook_is_refused_by_other_objects() {
    let mut av =
        crate::analog::AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS)
            .unwrap();
    let error = av
        .add_averaging_sample_internal(Some(PropertyValue::Real(1.0)))
        .unwrap_err();
    assert_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

#[test]
fn averaging_takes_property_cov_subscriptions_but_not_subscribe_cov() {
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert!(!avg.supports_cov(), "Table 13-1 has no Averaging row");
    assert!(avg.supports_subscribe_cov_property());
    for property in [
        PropertyIdentifier::MINIMUM_VALUE,
        PropertyIdentifier::MAXIMUM_VALUE,
        PropertyIdentifier::AVERAGE_VALUE,
        PropertyIdentifier::ATTEMPTED_SAMPLES,
        PropertyIdentifier::VALID_SAMPLES,
    ] {
        assert!(avg.supports_cov_property(property), "{property:?}");
    }
    // Objects that take SubscribeCOV keep taking property subscriptions, and
    // objects that take neither still refuse both.
    let av = crate::analog::AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS)
        .unwrap();
    assert!(av.supports_cov() && av.supports_subscribe_cov_property());
    let file = crate::file::FileObject::new(1, "FILE-1", "text/plain").unwrap();
    assert!(!file.supports_cov() && !file.supports_subscribe_cov_property());
    assert!(!file.supports_cov_property(PropertyIdentifier::FILE_SIZE));
}

#[test]
fn averaging_missed_samples_count_as_attempted_but_not_valid() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.set_window_samples(3).unwrap();
    avg.add_averaging_sample_internal(Some(PropertyValue::Real(10.0)))
        .unwrap();
    avg.add_averaging_sample_internal(None).unwrap();
    avg.add_averaging_sample_internal(Some(PropertyValue::Real(20.0)))
        .unwrap();
    // 10, a miss and 20: the statistics cover the two valid slots.
    assert_eq!(
        statistics(&avg),
        [
            PropertyValue::Real(10.0),
            PropertyValue::Real(20.0),
            PropertyValue::Real(15.0),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(2),
        ]
    );

    // The direct setter records a miss the same way; the 10 leaves the window.
    avg.add_missed_sample();
    assert_eq!(
        statistics(&avg),
        [
            PropertyValue::Real(20.0),
            PropertyValue::Real(20.0),
            PropertyValue::Real(20.0),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(1),
        ]
    );

    // Once only misses are left, the statistics return to their empty-window
    // values while Attempted_Samples stays at Window_Samples.
    avg.add_averaging_sample_internal(None).unwrap();
    avg.add_averaging_sample_internal(None).unwrap();
    let [minimum, maximum, average, attempted, valid] = statistics(&avg);
    assert_eq!(minimum, PropertyValue::Real(f32::INFINITY));
    assert_eq!(maximum, PropertyValue::Real(f32::NEG_INFINITY));
    assert!(matches!(average, PropertyValue::Real(v) if v.to_bits() == f32::NAN.to_bits()));
    assert_eq!(attempted, PropertyValue::Unsigned(3));
    assert_eq!(valid, PropertyValue::Unsigned(0));
}

#[test]
fn averaging_window_setters_validate_and_reset_like_writes() {
    type Setter = fn(&mut AveragingObject, &BACnetObjectPropertyReference);
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_eq!(avg.window_samples(), DEFAULT_WINDOW_SAMPLES);
    assert_eq!(avg.window_interval(), DEFAULT_WINDOW_INTERVAL);
    avg.add_sample(5.0).unwrap();
    let before = statistics(&avg);
    let refusals = [
        avg.set_window_samples(0),
        avg.set_window_samples(MAX_WINDOW_SAMPLES + 1),
        avg.set_window_samples(u32::MAX),
        avg.set_window_interval(0),
    ];
    for result in refusals {
        assert_error(
            result.unwrap_err(),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(statistics(&avg), before, "a refused setter changes nothing");
    assert_eq!(avg.window_samples(), DEFAULT_WINDOW_SAMPLES);
    assert_eq!(avg.window_interval(), DEFAULT_WINDOW_INTERVAL);

    // Each accepted setter discards the samples, as the matching write does.
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    let reference =
        BACnetObjectPropertyReference::new(oid, PropertyIdentifier::PRESENT_VALUE.to_raw());
    let setters: [(&str, Setter); 3] = [
        ("samples", |avg, _| {
            avg.set_window_samples(MAX_WINDOW_SAMPLES).unwrap()
        }),
        ("interval", |avg, _| avg.set_window_interval(60).unwrap()),
        ("reference", |avg, reference| {
            avg.set_object_property_reference(Some(reference.clone()))
        }),
    ];
    for (label, set) in setters {
        avg.add_sample(5.0).unwrap();
        set(&mut avg, &reference);
        let [.., attempted, valid] = statistics(&avg);
        assert_eq!(attempted, PropertyValue::Unsigned(0), "{label}");
        assert_eq!(valid, PropertyValue::Unsigned(0), "{label}");
    }
    assert_eq!(avg.window_samples(), MAX_WINDOW_SAMPLES);
    assert_eq!(avg.window_interval(), 60);

    // The buffer never grows past Window_Samples.
    for value in 0..MAX_WINDOW_SAMPLES + 5 {
        avg.add_sample(value as f32).unwrap();
    }
    let [minimum, maximum, _, attempted, valid] = statistics(&avg);
    assert_eq!(minimum, PropertyValue::Real(5.0));
    assert_eq!(
        maximum,
        PropertyValue::Real((MAX_WINDOW_SAMPLES + 4) as f32)
    );
    assert_eq!(
        attempted,
        PropertyValue::Unsigned(MAX_WINDOW_SAMPLES.into())
    );
    assert_eq!(valid, PropertyValue::Unsigned(MAX_WINDOW_SAMPLES.into()));
}
