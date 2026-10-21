//! The Averaging sample window (Clause 12.5, #1092): statistics over the most
//! recent Window_Samples samples, the empty-window values, and the writes that
//! discard the samples.
use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

use PropertyIdentifier as P;

fn read(avg: &AveragingObject, property: P) -> PropertyValue {
    avg.read_property(property, None).unwrap()
}

fn write(avg: &mut AveragingObject, property: P, value: PropertyValue) -> Result<(), Error> {
    avg.write_property(property, None, value, None)
}

/// `(minimum, maximum, average, attempted, valid)`, with the REALs as bit
/// patterns so NaN compares equal to itself.
fn statistics(avg: &AveragingObject) -> (u32, u32, u32, PropertyValue, PropertyValue) {
    let real = |property| match read(avg, property) {
        PropertyValue::Real(v) => v.to_bits(),
        other => panic!("{property:?} is a REAL: {other:?}"),
    };
    (
        real(P::MINIMUM_VALUE),
        real(P::MAXIMUM_VALUE),
        real(P::AVERAGE_VALUE),
        read(avg, P::ATTEMPTED_SAMPLES),
        read(avg, P::VALID_SAMPLES),
    )
}

/// The statistics of a window of `count` valid samples.
fn window_of(count: u64, minimum: f32, maximum: f32, average: f32) -> [PropertyValue; 5] {
    let count = PropertyValue::Unsigned(count);
    [
        PropertyValue::Real(minimum),
        PropertyValue::Real(maximum),
        PropertyValue::Real(average),
        count.clone(),
        count,
    ]
}

fn read_all(avg: &AveragingObject) -> [PropertyValue; 5] {
    [
        read(avg, P::MINIMUM_VALUE),
        read(avg, P::MAXIMUM_VALUE),
        read(avg, P::AVERAGE_VALUE),
        read(avg, P::ATTEMPTED_SAMPLES),
        read(avg, P::VALID_SAMPLES),
    ]
}

/// What an empty window reads: no samples counted, positive infinity, negative
/// infinity and the canonical NaN.
fn empty() -> (u32, u32, u32, PropertyValue, PropertyValue) {
    (
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        f32::NAN.to_bits(),
        PropertyValue::Unsigned(0),
        PropertyValue::Unsigned(0),
    )
}

fn assert_error(error: Error, code: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class, code: c }
            if class == ErrorClass::PROPERTY && c == code),
        "expected PROPERTY/{code:?}, got {error:?}"
    );
}

#[test]
fn averaging_statistics_start_at_infinity_nan_and_negative_infinity() {
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    assert_eq!(statistics(&avg), empty());
    assert_eq!(
        read(&avg, P::MINIMUM_VALUE),
        PropertyValue::Real(f32::INFINITY)
    );
    assert_eq!(
        read(&avg, P::MAXIMUM_VALUE),
        PropertyValue::Real(f32::NEG_INFINITY)
    );
    // The window properties read their defaults: 15 samples over 900 s.
    assert_eq!(read(&avg, P::WINDOW_SAMPLES), PropertyValue::Unsigned(15));
    assert_eq!(read(&avg, P::WINDOW_INTERVAL), PropertyValue::Unsigned(900));
}

#[test]
fn averaging_window_keeps_only_the_most_recent_samples() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    write(&mut avg, P::WINDOW_SAMPLES, PropertyValue::Unsigned(4)).unwrap();
    for value in [10.0, 50.0, 20.0, 30.0] {
        avg.add_sample(value).unwrap();
    }
    assert_eq!(read_all(&avg), window_of(4, 10.0, 50.0, 27.5));
    // The fifth sample pushes 10 out, so the minimum rises.
    avg.add_sample(40.0).unwrap();
    assert_eq!(read_all(&avg), window_of(4, 20.0, 50.0, 35.0));
    // The sixth pushes the 50 out, so the maximum falls as well.
    avg.add_sample(5.0).unwrap();
    assert_eq!(read_all(&avg), window_of(4, 5.0, 40.0, 23.75));
}

#[test]
fn averaging_window_average_has_no_drift_over_many_samples() {
    // A thousand samples through the default 15-sample window: the statistics
    // cover 985..=999 exactly, however many samples went before.
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    for value in 0..1000u16 {
        avg.add_sample(f32::from(value)).unwrap();
    }
    assert_eq!(
        read_all(&avg),
        window_of(15, 985.0, 999.0, 992.0),
        "Attempted_Samples and Valid_Samples stop at Window_Samples"
    );
}

#[test]
fn averaging_each_reset_route_discards_the_samples() {
    // [0] analog-input 3, [1] present-value.
    let reference = PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x03, 0x19, 0x55]);
    // Footnote 1 of Table 12-5: a write of any of these rows resets the
    // window, even when it stores the value already there.
    for (property, value) in [
        (P::ATTEMPTED_SAMPLES, PropertyValue::Unsigned(0)),
        (P::OBJECT_PROPERTY_REFERENCE, reference),
        // The unset form (#1417): analog-input 4194303's present-value.
        (
            P::OBJECT_PROPERTY_REFERENCE,
            PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55]),
        ),
        (P::WINDOW_INTERVAL, PropertyValue::Unsigned(900)),
        (P::WINDOW_INTERVAL, PropertyValue::Unsigned(60)),
        (P::WINDOW_SAMPLES, PropertyValue::Unsigned(15)),
        (P::WINDOW_SAMPLES, PropertyValue::Unsigned(30)),
    ] {
        let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
        for sample in [4.0, 8.0, 6.0] {
            avg.add_sample(sample).unwrap();
        }
        assert_ne!(statistics(&avg), empty());
        write(&mut avg, property, value.clone())
            .unwrap_or_else(|e| panic!("{property:?} = {value:?}: {e:?}"));
        assert_eq!(statistics(&avg), empty(), "{property:?} = {value:?}");
        assert_eq!(read(&avg, property), value, "the write itself lands");
        // The next sample starts a fresh window.
        avg.add_sample(2.0).unwrap();
        assert_eq!(read_all(&avg), window_of(1, 2.0, 2.0, 2.0));
    }
}

#[test]
fn averaging_description_write_keeps_the_samples() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.add_sample(7.0).unwrap();
    let before = statistics(&avg);
    write(
        &mut avg,
        P::DESCRIPTION,
        PropertyValue::CharacterString("zone".into()),
    )
    .unwrap();
    assert_eq!(statistics(&avg), before);
}

#[test]
fn averaging_window_writes_refuse_bad_values_and_keep_the_samples() {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    write(&mut avg, P::WINDOW_SAMPLES, PropertyValue::Unsigned(20)).unwrap();
    write(&mut avg, P::WINDOW_INTERVAL, PropertyValue::Unsigned(600)).unwrap();
    avg.add_sample(3.0).unwrap();
    avg.add_sample(9.0).unwrap();
    let before = statistics(&avg);
    for (property, value, code) in [
        // Window_Samples has to be above zero, and the buffer is bounded.
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Unsigned(1_441),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Unsigned(u64::MAX),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_SAMPLES,
            PropertyValue::Signed(4),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // A window with no length has no sample spacing.
        (
            P::WINDOW_INTERVAL,
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_INTERVAL,
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::WINDOW_INTERVAL,
            PropertyValue::Real(60.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // Zero is the only Attempted_Samples a client may write.
        (
            P::ATTEMPTED_SAMPLES,
            PropertyValue::Unsigned(2),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::ATTEMPTED_SAMPLES,
            PropertyValue::Null,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let error = write(&mut avg, property, value.clone()).unwrap_err();
        assert_error(error, code);
        assert_eq!(statistics(&avg), before, "{property:?} = {value:?}");
        assert_eq!(read(&avg, P::WINDOW_SAMPLES), PropertyValue::Unsigned(20));
        assert_eq!(read(&avg, P::WINDOW_INTERVAL), PropertyValue::Unsigned(600));
    }
    // The bound itself and the widest interval are accepted.
    write(&mut avg, P::WINDOW_SAMPLES, PropertyValue::Unsigned(1_440)).unwrap();
    write(
        &mut avg,
        P::WINDOW_INTERVAL,
        PropertyValue::Unsigned(u32::MAX.into()),
    )
    .unwrap();
    assert_eq!(
        read(&avg, P::WINDOW_SAMPLES),
        PropertyValue::Unsigned(1_440)
    );
}
