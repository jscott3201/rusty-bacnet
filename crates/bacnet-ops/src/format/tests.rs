//! The formatter on encoded values: floats, enumerations and bits, dates,
//! binary and multi-state states, and each constructed element it renders.

use bacnet_encoding::constructed::{
    encode_daily_schedule, encode_date_range, encode_object_property_reference, encode_value_source,
};
use bacnet_encoding::primitives::{encode_property_value, encode_timestamp_choice};
use bacnet_types::constructed::{
    BACnetDateRange, BACnetDeviceObjectReference, BACnetObjectPropertyReference, BACnetTimeValue,
    BACnetValueSource,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use super::primitive::display_float;
use super::*;

fn encoded(values: &[PropertyValue]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for value in values {
        encode_property_value(&mut buf, value).unwrap();
    }
    buf.to_vec()
}

fn text(object_type: ObjectType, property: PropertyIdentifier, data: &[u8]) -> String {
    let texts = StateTexts::default();
    ValueFormat::new(object_type, &texts, Floats::Display).value(property, data)
}

const AI: ObjectType = ObjectType::ANALOG_INPUT;
type P = PropertyIdentifier;

fn date(year: u16, month: u8, day: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week: Date::UNSPECIFIED,
    }
}

fn time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

#[test]
fn display_floats_keep_six_significant_digits_without_trailing_zeros() {
    for (value, shown) in [
        (53.333_94, "53.3339"),
        (72.5, "72.5"),
        (84.781_75, "84.7818"),
        (48.447_575, "48.4476"),
        (55.0, "55"),
        (0.0, "0"),
        (-12.345_678, "-12.3457"),
        (0.000_125, "0.000125"),
        (0.000_012_5, "1.25e-5"),
        (999_999.4, "999999"),
        (999_999.7, "1e6"),
        (12_345_678.0, "1.23457e7"),
    ] {
        assert_eq!(display_float(value), shown, "{value}");
    }
    assert_eq!(display_float(f64::NAN), "NaN");
    // A REAL takes the same rounding, from its f32 value.
    let real = encoded(&[PropertyValue::Real(84.781_75)]);
    assert_eq!(text(AI, P::PRESENT_VALUE, &real), "84.7818");
    let texts = StateTexts::default();
    let exact = ValueFormat::new(AI, &texts, Floats::Exact);
    assert_eq!(exact.value(P::PRESENT_VALUE, &real), "84.78175");
}

#[test]
fn enumerations_and_bits_are_kebab_case_and_an_empty_bit_string_is_none() {
    let units = encoded(&[PropertyValue::Enumerated(
        EngineeringUnits::DEGREES_FAHRENHEIT.to_raw(),
    )]);
    assert_eq!(text(AI, P::UNITS, &units), "degrees-fahrenheit");
    let state = encoded(&[PropertyValue::Enumerated(1)]);
    assert_eq!(text(AI, P::EVENT_STATE, &state), "fault");
    let flags = |data: u8| {
        encoded(&[PropertyValue::BitString {
            unused_bits: 4,
            data: vec![data],
        }])
    };
    assert_eq!(
        text(AI, P::STATUS_FLAGS, &flags(0b1010_0000)),
        "in-alarm | overridden"
    );
    assert_eq!(text(AI, P::STATUS_FLAGS, &flags(0)), "none");
    let events = encoded(&[PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0b1110_0000],
    }]);
    assert_eq!(
        text(AI, P::EVENT_ENABLE, &events),
        "to-offnormal | to-fault | to-normal"
    );
    let object = ObjectIdentifier::new(AI, 3).unwrap();
    let id = encoded(&[PropertyValue::ObjectIdentifier(object)]);
    assert_eq!(text(AI, P::OBJECT_IDENTIFIER, &id), "analog-input 3");
    let kind = encoded(&[PropertyValue::Enumerated(AI.to_raw())]);
    assert_eq!(text(AI, P::OBJECT_TYPE, &kind), "analog-input");
}

#[test]
fn lists_strings_and_raw_octets() {
    let many: Vec<_> = (0..10).map(PropertyValue::Unsigned).collect();
    assert_eq!(
        text(AI, P::DESCRIPTION, &encoded(&many)),
        "[0, 1, 2, 3, 4, 5, 6, 7, +2 more] (10 items)"
    );
    assert_eq!(text(AI, P::DESCRIPTION, &[0xFF]), "[raw ff]");
    assert_eq!(text(AI, P::DESCRIPTION, &[]), "(empty)");
    let name = encoded(&[PropertyValue::CharacterString("Zone".into())]);
    assert_eq!(text(AI, P::OBJECT_NAME, &name), "\"Zone\"");
    let texts = StateTexts::default();
    let card = ValueFormat::new(ObjectType::DEVICE, &texts, Floats::Display);
    assert_eq!(card.plain(P::OBJECT_NAME, &name), "Zone");
}

#[test]
fn date_times_and_date_ranges_read_as_dates() {
    let any = encoded(&[
        PropertyValue::Date(Date {
            year: 0xFF,
            month: 0xFF,
            day: 0xFF,
            day_of_week: 0xFF,
        }),
        PropertyValue::Time(Time {
            hour: 0xFF,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        }),
    ]);
    let log = ObjectType::TREND_LOG;
    assert_eq!(text(log, P::START_TIME, &any), "unspecified");
    let set = encoded(&[
        PropertyValue::Date(Date {
            day_of_week: 1,
            ..date(2026, 10, 19)
        }),
        PropertyValue::Time(time(7, 30)),
    ]);
    assert_eq!(text(log, P::START_TIME, &set), "2026-10-19 Mon 07:30");
    let mut range = BytesMut::new();
    encode_date_range(
        &mut range,
        &BACnetDateRange {
            start_date: date(2026, 1, 1),
            end_date: date(2026, 12, 31),
        },
    );
    let schedule = ObjectType::SCHEDULE;
    assert_eq!(
        text(schedule, P::EFFECTIVE_PERIOD, &range),
        "2026-01-01 to 2026-12-31"
    );
    let open = encoded(&[
        PropertyValue::Date(date(2026, 1, 1)),
        PropertyValue::Date(Date {
            year: 0xFF,
            month: 0xFF,
            day: 0xFF,
            day_of_week: 0xFF,
        }),
    ]);
    assert_eq!(
        text(schedule, P::EFFECTIVE_PERIOD, &open),
        "from 2026-01-01"
    );
}

#[test]
fn binary_and_multi_state_values_are_named_with_their_texts() {
    let texts = StateTexts {
        active: Some("On".into()),
        inactive: Some("Off".into()),
        states: vec!["Low".into(), "Medium".into(), "High".into()],
    };
    let bo = ValueFormat::new(ObjectType::BINARY_OUTPUT, &texts, Floats::Display);
    let one = encoded(&[PropertyValue::Enumerated(1)]);
    let zero = encoded(&[PropertyValue::Enumerated(0)]);
    assert_eq!(bo.value(P::PRESENT_VALUE, &one), "active (On)");
    assert_eq!(bo.value(P::FEEDBACK_VALUE, &zero), "inactive (Off)");
    assert_eq!(bo.value(P::RELINQUISH_DEFAULT, &zero), "inactive (Off)");
    // Other enumerations of a binary object keep their own names.
    assert_eq!(bo.value(P::EVENT_STATE, &zero), "normal");
    let none = StateTexts::default();
    let bare = ValueFormat::new(ObjectType::BINARY_INPUT, &none, Floats::Display);
    assert_eq!(bare.value(P::PRESENT_VALUE, &one), "active");
    let mso = ValueFormat::new(ObjectType::MULTI_STATE_OUTPUT, &texts, Floats::Display);
    let two = encoded(&[PropertyValue::Unsigned(2)]);
    let nine = encoded(&[PropertyValue::Unsigned(9)]);
    assert_eq!(mso.value(P::PRESENT_VALUE, &two), "2 (Medium)");
    assert_eq!(mso.value(P::PRESENT_VALUE, &nine), "9");
    // An analog object's Present_Value is a number, with a unit symbol
    // where it is shown compactly.
    let ai = ValueFormat::new(AI, &none, Floats::Display);
    let real = encoded(&[PropertyValue::Real(72.5)]);
    let fahrenheit = Some(EngineeringUnits::DEGREES_FAHRENHEIT);
    assert_eq!(
        ai.with_units(P::PRESENT_VALUE, &real, fahrenheit),
        "72.5 °F"
    );
    let area = Some(EngineeringUnits::SQUARE_METERS);
    assert_eq!(
        ai.with_units(P::PRESENT_VALUE, &real, area),
        "72.5 square-meters"
    );
    let none_units = Some(EngineeringUnits::NO_UNITS);
    assert_eq!(ai.with_units(P::PRESENT_VALUE, &real, none_units), "72.5");
    assert_eq!(
        bo.with_units(P::PRESENT_VALUE, &one, fahrenheit),
        "active (On)"
    );
}

#[test]
fn constructed_values_are_decoded_with_the_element_table() {
    // Event_Time_Stamps: a date-time, a sequence number and a time.
    let mut stamps = BytesMut::new();
    for stamp in [
        BACnetTimeStamp::DateTime {
            date: date(2026, 10, 19),
            time: time(7, 0),
        },
        BACnetTimeStamp::SequenceNumber(4),
        BACnetTimeStamp::Time(time(18, 5)),
    ] {
        encode_timestamp_choice(&mut stamps, &stamp).unwrap();
    }
    assert_eq!(
        text(AI, P::EVENT_TIME_STAMPS, &stamps),
        "[2026-10-19 07:00, seq 4, 18:05]"
    );
    // The inhibit reference, and the unset one.
    let reference = |instance| {
        let mut buf = BytesMut::new();
        let object = ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap();
        encode_object_property_reference(
            &mut buf,
            &BACnetObjectPropertyReference::new(object, P::PRESENT_VALUE.to_raw()),
        );
        buf.to_vec()
    };
    assert_eq!(
        text(AI, P::EVENT_ALGORITHM_INHIBIT_REF, &reference(2)),
        "binary-value 2 present-value"
    );
    assert_eq!(
        text(AI, P::EVENT_ALGORITHM_INHIBIT_REF, &reference(4_194_303)),
        "none"
    );
    // A Weekly_Schedule, Monday first.
    let mut week = BytesMut::new();
    let monday = [
        BACnetTimeValue {
            time: time(7, 0),
            value: PropertyValue::Real(21.0),
        },
        BACnetTimeValue {
            time: time(18, 0),
            value: PropertyValue::Null,
        },
    ];
    encode_daily_schedule(&mut week, &monday).unwrap();
    encode_daily_schedule(&mut week, &[]).unwrap();
    assert_eq!(
        text(ObjectType::SCHEDULE, P::WEEKLY_SCHEDULE, &week),
        "[Mon 07:00 = 21, 18:00 = null; Tue none]"
    );
    // Value_Source: an object on another device, and none.
    let mut source = BytesMut::new();
    encode_value_source(
        &mut source,
        &BACnetValueSource::Object(BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::PROGRAM, 1).unwrap(),
        }),
    )
    .unwrap();
    let ao = ObjectType::ANALOG_OUTPUT;
    assert_eq!(text(ao, P::VALUE_SOURCE, &source), "program 1 on device 9");
    let mut nobody = BytesMut::new();
    encode_value_source(&mut nobody, &BACnetValueSource::None).unwrap();
    assert_eq!(text(ao, P::VALUE_SOURCE, &nobody), "none");
    // Octets that aren't the element fall back to the generic view.
    assert_eq!(text(AI, P::EVENT_TIME_STAMPS, &[0x0e]), "[raw 0e]");
}

#[test]
fn special_date_values_are_named() {
    use bacnet_encoding::constructed::encode_calendar_entry;
    use bacnet_types::constructed::{BACnetCalendarEntry, BACnetWeekNDay};
    let special = |month, day| Date {
        year: Date::UNSPECIFIED,
        month,
        day,
        day_of_week: Date::UNSPECIFIED,
    };
    let date = |d| encoded(&[PropertyValue::Date(d)]);
    assert_eq!(
        text(AI, P::DESCRIPTION, &date(special(14, 32))),
        "*-even-months-last-day"
    );
    assert_eq!(
        text(AI, P::DESCRIPTION, &date(special(13, 33))),
        "*-odd-months-odd-days"
    );
    assert_eq!(
        text(AI, P::DESCRIPTION, &date(special(5, 34))),
        "*-05-even-days"
    );
    let week = |month, week_of_month, day_of_week| BACnetWeekNDay {
        month,
        week_of_month,
        day_of_week,
    };
    for (entry, shown) in [
        (week(11, 4, 4), "Thu, days 22-28, month 11"),
        (week(13, 6, 5), "Fri, the final 7 days, month odd-months"),
        (
            week(0xFF, 7, 1),
            "Mon, the 7 days before the final 7, any month",
        ),
        (week(2, 5, 0xFF), "any day, days 29-31, month 02"),
    ] {
        let mut buf = BytesMut::new();
        encode_calendar_entry(&mut buf, &BACnetCalendarEntry::WeekNDay(entry));
        assert_eq!(
            text(ObjectType::CALENDAR, P::DATE_LIST, &buf),
            format!("[{shown}]")
        );
    }
}

#[test]
fn the_name_is_the_object_name_with_its_control_characters_as_spaces() {
    let named = vec![(
        P::OBJECT_NAME,
        Ok(encoded(&[PropertyValue::CharacterString(
            "Zone\nTemp\u{1b}".into(),
        )])),
    )];
    assert_eq!(object_name(&named).as_deref(), Some("Zone Temp "));
    // An error for the name alone, or no name at all: no name.
    let refused = vec![(
        P::OBJECT_NAME,
        Err(RowError::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_PROPERTY,
        })),
    )];
    assert_eq!(object_name(&refused), None);
    assert_eq!(object_name(&[]), None);
    // Not a string, or a name longer than the cap.
    assert_eq!(name_text(&encoded(&[PropertyValue::Unsigned(5)])), None);
    let long = encoded(&[PropertyValue::CharacterString("é".repeat(500))]);
    assert_eq!(name_text(&long).unwrap().chars().count(), MAX_NAME_CHARS);
}

/// A row error keeps the device's answer as data: an Error PDU (structured
/// or not) as its class and code, a reject or abort as its reason, and only
/// other failures as text.
#[test]
fn row_errors_keep_the_device_answer_typed() {
    use bacnet_types::constructed::BACnetObjectPropertyReference;
    use bacnet_types::enums::{AbortReason, RejectReason};
    use bacnet_types::error::ErrorDetail;

    let unknown = Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY));
    let plain = RowError::from(Error::Remote(RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
    }));
    assert_eq!(plain.class_code(), unknown);
    let structured = RowError::from(Error::Structured {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
        detail: Box::new(ErrorDetail::FirstFailedWriteAttempt(
            BACnetObjectPropertyReference::new(
                ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
        )),
    });
    assert_eq!(structured, plain);
    assert_eq!(structured.to_string(), "PROPERTY:UNKNOWN_PROPERTY");
    let reject = RowError::from(Error::Remote(RemoteError::Reject {
        reason: RejectReason::UNRECOGNIZED_SERVICE,
    }));
    assert_eq!(reject.class_code(), None);
    assert!(matches!(
        reject,
        RowError::Remote(RemoteError::Reject { .. })
    ));
    let abort = RowError::from(Error::Remote(RemoteError::Abort {
        reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
    }));
    assert!(matches!(abort, RowError::Remote(RemoteError::Abort { .. })));
    let other = RowError::from(Error::Encoding("bad tag".into()));
    assert_eq!(other, RowError::Other("encoding error: bad tag".into()));
    let _: &dyn std::error::Error = &other;
}

/// A string, or an element that may quote one, may hold device text; the
/// formatter's own numbers, names and plain elements don't (#2074).
#[test]
fn device_text_is_told_apart_from_the_formatters_own() {
    let element = |kind: &str, fields: Option<Fields>| Datum::Constructed {
        element: kind.into(),
        text: "x".into(),
        fields,
    };
    let named = Datum::Enumerated {
        value: 1,
        name: Some("active".into()),
    };
    for plain in [
        Datum::Real(1.0),
        named.clone(),
        element("time-stamp", None),
        Datum::List(vec![Datum::Unsigned(1), element("date-range", None)]),
        element(
            "time-stamp",
            Some(Fields::new(vec![("state".into(), named)])),
        ),
    ] {
        assert!(!plain.may_hold_device_text(), "{plain:?}");
    }
    for text in [
        Datum::Text("Fan".into()),
        Datum::List(vec![Datum::Unsigned(1), Datum::Text("Fan".into())]),
        element("name-value", None),
        element(
            "time-stamp",
            Some(Fields::new(vec![("label".into(), Datum::Text("x".into()))])),
        ),
    ] {
        assert!(text.may_hold_device_text(), "{text:?}");
    }
}

/// A State_Text entry that isn't a string keeps its place, so the states
/// after it are still named by their own entries (#2122).
#[test]
fn state_text_positions_hold_past_a_value_that_is_not_text() {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    let rows = vec![
        (
            P::STATE_TEXT,
            Ok(encoded(&[text("Off"), PropertyValue::Null, text("High")])),
        ),
        (P::ACTIVE_TEXT, Ok(encoded(&[PropertyValue::Null]))),
    ];
    let texts = StateTexts::from_rows(&rows);
    assert_eq!(texts.states, ["Off", "", "High"]);
    assert_eq!(texts.active, None);
    let msv = ValueFormat::new(ObjectType::MULTI_STATE_VALUE, &texts, Floats::Display);
    let state = |n| encoded(&[PropertyValue::Unsigned(n)]);
    assert_eq!(msv.value(P::PRESENT_VALUE, &state(1)), "1 (Off)");
    assert_eq!(msv.value(P::PRESENT_VALUE, &state(2)), "2");
    assert_eq!(msv.value(P::PRESENT_VALUE, &state(3)), "3 (High)");
}

/// State texts name the states of binary and multi-state objects only, in
/// the properties that hold one, never an array's size (#2132).
#[test]
fn state_texts_name_only_state_values() {
    use ObjectType as O;
    assert!(StateTexts::name(O::BINARY_VALUE, P::PRESENT_VALUE, None));
    assert!(StateTexts::name(
        O::MULTI_STATE_INPUT,
        P::ALARM_VALUES,
        None
    ));
    assert!(StateTexts::name(
        O::BINARY_OUTPUT,
        P::PRIORITY_ARRAY,
        Some(8)
    ));
    assert!(StateTexts::name(
        O::MULTI_STATE_OUTPUT,
        P::PRIORITY_ARRAY,
        None
    ));
    assert!(!StateTexts::name(
        O::BINARY_OUTPUT,
        P::PRIORITY_ARRAY,
        Some(0)
    ));
    assert!(!StateTexts::name(O::ANALOG_VALUE, P::PRESENT_VALUE, None));
    assert!(!StateTexts::name(O::BINARY_VALUE, P::OBJECT_NAME, None));
}

/// Cleaned texts lose their hidden characters, as object names do.
#[test]
fn state_texts_clean_like_names() {
    let texts = StateTexts {
        active: Some("On\u{202E}".into()),
        inactive: Some("Off\nline".into()),
        states: vec!["\u{200B}Low".into()],
    };
    let cleaned = texts.cleaned();
    assert_eq!(cleaned.active.as_deref(), Some("On"));
    assert_eq!(cleaned.inactive.as_deref(), Some("Off line"));
    assert_eq!(cleaned.states, ["Low"]);
}
