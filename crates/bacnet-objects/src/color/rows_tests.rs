//! The values the colour objects' rows take (#1474): Present_Value on both
//! (Clauses 12.X.4 and 12.Y.4), the defaults and their ranges (12.X.8,
//! 12.X.10, 12.Y.8 to 12.Y.12), the limits (12.Y.13, 12.Y.14) and
//! Transition (12.X.11, 12.Y.15).

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};

fn assert_error(result: Result<(), Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn xy(x: f32, y: f32) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Real(x), PropertyValue::Real(y)])
}

fn write(object: &mut dyn BACnetObject, p: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(p, None, value, None)
}

fn read(object: &dyn BACnetObject, p: P) -> PropertyValue {
    object.read_property(p, None).unwrap()
}

#[test]
fn color_present_value_takes_an_xy_colour_within_the_unit_square() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    for (x, y) in [(0.0, 0.0), (1.0, 1.0), (0.64, 0.33)] {
        write(&mut color, P::PRESENT_VALUE, xy(x, y)).unwrap();
        assert_eq!(read(&color, P::PRESENT_VALUE), xy(x, y));
        // Transition NONE: the output is there at once.
        assert_eq!(read(&color, P::TRACKING_VALUE), xy(x, y));
    }
    for (x, y) in [(-0.01, 0.5), (0.5, 1.01), (f32::NAN, 0.5)] {
        assert_error(
            write(&mut color, P::PRESENT_VALUE, xy(x, y)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for wrong in [
        PropertyValue::Real(0.5),
        PropertyValue::List(vec![PropertyValue::Real(0.5)]),
        PropertyValue::List(vec![PropertyValue::Real(0.5); 3]),
        PropertyValue::List(vec![PropertyValue::Unsigned(0), PropertyValue::Real(0.5)]),
    ] {
        assert_error(
            write(&mut color, P::PRESENT_VALUE, wrong),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(read(&color, P::PRESENT_VALUE), xy(0.64, 0.33));
    // The setter checks the same way.
    assert_error(
        color.set_present_value(BACnetXyColor::new(2.0, 0.0)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    color
        .set_present_value(BACnetXyColor::new(0.15, 0.06))
        .unwrap();
    assert_eq!(read(&color, P::PRESENT_VALUE), xy(0.15, 0.06));
}

#[test]
fn color_default_color_takes_any_colour_in_range_origin_included() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    write(&mut color, P::DEFAULT_COLOR, xy(0.0, 0.0)).unwrap();
    assert_eq!(read(&color, P::DEFAULT_COLOR), xy(0.0, 0.0));
    assert_error(
        write(&mut color, P::DEFAULT_COLOR, xy(1.5, 0.0)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&color, P::DEFAULT_COLOR), xy(0.0, 0.0));
}

#[test]
fn default_fade_time_starts_at_100_and_takes_100_ms_to_a_day() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(ColorObject::new(1, "CLR-1").unwrap()),
        Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
    ];
    for mut object in objects {
        assert_eq!(
            read(object.as_ref(), P::DEFAULT_FADE_TIME),
            PropertyValue::Unsigned(100)
        );
        for ms in [100, 86_400_000] {
            write(
                object.as_mut(),
                P::DEFAULT_FADE_TIME,
                PropertyValue::Unsigned(ms),
            )
            .unwrap();
            assert_eq!(
                read(object.as_ref(), P::DEFAULT_FADE_TIME),
                PropertyValue::Unsigned(ms)
            );
        }
        for ms in [0, 99, 86_400_001, u64::MAX] {
            assert_error(
                write(
                    object.as_mut(),
                    P::DEFAULT_FADE_TIME,
                    PropertyValue::Unsigned(ms),
                ),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_error(
            write(
                object.as_mut(),
                P::DEFAULT_FADE_TIME,
                PropertyValue::Real(500.0),
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(
            read(object.as_ref(), P::DEFAULT_FADE_TIME),
            PropertyValue::Unsigned(86_400_000)
        );
    }
}

#[test]
fn color_temperature_present_value_clamps_inside_1000_to_30000() {
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    ct.set_min_max(2_000, 6_500).unwrap();
    for (written, stored) in [
        (1_000, 2_000),
        (1_999, 2_000),
        (2_000, 2_000),
        (4_321, 4_321),
        (6_500, 6_500),
        (6_501, 6_500),
        (30_000, 6_500),
    ] {
        write(&mut ct, P::PRESENT_VALUE, PropertyValue::Unsigned(written)).unwrap();
        assert_eq!(
            read(&ct, P::PRESENT_VALUE),
            PropertyValue::Unsigned(stored),
            "{written}"
        );
        assert_eq!(
            read(&ct, P::TRACKING_VALUE),
            PropertyValue::Unsigned(stored)
        );
    }
    // Outside the object's whole range is refused, and changes nothing.
    for refused in [0, 999, 30_001, 0x1_0000_03E8] {
        assert_error(
            write(&mut ct, P::PRESENT_VALUE, PropertyValue::Unsigned(refused)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_error(
        write(&mut ct, P::PRESENT_VALUE, PropertyValue::Real(4_000.0)),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(read(&ct, P::PRESENT_VALUE), PropertyValue::Unsigned(6_500));
    assert_error(ct.set_present_value(999), ErrorCode::VALUE_OUT_OF_RANGE);
    ct.set_present_value(1_500).unwrap();
    assert_eq!(read(&ct, P::PRESENT_VALUE), PropertyValue::Unsigned(2_000));
}

#[test]
fn color_temperature_limits_stay_within_1000_to_30000_and_move_the_value() {
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    for (min, max) in [(999, 6_500), (2_000, 30_001), (6_500, 2_000)] {
        assert_error(ct.set_min_max(min, max), ErrorCode::VALUE_OUT_OF_RANGE);
    }
    assert_eq!(read(&ct, P::MIN_PRES_VALUE), PropertyValue::Unsigned(1_000));
    assert_eq!(
        read(&ct, P::MAX_PRES_VALUE),
        PropertyValue::Unsigned(30_000)
    );
    // 4000 K lies above the new maximum, so it moves down to it, and so does
    // the default.
    ct.set_min_max(2_700, 3_000).unwrap();
    assert_eq!(read(&ct, P::PRESENT_VALUE), PropertyValue::Unsigned(3_000));
    assert_eq!(read(&ct, P::TRACKING_VALUE), PropertyValue::Unsigned(3_000));
    assert_eq!(
        read(&ct, P::DEFAULT_COLOR_TEMPERATURE),
        PropertyValue::Unsigned(3_000)
    );
    ct.set_min_max(3_000, 3_000).unwrap();
}

#[test]
fn color_temperature_defaults_take_their_ranges() {
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    ct.set_min_max(2_000, 6_500).unwrap();
    // Default_Color_Temperature clamps as Present_Value does, and keeps 0.
    for (written, stored) in [(0, 0), (1_000, 2_000), (5_000, 5_000), (30_000, 6_500)] {
        write(
            &mut ct,
            P::DEFAULT_COLOR_TEMPERATURE,
            PropertyValue::Unsigned(written),
        )
        .unwrap();
        assert_eq!(
            read(&ct, P::DEFAULT_COLOR_TEMPERATURE),
            PropertyValue::Unsigned(stored)
        );
    }
    for refused in [999, 30_001] {
        assert_error(
            write(
                &mut ct,
                P::DEFAULT_COLOR_TEMPERATURE,
                PropertyValue::Unsigned(refused),
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    // Default_Ramp_Rate and Default_Step_Increment take 1 to 30000.
    for p in [P::DEFAULT_RAMP_RATE, P::DEFAULT_STEP_INCREMENT] {
        for kelvin in [1, 30_000] {
            write(&mut ct, p, PropertyValue::Unsigned(kelvin)).unwrap();
            assert_eq!(read(&ct, p), PropertyValue::Unsigned(kelvin));
        }
        for refused in [0, 30_001] {
            assert_error(
                write(&mut ct, p, PropertyValue::Unsigned(refused)),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_error(
            write(&mut ct, p, PropertyValue::Signed(5)),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
}

#[test]
fn transition_takes_the_kinds_each_object_has() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    for object in [&mut color as &mut dyn BACnetObject, &mut ct] {
        assert_eq!(read(object, P::TRANSITION), PropertyValue::Enumerated(0));
        write(object, P::TRANSITION, PropertyValue::Enumerated(1)).unwrap();
        assert_eq!(read(object, P::TRANSITION), PropertyValue::Enumerated(1));
        assert_error(
            write(object, P::TRANSITION, PropertyValue::Unsigned(0)),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_error(
            write(object, P::TRANSITION, PropertyValue::Enumerated(3)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    // A Color object has no ramp.
    assert_error(
        write(&mut color, P::TRANSITION, PropertyValue::Enumerated(2)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    write(&mut ct, P::TRANSITION, PropertyValue::Enumerated(2)).unwrap();
    assert_eq!(read(&ct, P::TRANSITION), PropertyValue::Enumerated(2));
}
