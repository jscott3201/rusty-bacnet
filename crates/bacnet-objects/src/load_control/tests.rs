use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

use PropertyIdentifier as P;

fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class, code }
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
fn load_control_actual_shed_level_keeps_the_requested_units() {
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_requested_shed_level(BACnetShedLevel::Amount(40.0))
        .unwrap();
    lc.set_actual_shed_level(BACnetShedLevel::Amount(35.5))
        .unwrap();
    let actual = lc.read_property(P::ACTUAL_SHED_LEVEL, None).unwrap();
    assert_eq!(actual, wire(&BACnetShedLevel::Amount(35.5)));
    for level in [
        BACnetShedLevel::Percent(50),
        BACnetShedLevel::Level(1),
        BACnetShedLevel::Amount(f32::NAN),
    ] {
        assert_error(
            lc.set_actual_shed_level(level).unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            lc.read_property(P::ACTUAL_SHED_LEVEL, None).unwrap(),
            actual
        );
    }
    // Actual_Shed_Level has no network write route.
    assert_error(
        lc.write_property(P::ACTUAL_SHED_LEVEL, None, actual, None)
            .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
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
    let list = lc.property_list();
    assert!(list.contains(&P::PRESENT_VALUE));
    assert!(list.contains(&P::REQUESTED_SHED_LEVEL));
    assert!(list.contains(&P::EXPECTED_SHED_LEVEL));
    assert!(list.contains(&P::ACTUAL_SHED_LEVEL));
    assert!(list.contains(&P::SHED_DURATION));
    assert!(list.contains(&P::START_TIME));
}
