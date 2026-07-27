//! Date, Time, DateTime and the three non-commandable Pattern value tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_types::enums::ObjectType;

// DateValueObject
// -----------------------------------------------------------------------

#[test]
fn date_value_read_write() {
    let mut obj = DateValueObject::new(1, "DV-1").unwrap();
    let d = Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d));
}

#[test]
fn date_value_priority_array() {
    let mut obj = DateValueObject::new(1, "DV-1").unwrap();
    let d1 = Date {
        year: 124,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    let d2 = Date {
        year: 124,
        month: 12,
        day: 25,
        day_of_week: 3,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d1),
        Some(16),
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d2),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d2));
}

// -----------------------------------------------------------------------
// TimeValueObject
// -----------------------------------------------------------------------

#[test]
fn time_value_read_write() {
    let mut obj = TimeValueObject::new(1, "TV-1").unwrap();
    let t = Time {
        hour: 14,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Time(t),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Time(t));
}

#[test]
fn time_value_object_type() {
    let obj = TimeValueObject::new(1, "TV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::TIME_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// DateTimeValueObject
// -----------------------------------------------------------------------

#[test]
fn datetime_value_read_write() {
    let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let d = Date {
        year: 124,
        month: 6,
        day: 15,
        day_of_week: 6,
    };
    let t = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
    );
}

#[test]
fn datetime_value_object_type() {
    let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATETIME_VALUE.to_raw())
    );
}

#[test]
fn datetime_value_priority_array() {
    let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let d1 = Date {
        year: 124,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    let t1 = Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let d2 = Date {
        year: 124,
        month: 12,
        day: 31,
        day_of_week: 2,
    };
    let t2 = Time {
        hour: 23,
        minute: 59,
        second: 59,
        hundredths: 99,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d1), PropertyValue::Time(t1)]),
        Some(16),
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)]),
        Some(4),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)])
    );
}

// -----------------------------------------------------------------------
// DatePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn date_pattern_value_read_write() {
    let mut obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let d = Date {
        year: 0xFF,
        month: 0xFF,
        day: 25,
        day_of_week: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d));
}

#[test]
fn date_pattern_value_object_type() {
    let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATEPATTERN_VALUE.to_raw())
    );
}

#[test]
fn date_pattern_value_has_priority_array() {
    let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// -----------------------------------------------------------------------
// TimePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn time_pattern_value_read_write() {
    let mut obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
    let t = Time {
        hour: 12,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Time(t),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Time(t));
}

#[test]
fn time_pattern_value_object_type() {
    let obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::TIMEPATTERN_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// DateTimePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn datetime_pattern_value_read_write() {
    let mut obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let d = Date {
        year: 0xFF,
        month: 12,
        day: 25,
        day_of_week: 0xFF,
    };
    let t = Time {
        hour: 0xFF,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
    );
}

#[test]
fn datetime_pattern_value_object_type() {
    let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATETIMEPATTERN_VALUE.to_raw())
    );
}

#[test]
fn datetime_pattern_value_has_priority_array() {
    let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// -----------------------------------------------------------------------
