//! Application values as text: numbers, enumerations and bit strings named
//! for their property in kebab case, dates and times readable.

use bacnet_types::enums::{resolve_bits, resolve_enum, PropertyIdentifier, ResolvedBits};
use bacnet_types::primitives::{Date, PropertyValue, Time};

use super::Floats;
use crate::tui::app::objects::{kebab, object_label};

/// Significant digits a display float keeps.
const DISPLAY_DIGITS: usize = 6;

/// Octets shown as hex, `[raw 0e 0f]`.
pub(super) fn raw(data: &[u8]) -> String {
    format!("[raw {}]", hex(data, " "))
}

fn hex(data: &[u8], separator: &str) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(separator)
}

/// A float with at most six significant digits and no trailing zeros, in
/// positional form from 0.0001 up to 999,999.5 and as `1.5e6` beyond:
/// `53.3339`, `72.5`, `0.000125`, `1.23457e7`.
pub(super) fn display_float(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".into();
    }
    // Round to the digits first: the exponent of the rounded value decides
    // the form, so 999,999.7 becomes 1e6 rather than 1000000.
    let scientific = format!("{value:.prec$e}", prec = DISPLAY_DIGITS - 1);
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("an exponent form always has an e");
    let exponent: i32 = exponent.parse().expect("a decimal exponent");
    if (-4..DISPLAY_DIGITS as i32).contains(&exponent) {
        let decimals = (DISPLAY_DIGITS as i32 - 1 - exponent).max(0) as usize;
        trim(format!("{value:.decimals$}"))
    } else {
        format!("{}e{exponent}", trim(mantissa.to_string()))
    }
}

/// Drop the trailing zeros of a decimal fraction, and its point if nothing
/// is left after it.
fn trim(text: String) -> String {
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn float(value: f64, exact: String, floats: Floats) -> String {
    match floats {
        Floats::Display => display_float(value),
        Floats::Exact => exact,
    }
}

/// One application value of `property` as text.
pub(super) fn text(property: PropertyIdentifier, value: &PropertyValue, floats: Floats) -> String {
    match value {
        PropertyValue::Null => "null".into(),
        PropertyValue::Boolean(b) => b.to_string(),
        PropertyValue::Unsigned(n) => n.to_string(),
        PropertyValue::Signed(n) => n.to_string(),
        PropertyValue::Real(v) => float(f64::from(*v), v.to_string(), floats),
        PropertyValue::Double(v) => float(*v, v.to_string(), floats),
        PropertyValue::OctetString(bytes) if bytes.is_empty() => "(no octets)".into(),
        PropertyValue::OctetString(bytes) => format!("0x{}", hex(bytes, "")),
        // Quoted, so an empty or padded string shows.
        PropertyValue::CharacterString(s) => format!("\"{s}\""),
        PropertyValue::BitString { .. } => bits(property, value),
        PropertyValue::Enumerated(raw) => match resolve_enum(property, value) {
            Some(bacnet_types::enums::ResolvedEnum::Unknown(_)) | None => raw.to_string(),
            Some(named) => kebab(named),
        },
        PropertyValue::Date(date) => date_text(date),
        PropertyValue::Time(time) => time_text(time),
        PropertyValue::ObjectIdentifier(object) => object_label(*object),
        PropertyValue::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|item| text(property, item, floats))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PropertyValue::ApplicationData(bytes) => raw(bytes),
    }
}

/// A bit string named for `property`: its set bits in kebab case, `none`
/// when no bit is set, or the bits themselves for a property without names.
fn bits(property: PropertyIdentifier, value: &PropertyValue) -> String {
    match resolve_bits(property, value) {
        Some(ResolvedBits::Unknown { .. }) | None => plain_bits(value),
        Some(_) if !any_bit_set(value) => "none".into(),
        Some(named) => kebab(named),
    }
}

/// Whether a bit string has a bit set.
fn any_bit_set(value: &PropertyValue) -> bool {
    matches!(value, PropertyValue::BitString { data, .. } if data.iter().any(|b| *b != 0))
}

/// A bit string as its bits, first bit first: `bits(0101)`.
fn plain_bits(value: &PropertyValue) -> String {
    let PropertyValue::BitString { unused_bits, data } = value else {
        return String::new();
    };
    let total = (data.len() * 8).saturating_sub(usize::from(*unused_bits));
    let bits: String = (0..total)
        .map(|i| {
            if data[i / 8] & (0x80 >> (i % 8)) != 0 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    format!("bits({bits})")
}

/// Whether every field of a date is unspecified.
fn any_date(date: &Date) -> bool {
    [date.year, date.month, date.day, date.day_of_week] == [Date::UNSPECIFIED; 4]
}

/// Whether every field of a time is unspecified.
fn any_time(time: &Time) -> bool {
    [time.hour, time.minute, time.second, time.hundredths] == [Time::UNSPECIFIED; 4]
}

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A day of the week, Monday 1.
pub(super) fn weekday(day: u8) -> Option<&'static str> {
    DAYS.get(usize::from(day).checked_sub(1)?).copied()
}

/// A month field: its number, or the name of a special value (13 and 14
/// stand for the odd and the even months).
pub(super) fn month_text(month: u8) -> String {
    match month {
        Date::UNSPECIFIED => "*".into(),
        13 => "odd-months".into(),
        14 => "even-months".into(),
        month => format!("{month:02}"),
    }
}

/// A day-of-month field: its number, or the name of a special value (32 is
/// the month's final day, 33 and 34 its odd and even days).
fn day_text(day: u8) -> String {
    match day {
        Date::UNSPECIFIED => "*".into(),
        32 => "last-day".into(),
        33 => "odd-days".into(),
        34 => "even-days".into(),
        day => format!("{day:02}"),
    }
}

/// A date as `2026-10-19`, `*` for an unspecified field, special values by
/// name (`*-even-months-last-day`), and a weekday when one is given; `any
/// date` when nothing is specified.
pub(super) fn date_text(date: &Date) -> String {
    if any_date(date) {
        return "any date".into();
    }
    let year = if date.year == Date::UNSPECIFIED {
        "*".to_string()
    } else {
        (u16::from(date.year) + 1900).to_string()
    };
    let mut text = format!("{year}-{}-{}", month_text(date.month), day_text(date.day));
    if let Some(day) = weekday(date.day_of_week) {
        text.push(' ');
        text.push_str(day);
    }
    text
}

/// A time as `07:00`, with seconds and hundredths when they aren't zero;
/// `*` for an unspecified field, and `any time` when nothing is specified.
pub(super) fn time_text(time: &Time) -> String {
    if any_time(time) {
        return "any time".into();
    }
    let field = |value: u8| {
        if value == Time::UNSPECIFIED {
            "*".to_string()
        } else {
            format!("{value:02}")
        }
    };
    let mut text = format!("{}:{}", field(time.hour), field(time.minute));
    if time.second != 0 || time.hundredths != 0 {
        text.push(':');
        text.push_str(&field(time.second));
    }
    if time.hundredths != 0 {
        text.push('.');
        text.push_str(&field(time.hundredths));
    }
    text
}

/// A date and time: `2026-10-19 07:00`, or `unspecified` when neither is.
pub(super) fn date_time(date: &Date, time: &Time) -> String {
    match (any_date(date), any_time(time)) {
        (true, true) => "unspecified".into(),
        (false, true) => date_text(date),
        _ => format!("{} {}", date_text(date), time_text(time)),
    }
}

/// A date range: `2026-01-01 to 2026-12-31`, `from 2026-01-01`,
/// `until 2026-12-31`, or `any date` when neither end is given.
pub(super) fn date_range(start: &Date, end: &Date) -> String {
    match (any_date(start), any_date(end)) {
        (true, true) => "any date".into(),
        (false, true) => format!("from {}", date_text(start)),
        (true, false) => format!("until {}", date_text(end)),
        (false, false) => format!("{} to {}", date_text(start), date_text(end)),
    }
}
