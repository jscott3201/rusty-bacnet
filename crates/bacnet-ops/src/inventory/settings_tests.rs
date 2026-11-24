//! A record's settings judged against the object's type (#2765): a value
//! of another datatype is a problem and stays out of the record, from rows
//! and from a device by ReadPropertyMultiple and by ReadProperty; a value
//! a point list builder would refuse is kept and is a problem; a State_Text
//! entry that isn't a string is a problem.

use bacnet_objects::access_control::AccessDoorObject;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::multistate::{MultiStateOutputObject, MultiStateValueObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, PropertyValue as V, Time};

use super::record::record;
use super::test_lab::{
    Answering, Faults, LAB, lab_device, oid, refusing_rpm, row, serve, stop, text,
};
use super::*;
use crate::device::DeviceAddress;
use crate::format::RawRow;

/// The settings an inventory reads.
const SETTINGS: [P; 5] = [
    P::COV_INCREMENT,
    P::MIN_PRES_VALUE,
    P::MAX_PRES_VALUE,
    P::POLARITY,
    P::RELINQUISH_DEFAULT,
];

fn of(object_type: ObjectType, rows: Vec<RawRow>) -> PointRecord {
    let at = DeviceAddress::local(&[2]).unwrap();
    record(&at, 7, oid(object_type, 1), rows)
}

fn settings() -> InventoryOptions {
    InventoryOptions::new().with_settings(true)
}

/// The settings the inventory reads of an object of `object_type`.
fn asked(object_type: ObjectType) -> Vec<P> {
    properties(object_type, &settings())
        .into_iter()
        .filter(|property| SETTINGS.contains(property))
        .collect()
}

/// The datatype `property` has on an object of `object_type`, as a problem
/// names it, and a value of it; from the Clause 12 tables of each type
/// (12.2 to 12.4, 12.6 to 12.8, 12.19, 12.20, 12.26, 12.37 to 12.48,
/// 12.54 and 12.55), written out here apart from the code's table.
fn datatype(object_type: ObjectType, property: P) -> Option<(&'static str, V)> {
    type O = ObjectType;
    let real = || ("a Real", V::Real(1.5));
    let enumerated = || ("an Enumerated", V::Enumerated(1));
    let unsigned = || ("an Unsigned", V::Unsigned(2));
    let signed = || ("a Signed", V::Signed(-3));
    let double = || ("a Double", V::Double(2.5));
    let date = Date {
        year: 126,
        month: 11,
        day: 24,
        day_of_week: 2,
    };
    let time = Time {
        hour: 7,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    Some(match (object_type, property) {
        (O::ANALOG_INPUT | O::ANALOG_OUTPUT | O::ANALOG_VALUE, _) => real(),
        (O::LIGHTING_OUTPUT, P::RELINQUISH_DEFAULT) => real(),
        (O::BINARY_INPUT | O::BINARY_OUTPUT | O::BINARY_VALUE, _) => enumerated(),
        (O::ACCESS_DOOR | O::BINARY_LIGHTING_OUTPUT, P::RELINQUISH_DEFAULT) => enumerated(),
        (O::MULTI_STATE_OUTPUT | O::MULTI_STATE_VALUE | O::POSITIVE_INTEGER_VALUE, _) => unsigned(),
        (O::INTEGER_VALUE, P::COV_INCREMENT) => unsigned(),
        (O::INTEGER_VALUE, _) => signed(),
        (O::LARGE_ANALOG_VALUE, _) => double(),
        (O::CHARACTERSTRING_VALUE, _) => ("a character string", text("on")),
        (O::BITSTRING_VALUE, _) => (
            "a bit string",
            V::BitString {
                unused_bits: 4,
                data: vec![0xA0],
            },
        ),
        (O::OCTETSTRING_VALUE, _) => ("an octet string", V::OctetString(vec![1, 2])),
        (O::DATE_VALUE | O::DATEPATTERN_VALUE, _) => ("a Date", V::Date(date)),
        (O::TIME_VALUE | O::TIMEPATTERN_VALUE, _) => ("a Time", V::Time(time)),
        (O::DATETIME_VALUE | O::DATETIMEPATTERN_VALUE, _) => (
            "a date and time",
            V::List(vec![V::Date(date), V::Time(time)]),
        ),
        _ => return None,
    })
}

/// A value of another datatype than `name` names: a character string, or
/// a Real in place of one.
fn other_than(name: &str) -> V {
    if name == "a character string" {
        V::Real(1.0)
    } else {
        text("wrong")
    }
}

/// A value of the datatype nearest the one `name` names, which a device
/// might give in its place: a Double for a Real, a Signed for an
/// Unsigned, a Time for a Date, a Date alone for a date and time.
fn near(name: &str) -> V {
    let date = V::Date(Date {
        year: 126,
        month: 1,
        day: 1,
        day_of_week: 4,
    });
    let time = V::Time(Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    });
    match name {
        "a Real" => V::Double(1.5),
        "a Double" => V::Real(2.5),
        "an Unsigned" | "an Enumerated" => V::Signed(2),
        "a Signed" => V::Unsigned(3),
        "a character string" | "a bit string" => V::OctetString(vec![0xA0]),
        "an octet string" => text("0a"),
        "a Date" => time,
        "a Time" | "a date and time" => date,
        other => panic!("no near value for {other}"),
    }
}

/// `record`'s field for the setting `property`.
fn field(record: &PointRecord, property: P) -> &Option<Value> {
    match property {
        P::COV_INCREMENT => &record.cov_increment,
        P::MIN_PRES_VALUE => &record.min_present_value,
        P::MAX_PRES_VALUE => &record.max_present_value,
        P::POLARITY => &record.polarity,
        P::RELINQUISH_DEFAULT => &record.relinquish_default,
        other => panic!("{other} is no setting"),
    }
}

fn said(record: &PointRecord, property: P) -> Option<String> {
    record.problem(property).map(ToString::to_string)
}

/// Every setting the plan reads, of every type the standard defines up to
/// Color Temperature: one of its datatype is kept with no problem, one of
/// another (a value of an unrelated type or of the nearest one, two
/// values) is a problem naming the datatype and what came back, and
/// octets that don't decode are a problem saying so; the field stays
/// empty. A type this release doesn't know reads Relinquish_Default and
/// takes any value there.
#[test]
fn a_setting_of_another_datatype_is_a_problem() {
    let mut judged = 0;
    for raw in 0..=ObjectType::COLOR_TEMPERATURE.to_raw() {
        let object_type = ObjectType::from_raw(raw);
        for property in asked(object_type) {
            let Some((name, good)) = datatype(object_type, property) else {
                panic!("{object_type} reads {property}, which the test doesn't type");
            };
            judged += 1;
            let read = of(object_type, vec![row(property, good.clone())]);
            assert!(
                read.problems.is_empty(),
                "{object_type} {property}: {read:?}"
            );
            assert!(field(&read, property).is_some(), "{object_type} {property}");
            let once = row(property, good.clone()).1.unwrap();
            let twice = (property, Ok([once.clone(), once].concat()));
            let broken = (property, Ok(vec![0x44, 0x3F]));
            let other = row(property, other_than(name));
            let nearest = row(property, near(name));
            let mistyped = format!("not {name} (got ");
            let several = format!("not {name} (got several values, ");
            let cases = [
                (other, mistyped.as_str()),
                (nearest, &mistyped),
                (twice, &several),
                (broken, "can't be decoded"),
            ];
            for (wrong, expected) in cases {
                let read = of(object_type, vec![wrong]);
                let said = said(&read, property).unwrap_or_default();
                assert!(
                    said.starts_with(expected),
                    "{object_type} {property}: {said}"
                );
                assert!(field(&read, property).is_none(), "{object_type} {property}");
            }
        }
    }
    // Twenty-one types read a Relinquish_Default, six a COV_Increment and
    // a range, two a Polarity.
    assert_eq!(judged, 21 + 6 * 3 + 2);

    let vendor = ObjectType::from_raw(700);
    assert_eq!(asked(vendor), [P::RELINQUISH_DEFAULT]);
    let read = of(vendor, vec![row(P::RELINQUISH_DEFAULT, text("any"))]);
    assert!(read.problems.is_empty() && read.relinquish_default.is_some());
}

/// A problem says what came back: its datatype and its text, device text
/// cleaned as a name is; NULL as NULL. A character string in a character
/// set this stack doesn't convert can't be decoded, which isn't a wrong
/// datatype: the problem names the set.
#[test]
fn a_problem_says_what_came_back() {
    let cases: Vec<(ObjectType, RawRow, &str)> = vec![
        (
            ObjectType::ANALOG_VALUE,
            row(P::RELINQUISH_DEFAULT, V::Unsigned(5)),
            "not a Real (got an Unsigned, 5)",
        ),
        (
            ObjectType::ANALOG_OUTPUT,
            row(P::RELINQUISH_DEFAULT, text("auto\u{7}")),
            "not a Real (got a character string, \"auto \")",
        ),
        (
            ObjectType::BINARY_OUTPUT,
            row(P::RELINQUISH_DEFAULT, V::Null),
            "not an Enumerated (got NULL)",
        ),
        (
            ObjectType::INTEGER_VALUE,
            row(P::COV_INCREMENT, V::Real(0.5)),
            "not an Unsigned (got a Real, 0.5)",
        ),
        (
            ObjectType::ANALOG_INPUT,
            (P::COV_INCREMENT, Ok(Vec::new())),
            "not a Real (got no value)",
        ),
        (
            ObjectType::CHARACTERSTRING_VALUE,
            (P::RELINQUISH_DEFAULT, Ok(vec![0x73, 0x02, 0x30, 0x30])),
            "can't be decoded: a character string in JIS X 0208",
        ),
        (
            ObjectType::CHARACTERSTRING_VALUE,
            (P::RELINQUISH_DEFAULT, Ok(vec![0x73, 0x01, 0x00, 0x41])),
            "can't be decoded: a character string in IBM/Microsoft DBCS",
        ),
        (
            ObjectType::ANALOG_VALUE,
            (
                P::RELINQUISH_DEFAULT,
                Ok(vec![0x75, 0x05, 0x03, 0, 0, 0, 0x41]),
            ),
            "can't be decoded: a character string in UCS-4",
        ),
        (
            ObjectType::ANALOG_VALUE,
            (P::RELINQUISH_DEFAULT, Ok(vec![0x44, 0x3F])),
            "can't be decoded",
        ),
    ];
    for (object_type, wrong, expected) in cases {
        let property = wrong.0;
        let read = of(object_type, vec![wrong]);
        assert_eq!(said(&read, property).as_deref(), Some(expected), "{read:?}");
        assert!(field(&read, property).is_none());
    }
}

/// The ops crate keeps the builder's largest state count as its own
/// constant; it must be the builder's.
#[test]
fn the_state_cap_is_the_builders() {
    assert_eq!(
        super::settings::MAX_STATES,
        bacnet_objects::multistate::MAX_NUMBER_OF_STATES
    );
}

/// A value a point list builder would refuse is kept as the device gave
/// it, with a problem that says why: an empty name, Units past 65535, a
/// state count out of range or State_Text that doesn't match it, an analog
/// Real that isn't finite, a negative COV_Increment, Min_Pres_Value above
/// Max_Pres_Value, a binary state or Polarity past 1, a multi-state
/// relinquish default that names no state. A relinquish default counts
/// only where the point holds one: on an output, or a value with a
/// Priority_Array. The types a point list doesn't hold aren't judged so.
#[test]
fn a_value_a_point_list_would_refuse_is_kept_and_named() {
    let real = |property, value| row(property, V::Real(value));
    let commanded = || row(P::PRIORITY_ARRAY, V::Null);
    let three = || row(P::NUMBER_OF_STATES, V::Unsigned(3));
    let cases: Vec<(ObjectType, Vec<RawRow>, P, &str)> = vec![
        (
            ObjectType::ANALOG_INPUT,
            vec![row(P::OBJECT_NAME, text(""))],
            P::OBJECT_NAME,
            "empty",
        ),
        (
            ObjectType::INTEGER_VALUE,
            vec![row(P::UNITS, V::Enumerated(65_536))],
            P::UNITS,
            "65536 is above 65535",
        ),
        (
            ObjectType::MULTI_STATE_INPUT,
            vec![row(P::NUMBER_OF_STATES, V::Unsigned(0))],
            P::NUMBER_OF_STATES,
            "outside 1 to 1024",
        ),
        (
            ObjectType::MULTI_STATE_OUTPUT,
            vec![row(P::NUMBER_OF_STATES, V::Unsigned(1025))],
            P::NUMBER_OF_STATES,
            "outside 1 to 1024",
        ),
        (
            ObjectType::MULTI_STATE_VALUE,
            vec![
                three(),
                row(P::STATE_TEXT, V::List(vec![text("Off"), text("On")])),
            ],
            P::STATE_TEXT,
            "2 labels for 3 states",
        ),
        (
            ObjectType::ANALOG_OUTPUT,
            vec![real(P::RELINQUISH_DEFAULT, f32::NAN)],
            P::RELINQUISH_DEFAULT,
            "not a finite number",
        ),
        (
            ObjectType::ANALOG_INPUT,
            vec![real(P::COV_INCREMENT, f32::INFINITY)],
            P::COV_INCREMENT,
            "not a finite number",
        ),
        (
            ObjectType::ANALOG_VALUE,
            vec![real(P::COV_INCREMENT, -0.5)],
            P::COV_INCREMENT,
            "negative",
        ),
        (
            ObjectType::ANALOG_INPUT,
            vec![real(P::MIN_PRES_VALUE, f32::NEG_INFINITY)],
            P::MIN_PRES_VALUE,
            "not a finite number",
        ),
        (
            ObjectType::ANALOG_OUTPUT,
            vec![real(P::MAX_PRES_VALUE, f32::NAN)],
            P::MAX_PRES_VALUE,
            "not a finite number",
        ),
        (
            ObjectType::ANALOG_VALUE,
            vec![real(P::MIN_PRES_VALUE, 10.0), real(P::MAX_PRES_VALUE, 5.0)],
            P::MIN_PRES_VALUE,
            "above max-pres-value",
        ),
        (
            ObjectType::INTEGER_VALUE,
            vec![
                row(P::MIN_PRES_VALUE, V::Signed(10)),
                row(P::MAX_PRES_VALUE, V::Signed(-10)),
            ],
            P::MIN_PRES_VALUE,
            "above max-pres-value",
        ),
        (
            ObjectType::BINARY_OUTPUT,
            vec![row(P::RELINQUISH_DEFAULT, V::Enumerated(2))],
            P::RELINQUISH_DEFAULT,
            "neither inactive nor active",
        ),
        (
            ObjectType::BINARY_INPUT,
            vec![row(P::POLARITY, V::Enumerated(7))],
            P::POLARITY,
            "neither normal nor reverse",
        ),
        (
            ObjectType::MULTI_STATE_OUTPUT,
            vec![
                row(P::NUMBER_OF_STATES, V::Unsigned(3)),
                row(P::RELINQUISH_DEFAULT, V::Unsigned(0)),
            ],
            P::RELINQUISH_DEFAULT,
            "not one of the states 1 to 3",
        ),
        (
            ObjectType::MULTI_STATE_VALUE,
            vec![
                commanded(),
                row(P::RELINQUISH_DEFAULT, V::Unsigned(4)),
                three(),
            ],
            P::RELINQUISH_DEFAULT,
            "not one of the states 1 to 3",
        ),
        (
            ObjectType::MULTI_STATE_VALUE,
            vec![commanded(), row(P::RELINQUISH_DEFAULT, V::Unsigned(0))],
            P::RELINQUISH_DEFAULT,
            "not a state; states start at 1",
        ),
        (
            ObjectType::BINARY_VALUE,
            vec![commanded(), row(P::RELINQUISH_DEFAULT, V::Enumerated(5))],
            P::RELINQUISH_DEFAULT,
            "neither inactive nor active",
        ),
        (
            ObjectType::ANALOG_VALUE,
            vec![commanded(), real(P::RELINQUISH_DEFAULT, f32::INFINITY)],
            P::RELINQUISH_DEFAULT,
            "not a finite number",
        ),
    ];
    for (object_type, rows, property, why) in cases {
        let read = of(object_type, rows);
        assert_eq!(read.problems.len(), 1, "{object_type}: {read:?}");
        assert_eq!(said(&read, property).as_deref(), Some(why), "{object_type}");
        let kept = match property {
            P::OBJECT_NAME => read.name.is_some(),
            P::UNITS => read.units.is_some(),
            P::NUMBER_OF_STATES => read.number_of_states.is_some(),
            P::STATE_TEXT => !read.state_texts.states.is_empty(),
            setting => field(&read, setting).is_some(),
        };
        assert!(kept, "kept: {read:?}");
    }

    // Within range, nothing.
    let wild = Date {
        year: 0xFF,
        month: 0xFF,
        day: 0xFF,
        day_of_week: 0xFF,
    };
    let any_time = Time {
        hour: 0xFF,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    let fine: Vec<(ObjectType, Vec<RawRow>)> = vec![
        (
            ObjectType::ANALOG_VALUE,
            vec![
                commanded(),
                real(P::COV_INCREMENT, 0.0),
                real(P::MIN_PRES_VALUE, 5.0),
                real(P::MAX_PRES_VALUE, 5.0),
                real(P::RELINQUISH_DEFAULT, -40.0),
                row(P::UNITS, V::Enumerated(65_535)),
            ],
        ),
        (
            ObjectType::BINARY_VALUE,
            vec![commanded(), row(P::RELINQUISH_DEFAULT, V::Enumerated(1))],
        ),
        // A value without a Priority_Array: the export leaves its
        // relinquish default out, so the builder never sees it.
        (
            ObjectType::ANALOG_VALUE,
            vec![real(P::RELINQUISH_DEFAULT, f32::NAN)],
        ),
        (
            ObjectType::BINARY_VALUE,
            vec![row(P::RELINQUISH_DEFAULT, V::Enumerated(2))],
        ),
        (
            ObjectType::MULTI_STATE_VALUE,
            vec![three(), row(P::RELINQUISH_DEFAULT, V::Unsigned(4))],
        ),
        (
            ObjectType::MULTI_STATE_INPUT,
            vec![
                three(),
                row(
                    P::STATE_TEXT,
                    V::List(vec![text("a"), text("b"), text("c")]),
                ),
            ],
        ),
        (
            ObjectType::MULTI_STATE_INPUT,
            vec![row(P::NUMBER_OF_STATES, V::Unsigned(1024))],
        ),
        // Their enumerations run past 1.
        (
            ObjectType::ACCESS_DOOR,
            vec![row(P::RELINQUISH_DEFAULT, V::Enumerated(3))],
        ),
        (
            ObjectType::BINARY_LIGHTING_OUTPUT,
            vec![row(P::RELINQUISH_DEFAULT, V::Enumerated(4))],
        ),
        // Patterns with every field unspecified.
        (
            ObjectType::DATEPATTERN_VALUE,
            vec![row(P::RELINQUISH_DEFAULT, V::Date(wild))],
        ),
        (
            ObjectType::DATETIMEPATTERN_VALUE,
            vec![row(
                P::RELINQUISH_DEFAULT,
                V::List(vec![V::Date(wild), V::Time(any_time)]),
            )],
        ),
        (ObjectType::TREND_LOG, vec![row(P::OBJECT_NAME, text(""))]),
        (
            ObjectType::BINARY_OUTPUT,
            vec![row(P::POLARITY, V::Enumerated(1))],
        ),
        (
            ObjectType::MULTI_STATE_OUTPUT,
            vec![
                row(P::NUMBER_OF_STATES, V::Unsigned(3)),
                row(P::RELINQUISH_DEFAULT, V::Unsigned(3)),
            ],
        ),
        // With no Number_Of_States read, only 0 is surely no state.
        (
            ObjectType::MULTI_STATE_OUTPUT,
            vec![row(P::RELINQUISH_DEFAULT, V::Unsigned(9))],
        ),
        // A point list holds none of these types.
        (
            ObjectType::LARGE_ANALOG_VALUE,
            vec![
                row(P::COV_INCREMENT, V::Double(f64::NAN)),
                row(P::MIN_PRES_VALUE, V::Double(10.0)),
                row(P::MAX_PRES_VALUE, V::Double(5.0)),
            ],
        ),
        (
            ObjectType::LIGHTING_OUTPUT,
            vec![real(P::RELINQUISH_DEFAULT, f32::NAN)],
        ),
        (
            ObjectType::POSITIVE_INTEGER_VALUE,
            vec![
                row(P::MIN_PRES_VALUE, V::Unsigned(10)),
                row(P::MAX_PRES_VALUE, V::Unsigned(5)),
            ],
        ),
    ];
    for (object_type, rows) in fine {
        let read = of(object_type, rows);
        assert!(read.problems.is_empty(), "{object_type}: {read:?}");
    }
}

/// The problems stay in the order asked when a range check, made once
/// every row is read, joins those of the rows.
#[test]
fn problems_keep_the_order_asked() {
    let rows = vec![
        row(P::NUMBER_OF_STATES, V::Unsigned(2)),
        row(P::STATE_TEXT, V::List(vec![text("Off"), V::Unsigned(1)])),
        row(P::RELINQUISH_DEFAULT, V::Unsigned(5)),
        row(P::PRESENT_VALUE, V::Unsigned(1)),
        (P::STATUS_FLAGS, Err(PropertyError::NoValue)),
    ];
    let read = of(ObjectType::MULTI_STATE_OUTPUT, rows);
    let order: Vec<P> = read
        .problems
        .iter()
        .map(|problem| problem.property)
        .collect();
    assert_eq!(
        order,
        [P::STATE_TEXT, P::RELINQUISH_DEFAULT, P::STATUS_FLAGS]
    );
}

/// A State_Text entry that isn't a character string is a problem naming
/// its state and what came back; the record holds it as an empty text, so
/// the states after it keep their numbers. One that can't be decoded is a
/// problem naming the character set, and leaves no state texts.
#[test]
fn a_state_text_entry_that_isnt_a_string_is_a_problem() {
    let entries = V::List(vec![text("Off"), V::Unsigned(5), text("High")]);
    let read = of(
        ObjectType::MULTI_STATE_VALUE,
        vec![row(P::STATE_TEXT, entries)],
    );
    assert_eq!(read.state_texts.states, ["Off", "", "High"]);
    assert_eq!(
        said(&read, P::STATE_TEXT).as_deref(),
        Some("state 2 is not a character string (got an Unsigned, 5)")
    );
    let mut octets = row(P::STATE_TEXT, text("Off")).1.unwrap();
    octets.extend([0x73, 0x02, 0x30, 0x30]);
    let read = of(
        ObjectType::MULTI_STATE_INPUT,
        vec![(P::STATE_TEXT, Ok(octets))],
    );
    assert!(read.state_texts.states.is_empty(), "{read:?}");
    assert_eq!(
        said(&read, P::STATE_TEXT).as_deref(),
        Some("state 2 can't be decoded: a character string in JIS X 0208")
    );
    let broken = (P::STATE_TEXT, Ok(vec![0x75, 0x05, 0x00]));
    let read = of(ObjectType::MULTI_STATE_INPUT, vec![broken]);
    assert_eq!(
        said(&read, P::STATE_TEXT).as_deref(),
        Some("state 1 can't be decoded: a character string in UTF-8")
    );
    let texts = V::List(vec![text("Off"), text("")]);
    let read = of(
        ObjectType::MULTI_STATE_INPUT,
        vec![row(P::STATE_TEXT, texts)],
    );
    assert!(read.problems.is_empty(), "{read:?}");
}

/// One object of each of the 23 types whose settings the plan reads: the
/// analog and binary inputs, outputs and values, the multi-state outputs
/// and values, Access Door, both lighting outputs and the twelve value
/// types from CharacterString Value to Date Pattern Value.
fn objects() -> Vec<Box<dyn BACnetObject>> {
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    let mut ai = AnalogInputObject::new(1, "ai", celsius).unwrap();
    ai.set_min_pres_value(-40.0);
    ai.set_max_pres_value(120.0);
    let mut iv = IntegerValueObject::new(1, "iv").unwrap();
    iv.set_min_pres_value(-5);
    iv.set_max_pres_value(5);
    vec![
        Box::new(ai),
        Box::new(AnalogOutputObject::new(1, "ao", celsius).unwrap()),
        Box::new(AnalogValueObject::new(1, "av", celsius).unwrap()),
        Box::new(BinaryInputObject::new(1, "bi").unwrap()),
        Box::new(BinaryOutputObject::new(1, "bo").unwrap()),
        Box::new(BinaryValueObject::new(1, "bv").unwrap()),
        Box::new(MultiStateOutputObject::new(1, "mso", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "msv", 3).unwrap()),
        Box::new(iv),
        Box::new(PositiveIntegerValueObject::new(1, "piv").unwrap()),
        Box::new(LargeAnalogValueObject::new(1, "lav").unwrap()),
        Box::new(AccessDoorObject::new(1, "door").unwrap()),
        Box::new(LightingOutputObject::new(1, "light").unwrap()),
        Box::new(BinaryLightingOutputObject::new(1, "switch").unwrap()),
        Box::new(CharacterStringValueObject::new(1, "csv").unwrap()),
        Box::new(BitStringValueObject::new(1, "bsv").unwrap()),
        Box::new(OctetStringValueObject::new(1, "osv").unwrap()),
        Box::new(DateValueObject::new(1, "date").unwrap()),
        Box::new(DatePatternValueObject::new(1, "date pattern").unwrap()),
        Box::new(TimeValueObject::new(1, "time").unwrap()),
        Box::new(TimePatternValueObject::new(1, "time pattern").unwrap()),
        Box::new(DateTimeValueObject::new(1, "datetime").unwrap()),
        Box::new(DateTimePatternValueObject::new(1, "datetime pattern").unwrap()),
    ]
}

/// A device whose objects give every setting the plan reads of them as a
/// value of another datatype, by ReadPropertyMultiple and by ReadProperty:
/// each record names each setting's datatype in a problem, holds none of
/// them, and has no other problem. The same objects giving their own
/// values read with no problem, every Relinquish_Default held.
#[tokio::test(start_paused = true)]
async fn a_device_giving_mistyped_settings_by_rpm_and_by_read_property() {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let mut covered = 0;
    for faults in [Faults::default(), refusing_rpm()] {
        let database = build_database_with_extra(&identity, objects()).unwrap();
        let (client, server) = serve(faults.clone(), identity.clone(), database).await;
        let inventory = lab_device(&client)
            .await
            .inventory(&settings())
            .await
            .unwrap();
        stop(client, server).await;
        assert_eq!(inventory.points.len(), 24, "{inventory:?}");
        for record in &inventory.points {
            assert!(record.problems.is_empty(), "{record:?}");
            let reads_default = asked(record.object.object_type()).contains(&P::RELINQUISH_DEFAULT);
            assert_eq!(
                record.relinquish_default.is_some(),
                reads_default,
                "{record:?}"
            );
        }

        let wrapped = objects()
            .into_iter()
            .map(|object| {
                let object_type = object.object_identifier().object_type();
                let answers = asked(object_type)
                    .into_iter()
                    .map(|property| {
                        let (name, _) = datatype(object_type, property).unwrap();
                        (property, Ok(other_than(name)))
                    })
                    .collect();
                Answering::wrap(object, answers)
            })
            .collect();
        let database = build_database_with_extra(&identity, wrapped).unwrap();
        let (client, server) = serve(faults, identity.clone(), database).await;
        let inventory = lab_device(&client)
            .await
            .inventory(&settings())
            .await
            .unwrap();
        stop(client, server).await;
        assert!(inventory.stopped.is_none(), "{inventory:?}");
        for record in &inventory.points {
            let object_type = record.object.object_type();
            let problems: Vec<P> = record
                .problems
                .iter()
                .map(|problem| problem.property)
                .collect();
            assert_eq!(problems, asked(object_type), "{record:?}");
            for property in asked(object_type) {
                let (name, _) = datatype(object_type, property).unwrap();
                let said = said(record, property).unwrap_or_default();
                assert!(said.starts_with(&format!("not {name} (got ")), "{said}");
                assert!(field(record, property).is_none(), "{record:?}");
                covered += 1;
            }
            assert!(record.name.is_some(), "{record:?}");
        }
    }
    assert_eq!(covered, 2 * (21 + 6 * 3 + 2));
}

/// A setting a device refuses with an error other than an unknown property
/// is still a problem with the device's error, as before.
#[test]
fn a_refused_setting_is_still_the_devices_error() {
    let denied = RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::READ_ACCESS_DENIED,
    };
    let rows = vec![(P::COV_INCREMENT, Err(PropertyError::Remote(denied)))];
    let read = of(ObjectType::ANALOG_INPUT, rows);
    assert_eq!(
        said(&read, P::COV_INCREMENT).as_deref(),
        Some("PROPERTY:READ_ACCESS_DENIED")
    );
}
