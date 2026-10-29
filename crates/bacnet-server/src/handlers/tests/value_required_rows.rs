//! Rows #1111 added to the commandable value types, over the wire:
//! Current_Command_Priority on all 12 (footnote 2 of Tables 12-44 to 12-55)
//! and COV_Increment on Integer, Positive Integer and Large Analog Value
//! (footnote 3 of Tables 12-50, 12-51 and 12-46). Both are optional rows, so
//! RPM ALL and OPTIONAL carry them and REQUIRED doesn't.

use super::lighting_required_rows::{assert_refused, db_with, read_wire, write_wire};
use super::property_metadata::assert_rpm_selector_bytes;
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};
use bacnet_types::primitives::{Date, Time};

/// One object of each commandable value type, with a Present_Value it takes.
/// The three numeric types come first.
fn every_value_type() -> Vec<(Box<dyn BACnetObject>, PropertyValue)> {
    let date = Date {
        year: 126,
        month: 10,
        day: 2,
        day_of_week: 5,
    };
    let time = Time {
        hour: 6,
        minute: 15,
        second: 0,
        hundredths: 0,
    };
    let datetime = PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    vec![
        (
            Box::new(IntegerValueObject::new(3, "IV-3").unwrap()),
            PropertyValue::Signed(-40),
        ),
        (
            Box::new(PositiveIntegerValueObject::new(3, "PIV-3").unwrap()),
            PropertyValue::Unsigned(40),
        ),
        (
            Box::new(LargeAnalogValueObject::new(3, "LAV-3").unwrap()),
            PropertyValue::Double(40.25),
        ),
        (
            Box::new(CharacterStringValueObject::new(3, "CSV-3").unwrap()),
            PropertyValue::CharacterString("occupied".into()),
        ),
        (
            Box::new(OctetStringValueObject::new(3, "OSV-3").unwrap()),
            PropertyValue::OctetString(vec![0xBE, 0xEF]),
        ),
        (
            Box::new(BitStringValueObject::new(3, "BSV-3").unwrap()),
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0x40],
            },
        ),
        (
            Box::new(DateValueObject::new(3, "DV-3").unwrap()),
            PropertyValue::Date(date),
        ),
        (
            Box::new(TimeValueObject::new(3, "TV-3").unwrap()),
            PropertyValue::Time(time),
        ),
        (
            Box::new(DateTimeValueObject::new(3, "DTV-3").unwrap()),
            datetime.clone(),
        ),
        (
            Box::new(DatePatternValueObject::new(3, "DPV-3").unwrap()),
            PropertyValue::Date(date),
        ),
        (
            Box::new(TimePatternValueObject::new(3, "TPV-3").unwrap()),
            PropertyValue::Time(time),
        ),
        (
            Box::new(DateTimePatternValueObject::new(3, "DTPV-3").unwrap()),
            datetime,
        ),
    ]
}

#[test]
fn rpm_non_numeric_value_selectors_carry_current_command_priority() {
    // The numeric three also serve Units and COV_Increment; their exact
    // selectors are pinned in property_metadata/value_units.rs.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
    ];
    let optional = [
        P::DESCRIPTION,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
    ];
    for (object, value) in every_value_type().into_iter().skip(3) {
        let (mut db, oid) = db_with(object);
        // Once with Relinquish_Default in effect, once commanded.
        for commanded in [false, true] {
            if commanded {
                write_wire(&mut db, oid, P::PRESENT_VALUE, value.clone(), Some(12)).unwrap();
            }
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

#[test]
fn current_command_priority_reads_over_the_wire_for_every_value_type() {
    let ccp = P::CURRENT_COMMAND_PRIORITY;
    for (object, value) in every_value_type() {
        let (mut db, oid) = db_with(object);
        // Null (0x00) while Relinquish_Default is in effect.
        assert_eq!(read_wire(&db, oid, ccp), [0x00], "{oid:?}");
        write_wire(&mut db, oid, P::PRESENT_VALUE, value.clone(), Some(11)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 11], "{oid:?}");
        write_wire(&mut db, oid, P::PRESENT_VALUE, value, Some(2)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 2], "{oid:?}");
        assert_refused(
            write_wire(&mut db, oid, ccp, PropertyValue::Unsigned(5), None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        write_wire(&mut db, oid, P::PRESENT_VALUE, PropertyValue::Null, Some(2)).unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x21, 11], "{oid:?}");
        write_wire(
            &mut db,
            oid,
            P::PRESENT_VALUE,
            PropertyValue::Null,
            Some(11),
        )
        .unwrap();
        assert_eq!(read_wire(&db, oid, ccp), [0x00], "{oid:?}");
    }
}

#[test]
fn integer_value_cov_increment_is_a_writable_unsigned_over_the_wire() {
    let cov = P::COV_INCREMENT;
    for (object, _) in every_value_type().into_iter().take(2) {
        let (mut db, oid) = db_with(object);
        assert_eq!(read_wire(&db, oid, cov), [0x21, 0x00], "{oid:?}");
        write_wire(&mut db, oid, cov, PropertyValue::Unsigned(300), None).unwrap();
        // 300 is 0x012C, a two-octet Unsigned.
        assert_eq!(read_wire(&db, oid, cov), [0x22, 0x01, 0x2C], "{oid:?}");
        for wrong in [
            PropertyValue::Signed(-1),
            PropertyValue::Real(1.0),
            PropertyValue::Double(1.0),
        ] {
            assert_refused(
                write_wire(&mut db, oid, cov, wrong, None),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert_eq!(read_wire(&db, oid, cov), [0x22, 0x01, 0x2C], "{oid:?}");
    }
}

#[test]
fn large_analog_value_cov_increment_is_a_writable_double_over_the_wire() {
    let cov = P::COV_INCREMENT;
    let (mut db, oid) = db_with(Box::new(LargeAnalogValueObject::new(3, "LAV-3").unwrap()));
    // Application tag 5 with an 8-octet length: Double 0.0.
    assert_eq!(
        read_wire(&db, oid, cov),
        [0x55, 0x08, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    write_wire(&mut db, oid, cov, PropertyValue::Double(0.5), None).unwrap();
    let half = [0x55, 0x08, 0x3F, 0xE0, 0, 0, 0, 0, 0, 0];
    assert_eq!(read_wire(&db, oid, cov), half);
    for outside in [-0.5, f64::NAN, f64::INFINITY] {
        assert_refused(
            write_wire(&mut db, oid, cov, PropertyValue::Double(outside), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for wrong in [PropertyValue::Real(0.5), PropertyValue::Unsigned(1)] {
        assert_refused(
            write_wire(&mut db, oid, cov, wrong, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(read_wire(&db, oid, cov), half);
}

#[test]
fn pics_value_types_list_current_command_priority_and_cov_increment() {
    use crate::pics::{PicsConfig, generate_pics};
    use crate::server::ServerConfig;

    for (index, (object, _)) in every_value_type().into_iter().enumerate() {
        let kind = object.object_identifier().object_type();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let support = &pics.supported_object_types[0];
        assert_eq!(support.object_type, kind);
        let row = |property: P| {
            support
                .supported_properties
                .iter()
                .find(|row| row.property_id == property)
        };
        let ccp = row(P::CURRENT_COMMAND_PRIORITY)
            .unwrap_or_else(|| panic!("{kind:?} lists Current_Command_Priority"));
        assert!(ccp.access.readable && ccp.access.optional, "{kind:?}");
        assert!(!ccp.access.writable, "{kind:?}");
        match row(P::COV_INCREMENT) {
            Some(cov) => {
                assert!(index < 3, "{kind:?} lists COV_Increment");
                assert!(cov.access.readable && cov.access.optional, "{kind:?}");
                assert!(cov.access.writable, "{kind:?}");
            }
            None => assert!(index >= 3, "{kind:?} lacks COV_Increment"),
        }
    }
}
