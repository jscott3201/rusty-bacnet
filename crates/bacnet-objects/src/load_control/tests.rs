use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

use PropertyIdentifier as P;

fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

/// A shed level as a client writes it and the object serves it: one
/// context-tagged CHOICE alternative.
fn wire(level: &BACnetShedLevel) -> PropertyValue {
    shed_level_value(level)
}

/// The three shed levels, in property order.
fn levels(lc: &LoadControlObject) -> [PropertyValue; 3] {
    [
        P::REQUESTED_SHED_LEVEL,
        P::EXPECTED_SHED_LEVEL,
        P::ACTUAL_SHED_LEVEL,
    ]
    .map(|p| lc.read_property(p, None).unwrap())
}

#[test]
fn load_control_create_and_read_defaults() {
    let lc = LoadControlObject::new(1, "LC-1").unwrap();
    assert_eq!(lc.object_name(), "LC-1");
    assert_eq!(
        lc.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn load_control_object_type() {
    let lc = LoadControlObject::new(1, "LC-1").unwrap();
    assert_eq!(
        lc.read_property(P::OBJECT_TYPE, None).unwrap(),
        PropertyValue::Enumerated(ObjectType::LOAD_CONTROL.to_raw())
    );
}

#[test]
fn load_control_shed_levels_start_at_level_zero_in_their_choice_form() {
    let lc = LoadControlObject::new(1, "LC-1").unwrap();
    // level [1] holding 0, the default of the LEVEL choice.
    let level_zero = PropertyValue::ApplicationData(vec![0x19, 0x00]);
    assert_eq!(levels(&lc), [0, 1, 2].map(|_| level_zero.clone()));
}

#[test]
fn load_control_write_requested_shed_level_takes_each_choice() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    // (written level, its bytes, the choice default Expected and Actual take)
    let cases = [
        (
            BACnetShedLevel::Percent(80),
            vec![0x09, 0x50],
            vec![0x09, 0x64],
        ),
        (
            BACnetShedLevel::Level(3),
            vec![0x19, 0x03],
            vec![0x19, 0x00],
        ),
        (
            BACnetShedLevel::Amount(12.5),
            vec![0x2C, 0x41, 0x48, 0x00, 0x00],
            vec![0x2C, 0x00, 0x00, 0x00, 0x00],
        ),
        // The edges of the accepted ranges, and the defaults themselves.
        (
            BACnetShedLevel::Percent(100),
            vec![0x09, 0x64],
            vec![0x09, 0x64],
        ),
        (
            BACnetShedLevel::Percent(0),
            vec![0x09, 0x00],
            vec![0x09, 0x64],
        ),
        (
            BACnetShedLevel::Level(u64::MAX),
            vec![0x1D, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            vec![0x19, 0x00],
        ),
        (
            BACnetShedLevel::Level(0),
            vec![0x19, 0x00],
            vec![0x19, 0x00],
        ),
        (
            BACnetShedLevel::Amount(0.0),
            vec![0x2C, 0x00, 0x00, 0x00, 0x00],
            vec![0x2C, 0x00, 0x00, 0x00, 0x00],
        ),
    ];
    for (level, bytes, default) in cases {
        let value = PropertyValue::ApplicationData(bytes);
        lc.write_property(P::REQUESTED_SHED_LEVEL, None, value.clone(), None)
            .unwrap();
        let default = PropertyValue::ApplicationData(default);
        assert_eq!(levels(&lc), [value, default.clone(), default], "{level:?}");
    }
}

#[test]
fn load_control_write_requested_shed_level_refuses_other_forms() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    let before = levels(&lc);
    let refused = [
        // The forms served and taken before #1133.
        PropertyValue::Unsigned(50),
        PropertyValue::Real(25.5),
        PropertyValue::List(vec![PropertyValue::Unsigned(50)]),
        PropertyValue::List(vec![PropertyValue::Real(25.5)]),
        PropertyValue::Null,
        // A context tag the CHOICE doesn't have, a framed alternative, an
        // empty Unsigned, a short REAL, and a second value after the first.
        PropertyValue::ApplicationData(vec![0x39, 0x01]),
        PropertyValue::ApplicationData(vec![0x0E, 0x21, 0x32, 0x0F]),
        PropertyValue::ApplicationData(vec![0x08]),
        PropertyValue::ApplicationData(vec![0x2B, 0x41, 0x48, 0x00]),
        PropertyValue::ApplicationData(vec![0x19, 0x01, 0x19, 0x02]),
        PropertyValue::ApplicationData(vec![]),
    ];
    for value in refused {
        assert_error(
            lc.write_property(P::REQUESTED_SHED_LEVEL, None, value.clone(), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(levels(&lc), before, "{value:?}");
    }
}

#[test]
fn load_control_requested_shed_level_refuses_more_load_than_the_baseline() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_requested_shed_level(BACnetShedLevel::Percent(70))
        .unwrap();
    let before = levels(&lc);
    for level in [
        BACnetShedLevel::Percent(101),
        BACnetShedLevel::Percent(u64::MAX),
        BACnetShedLevel::Amount(-0.5),
        BACnetShedLevel::Amount(f32::NEG_INFINITY),
        BACnetShedLevel::Amount(f32::INFINITY),
        BACnetShedLevel::Amount(f32::NAN),
    ] {
        assert_error(
            lc.write_property(P::REQUESTED_SHED_LEVEL, None, wire(&level), None)
                .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_error(
            lc.set_requested_shed_level(level.clone()).unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(levels(&lc), before, "{level:?}");
    }
}

#[test]
fn load_control_write_shed_duration() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.write_property(P::SHED_DURATION, None, PropertyValue::Unsigned(3600), None)
        .unwrap();
    assert_eq!(
        lc.read_property(P::SHED_DURATION, None).unwrap(),
        PropertyValue::Unsigned(3600)
    );
}

#[test]
fn load_control_read_start_time() {
    let lc = LoadControlObject::new(1, "LC-1").unwrap();
    let val = lc.read_property(P::START_TIME, None).unwrap();
    let unspec_date = Date {
        year: 0xFF,
        month: 0xFF,
        day: 0xFF,
        day_of_week: 0xFF,
    };
    let unspec_time = Time {
        hour: 0xFF,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::Date(unspec_date),
            PropertyValue::Time(unspec_time),
        ])
    );
}

#[test]
fn load_control_property_list() {
    let lc = LoadControlObject::new(1, "LC-1").unwrap();
    let list = crate::test_rows::listed(&lc.property_metadata());
    assert!(list.contains(&P::PRESENT_VALUE));
    assert!(list.contains(&P::REQUESTED_SHED_LEVEL));
    assert!(list.contains(&P::EXPECTED_SHED_LEVEL));
    assert!(list.contains(&P::ACTUAL_SHED_LEVEL));
    assert!(list.contains(&P::SHED_DURATION));
    assert!(list.contains(&P::START_TIME));
    for p in [
        P::STATUS_FLAGS,
        P::DUTY_WINDOW,
        P::ENABLE,
        P::SHED_LEVELS,
        P::SHED_LEVEL_DESCRIPTIONS,
    ] {
        assert!(list.contains(&p), "{p:?}");
    }
}

/// A Start_Time as a client writes it: an application Date then Time.
fn date_time(date: Date, time: Time) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

const DAY: Date = Date {
    year: 126,
    month: 11,
    day: 11,
    day_of_week: 3,
};

const TEN: Time = Time {
    hour: 10,
    minute: 0,
    second: 0,
    hundredths: 0,
};

#[test]
fn load_control_start_time_takes_a_moment_or_the_unspecified_value() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    // Requested_Shed_Level at its default cancels a request at once, so a
    // Start_Time is kept only behind a level to shed.
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    // Unspecified seconds and hundredths count as zero, and the weekday may
    // be left unspecified.
    let loose = Time {
        second: 0xFF,
        hundredths: 0xFF,
        ..TEN
    };
    let any_day = Date {
        day_of_week: 0xFF,
        ..DAY
    };
    for written in [date_time(DAY, TEN), date_time(any_day, loose)] {
        lc.write_property(P::START_TIME, None, written.clone(), None)
            .unwrap();
        assert_eq!(lc.read_property(P::START_TIME, None).unwrap(), written);
    }
    // A weekday that disagrees with the date is taken and served as
    // written: the date alone names the day.
    let wrong_weekday = date_time(
        Date {
            day_of_week: 6,
            ..DAY
        },
        TEN,
    );
    lc.write_property(P::START_TIME, None, wrong_weekday.clone(), None)
        .unwrap();
    assert_eq!(
        lc.read_property(P::START_TIME, None).unwrap(),
        wrong_weekday
    );
    let kept = lc.read_property(P::START_TIME, None).unwrap();
    let refused = [
        // Partly unspecified: no moment.
        (
            date_time(Date { year: 0xFF, ..DAY }, TEN),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            date_time(
                DAY,
                Time {
                    minute: 0xFF,
                    ..TEN
                },
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            date_time(Date { day: 31, ..DAY }, TEN),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            date_time(
                Date {
                    day_of_week: 8,
                    ..DAY
                },
                TEN,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // Not a BACnetDateTime.
        (PropertyValue::Date(DAY), ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::List(vec![PropertyValue::Time(TEN), PropertyValue::Date(DAY)]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
    ];
    for (value, code) in refused {
        assert_error(
            lc.write_property(P::START_TIME, None, value.clone(), None)
                .unwrap_err(),
            code,
        );
        assert_eq!(
            lc.read_property(P::START_TIME, None).unwrap(),
            kept,
            "{value:?}"
        );
    }
    assert_error(
        lc.set_start_time((DAY, Time { hour: 24, ..TEN }))
            .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    // A Start_Time is an index-free BACnetDateTime.
    assert_error(
        lc.write_property(P::START_TIME, Some(1), date_time(DAY, TEN), None)
            .unwrap_err(),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
}

#[test]
fn load_control_duty_window_and_enable_take_their_datatypes() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.write_property(P::DUTY_WINDOW, None, PropertyValue::Unsigned(15), None)
        .unwrap();
    assert_eq!(
        lc.read_property(P::DUTY_WINDOW, None).unwrap(),
        PropertyValue::Unsigned(15)
    );
    lc.write_property(P::ENABLE, None, PropertyValue::Boolean(false), None)
        .unwrap();
    assert_eq!(
        lc.read_property(P::ENABLE, None).unwrap(),
        PropertyValue::Boolean(false)
    );
    for (p, wrong) in [
        (P::DUTY_WINDOW, PropertyValue::Real(15.0)),
        (P::DUTY_WINDOW, PropertyValue::Boolean(true)),
        (P::SHED_DURATION, PropertyValue::Signed(5)),
        (P::ENABLE, PropertyValue::Unsigned(1)),
        (P::ENABLE, PropertyValue::Null),
    ] {
        let before = lc.read_property(p, None).unwrap();
        assert_error(
            lc.write_property(p, None, wrong, None).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(lc.read_property(p, None).unwrap(), before);
    }
    for p in [P::DUTY_WINDOW, P::ENABLE] {
        assert_error(
            lc.read_property(p, Some(0)).unwrap_err(),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
    }
}

#[test]
fn load_control_shed_levels_take_elements_but_keep_their_size() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_shed_levels(vec![
        (1, "Dim the lights".into()),
        (2, "Raise cooling setpoints".into()),
        (3, "Stop the chillers".into()),
    ])
    .unwrap();
    let levels = |lc: &LoadControlObject| lc.read_property(P::SHED_LEVELS, None).unwrap();
    let unsigned = |values: &[u64]| {
        PropertyValue::List(values.iter().map(|&v| PropertyValue::Unsigned(v)).collect())
    };
    assert_eq!(levels(&lc), unsigned(&[1, 2, 3]));
    assert_eq!(
        lc.read_property(P::SHED_LEVEL_DESCRIPTIONS, Some(2))
            .unwrap(),
        PropertyValue::CharacterString("Raise cooling setpoints".into())
    );
    assert_eq!(
        lc.read_property(P::SHED_LEVEL_DESCRIPTIONS, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    // One element, then the whole array at its size.
    lc.write_property(P::SHED_LEVELS, Some(2), PropertyValue::Unsigned(20), None)
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[1, 20, 3]));
    lc.write_property(P::SHED_LEVELS, None, unsigned(&[10, 30, 50]), None)
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[10, 30, 50]));
    assert_eq!(
        lc.read_property(P::SHED_LEVELS, Some(3)).unwrap(),
        PropertyValue::Unsigned(50)
    );
    let refused = [
        // The size is the embedder's: not through index 0, nor a whole
        // array of another size.
        (
            Some(0),
            PropertyValue::Unsigned(4),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (None, unsigned(&[1, 2]), ErrorCode::VALUE_OUT_OF_RANGE),
        (None, unsigned(&[1, 2, 3, 4]), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            None,
            PropertyValue::Unsigned(1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            None,
            PropertyValue::List(vec![]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            Some(4),
            PropertyValue::Unsigned(4),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        (
            Some(u32::MAX),
            PropertyValue::Unsigned(4),
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        // Elements are Unsigned.
        (
            Some(1),
            PropertyValue::Real(1.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            None,
            PropertyValue::List(vec![
                PropertyValue::Unsigned(1),
                PropertyValue::Signed(2),
                PropertyValue::Unsigned(3),
            ]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // Level 0 is no shed, which the array doesn't hold.
        (
            Some(2),
            PropertyValue::Unsigned(0),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (None, unsigned(&[10, 0, 50]), ErrorCode::VALUE_OUT_OF_RANGE),
    ];
    for (index, value, code) in refused {
        assert_error(
            lc.write_property(P::SHED_LEVELS, index, value.clone(), None)
                .unwrap_err(),
            code,
        );
        assert_eq!(levels(&lc), unsigned(&[10, 30, 50]), "{index:?} {value:?}");
    }
    assert_error(
        lc.read_property(P::SHED_LEVELS, Some(4)).unwrap_err(),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    // The embedder's setter refuses a 0 the same way, changing nothing.
    assert_error(
        lc.set_shed_levels(vec![(0, "none".into()), (1, "one".into())])
            .unwrap_err(),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(levels(&lc), unsigned(&[10, 30, 50]));
    // The order isn't checked.
    lc.write_property(P::SHED_LEVELS, None, unsigned(&[50, 10, 30]), None)
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[50, 10, 30]));
    // A one-element array takes its element written whole, as the
    // WriteProperty decoder passes it on.
    lc.set_shed_levels(vec![(1, "Dim the lights".into())])
        .unwrap();
    lc.write_property(P::SHED_LEVELS, None, PropertyValue::Unsigned(7), None)
        .unwrap();
    assert_eq!(levels(&lc), unsigned(&[7]));
}
