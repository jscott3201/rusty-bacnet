//! Current_Command_Priority on all 12 commandable value types, and
//! COV_Increment on Integer, Positive Integer and Large Analog Value (#1111).
//!
//! Footnote 2 of Tables 12-44 to 12-55 requires Current_Command_Priority
//! wherever Present_Value is commandable, which it is on every value type
//! here. Footnote 3 of Tables 12-46, 12-50 and 12-51 requires COV_Increment
//! on an object that reports COV; its datatype is Unsigned on the two integer
//! types and Double on Large Analog Value.

use super::super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn command(object: &mut dyn BACnetObject, value: PropertyValue, priority: u8) {
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            Some(priority),
        )
        .unwrap();
}

/// One object of each commandable value type, with a Present_Value it takes.
fn every_value_type() -> Vec<(Box<dyn BACnetObject>, PropertyValue)> {
    let date = Date {
        year: 126,
        month: 10,
        day: 2,
        day_of_week: 5,
    };
    let time = Time {
        hour: 12,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let datetime = PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    vec![
        (
            Box::new(IntegerValueObject::new(1, "IV").unwrap()),
            PropertyValue::Signed(-7),
        ),
        (
            Box::new(PositiveIntegerValueObject::new(1, "PIV").unwrap()),
            PropertyValue::Unsigned(7),
        ),
        (
            Box::new(LargeAnalogValueObject::new(1, "LAV").unwrap()),
            PropertyValue::Double(7.5),
        ),
        (
            Box::new(CharacterStringValueObject::new(1, "CSV").unwrap()),
            PropertyValue::CharacterString("on".into()),
        ),
        (
            Box::new(OctetStringValueObject::new(1, "OSV").unwrap()),
            PropertyValue::OctetString(vec![1, 2]),
        ),
        (
            Box::new(BitStringValueObject::new(1, "BSV").unwrap()),
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0xA0],
            },
        ),
        (
            Box::new(DateValueObject::new(1, "DV").unwrap()),
            PropertyValue::Date(date),
        ),
        (
            Box::new(TimeValueObject::new(1, "TV").unwrap()),
            PropertyValue::Time(time),
        ),
        (
            Box::new(DateTimeValueObject::new(1, "DTV").unwrap()),
            datetime.clone(),
        ),
        (
            Box::new(DatePatternValueObject::new(1, "DPV").unwrap()),
            PropertyValue::Date(date),
        ),
        (
            Box::new(TimePatternValueObject::new(1, "TPV").unwrap()),
            PropertyValue::Time(time),
        ),
        (
            Box::new(DateTimePatternValueObject::new(1, "DTPV").unwrap()),
            datetime,
        ),
    ]
}

#[test]
fn every_value_type_serves_current_command_priority_from_its_priority_array() {
    let ccp = PropertyIdentifier::CURRENT_COMMAND_PRIORITY;
    for (mut object, value) in every_value_type() {
        let kind = object.object_identifier().object_type();
        // Listed as the commandable O2 row it is, scalar and read-only.
        assert!(object.property_list().contains(&ccp), "{kind:?}");
        assert!(!object.required_properties().contains(&ccp), "{kind:?}");
        assert!(!object.is_array_property(ccp), "{kind:?}");
        assert!(!object.is_writable_property(ccp), "{kind:?}");

        // Nothing commands a new object, so Relinquish_Default is in effect.
        assert_eq!(read(&*object, ccp), PropertyValue::Null, "{kind:?}");
        command(&mut *object, value.clone(), 9);
        assert_eq!(read(&*object, ccp), PropertyValue::Unsigned(9), "{kind:?}");
        command(&mut *object, value.clone(), 4);
        assert_eq!(read(&*object, ccp), PropertyValue::Unsigned(4), "{kind:?}");
        // A lower priority slot doesn't take over from the winning one.
        command(&mut *object, value, 16);
        assert_eq!(read(&*object, ccp), PropertyValue::Unsigned(4), "{kind:?}");
        command(&mut *object, PropertyValue::Null, 4);
        assert_eq!(read(&*object, ccp), PropertyValue::Unsigned(9), "{kind:?}");
        command(&mut *object, PropertyValue::Null, 9);
        command(&mut *object, PropertyValue::Null, 16);
        assert_eq!(read(&*object, ccp), PropertyValue::Null, "{kind:?}");

        assert_error(
            object.write_property(ccp, None, PropertyValue::Unsigned(1), None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

/// Run the COV_Increment checks shared by the three numeric value objects.
fn assert_cov_increment_row(object: &mut dyn BACnetObject, zero: PropertyValue) {
    let cov = PropertyIdentifier::COV_INCREMENT;
    let kind = object.object_identifier().object_type();
    assert!(object.property_list().contains(&cov), "{kind:?}");
    assert!(!object.required_properties().contains(&cov), "{kind:?}");
    assert!(object.is_writable_property(cov), "{kind:?}");
    assert!(!object.is_array_property(cov), "{kind:?}");
    // A new object notifies on any change.
    assert_eq!(read(object, cov), zero, "{kind:?}");
    assert_eq!(object.cov_increment(), Some(0.0), "{kind:?}");
}

#[test]
fn integer_value_types_serve_an_unsigned_cov_increment() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(IntegerValueObject::new(1, "IV").unwrap()),
        Box::new(PositiveIntegerValueObject::new(1, "PIV").unwrap()),
    ];
    let cov = PropertyIdentifier::COV_INCREMENT;
    for mut object in objects {
        let kind = object.object_identifier().object_type();
        assert_cov_increment_row(&mut *object, PropertyValue::Unsigned(0));
        for value in [5, 1 << 40] {
            object
                .write_property(cov, None, PropertyValue::Unsigned(value), None)
                .unwrap();
            assert_eq!(read(&*object, cov), PropertyValue::Unsigned(value));
            assert_eq!(object.cov_increment(), Some(value as f64), "{kind:?}");
        }
        // Only Unsigned is the table's datatype; a negative can't be one.
        for value in [
            PropertyValue::Signed(-5),
            PropertyValue::Signed(5),
            PropertyValue::Real(5.0),
            PropertyValue::Double(5.0),
            PropertyValue::Null,
        ] {
            assert_error(
                object.write_property(cov, None, value, None),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert_eq!(read(&*object, cov), PropertyValue::Unsigned(1 << 40));
    }

    let mut iv = IntegerValueObject::new(2, "IV-2").unwrap();
    iv.set_cov_increment(3).unwrap();
    assert_eq!(read(&iv, cov), PropertyValue::Unsigned(3));
    let mut piv = PositiveIntegerValueObject::new(2, "PIV-2").unwrap();
    piv.set_cov_increment(u64::MAX).unwrap();
    assert_eq!(read(&piv, cov), PropertyValue::Unsigned(u64::MAX));
}

#[test]
fn large_analog_value_serves_a_double_cov_increment_and_refuses_bad_ones() {
    let cov = PropertyIdentifier::COV_INCREMENT;
    let mut lav = LargeAnalogValueObject::new(1, "LAV").unwrap();
    assert_cov_increment_row(&mut lav, PropertyValue::Double(0.0));
    // A Double that a REAL can't hold exactly is kept as written.
    for value in [0.1, 2.5, 1e300] {
        lav.write_property(cov, None, PropertyValue::Double(value), None)
            .unwrap();
        assert_eq!(read(&lav, cov), PropertyValue::Double(value));
        assert_eq!(lav.cov_increment(), Some(value));
    }
    lav.write_property(cov, None, PropertyValue::Double(0.1), None)
        .unwrap();
    // A negative or non-finite minimum change is out of range.
    for value in [-0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_error(
            lav.write_property(cov, None, PropertyValue::Double(value), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_error(lav.set_cov_increment(value), ErrorCode::VALUE_OUT_OF_RANGE);
    }
    for value in [
        PropertyValue::Real(0.5),
        PropertyValue::Unsigned(1),
        PropertyValue::Null,
    ] {
        assert_error(
            lav.write_property(cov, None, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(read(&lav, cov), PropertyValue::Double(0.1));
    lav.set_cov_increment(0.75).unwrap();
    assert_eq!(read(&lav, cov), PropertyValue::Double(0.75));
}

#[test]
fn non_numeric_value_types_have_no_cov_increment() {
    for (object, _) in every_value_type().into_iter().skip(3) {
        let kind = object.object_identifier().object_type();
        assert!(
            !object
                .property_list()
                .contains(&PropertyIdentifier::COV_INCREMENT),
            "{kind:?}"
        );
        assert_error(
            object.read_property(PropertyIdentifier::COV_INCREMENT, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_eq!(object.cov_increment(), None, "{kind:?}");
    }
}
