//! Constructed values typed as JSON: Weekly_Schedule days,
//! Exception_Schedule special events, Date_List calendar entries and
//! Recipient_List destinations.
//!
//! The shapes mirror the Python bindings' typed constructors, keys and
//! all, with dates and times written as the formatter prints them:
//!
//! - a day is a list of entries, each `["07:00", 72.5]` or
//!   `{"time": "07:00", "value": 72.5}`; a week is seven days, Monday first;
//! - a calendar entry is `{"kind": "date", "date": "2026-12-25"}`,
//!   `{"kind": "date_range", "start_date": ..., "end_date": ...}` or
//!   `{"kind": "week_n_day", "month": 12, "week_of_month": 1,
//!   "day_of_week": 1}` (255 or `null` for any);
//! - a special event is `{"period": <calendar entry or "calendar:1">,
//!   "time_values": <day>, "priority": 16}`;
//! - a destination is `{"recipient": "device:99", "process_identifier": 7}`
//!   with optional `valid_days`, `from_time`, `to_time`,
//!   `issue_confirmed_notifications` and `transitions`, defaulting to every
//!   day, all day, every transition, unconfirmed.
//!
//! An entry's value is a JSON null, boolean or number, or a string in the
//! untyped value grammar (`"active"`, `"enumerated:3"`, `"\"text\""`).
//! Unknown fields are refused, each error naming where it is.

use bacnet_encoding::constructed::{
    ConstructedElement, encode_calendar_entry, encode_daily_schedule, encode_destination,
    encode_special_event,
};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetCalendarEntry, BACnetDateRange, BACnetDestination, BACnetRecipient,
    BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay, SpecialEventPeriod,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;
use serde_json::{Map, Value};

use super::datetime::{parse_date, parse_time};
use super::numeric::finite;
use super::{parse_object_specifier, parse_value};

type Fields = Map<String, Value>;

/// The constructed elements typed as JSON.
pub(super) fn takes_json(element: ConstructedElement) -> bool {
    matches!(
        element,
        ConstructedElement::DailySchedule
            | ConstructedElement::SpecialEvent
            | ConstructedElement::CalendarEntry
            | ConstructedElement::Destination
    )
}

/// The JSON `text` as the element, or a list of them when `whole`,
/// encoded.
pub(super) fn parse(
    element: ConstructedElement,
    whole: bool,
    text: &str,
) -> Result<PropertyValue, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let name = element_name(element);
    let mut buf = BytesMut::new();
    let items: Vec<(String, &Value)> = if whole {
        let list = value
            .as_array()
            .ok_or_else(|| format!("expected a JSON list of {name} values"))?;
        if element == ConstructedElement::DailySchedule && list.len() != 7 {
            return Err(format!(
                "a weekly schedule is 7 days, Monday first, got {}",
                list.len()
            ));
        }
        list.iter()
            .enumerate()
            .map(|(index, item)| (format!("{name}[{index}]"), item))
            .collect()
    } else {
        vec![(name.to_string(), &value)]
    };
    for (path, item) in items {
        encode(element, &path, item, &mut buf)?;
    }
    Ok(PropertyValue::ApplicationData(buf.to_vec()))
}

fn element_name(element: ConstructedElement) -> &'static str {
    match element {
        ConstructedElement::DailySchedule => "day",
        ConstructedElement::SpecialEvent => "special-event",
        ConstructedElement::CalendarEntry => "calendar-entry",
        _ => "destination",
    }
}

fn encode(
    element: ConstructedElement,
    path: &str,
    value: &Value,
    buf: &mut BytesMut,
) -> Result<(), String> {
    let refused = |e: bacnet_types::error::Error| format!("{path}: {e}");
    match element {
        ConstructedElement::DailySchedule => {
            encode_daily_schedule(buf, &time_values(path, value)?).map_err(refused)
        }
        ConstructedElement::SpecialEvent => {
            encode_special_event(buf, &special_event(path, value)?).map_err(refused)
        }
        ConstructedElement::CalendarEntry => {
            encode_calendar_entry(buf, &calendar_entry(path, value)?);
            Ok(())
        }
        _ => encode_destination(buf, &destination(path, value)?).map_err(refused),
    }
}

/// `value` as an object with every `required` key and no key outside
/// `required` and `optional`.
fn object<'a>(
    path: &str,
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a Fields, String> {
    let fields = value
        .as_object()
        .ok_or_else(|| format!("{path}: expected a JSON object"))?;
    if let Some(unknown) = fields
        .keys()
        .find(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        let mut known: Vec<&str> = required.iter().chain(optional).copied().collect();
        known.sort_unstable();
        return Err(format!(
            "{path}: unknown field '{unknown}'; the fields are {}",
            known.join(", ")
        ));
    }
    if let Some(missing) = required.iter().find(|key| !fields.contains_key(**key)) {
        return Err(format!("{path}: missing field '{missing}'"));
    }
    Ok(fields)
}

fn string<'a>(path: &str, value: &'a Value) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{path}: expected a string"))
}

fn integer<T: TryFrom<u64>>(path: &str, value: &Value) -> Result<T, String> {
    value
        .as_u64()
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("{path}: expected a whole number in range, got {value}"))
}

fn time(path: &str, value: &Value) -> Result<Time, String> {
    parse_time(string(path, value)?).map_err(|e| format!("{path}: {e}"))
}

fn object_identifier(path: &str, value: &Value) -> Result<ObjectIdentifier, String> {
    let text = string(path, value)?;
    let (object_type, instance) =
        parse_object_specifier(text).map_err(|e| format!("{path}: {e}"))?;
    ObjectIdentifier::new(object_type, instance).map_err(|e| format!("{path}: {e}"))
}

/// An entry's value: null, a boolean, a number (whole numbers Unsigned or
/// Signed, others REAL), or a string in the untyped grammar.
fn primitive(path: &str, value: &Value) -> Result<PropertyValue, String> {
    match value {
        Value::Null => Ok(PropertyValue::Null),
        Value::Bool(on) => Ok(PropertyValue::Boolean(*on)),
        Value::Number(n) => {
            if let Some(n) = n.as_u64() {
                Ok(PropertyValue::Unsigned(n))
            } else if let Some(n) = n.as_i64() {
                i32::try_from(n)
                    .map(PropertyValue::Signed)
                    .map_err(|_| format!("{path}: {n} is out of a Signed's range"))
            } else {
                real(path, n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(text) => parse_value(text).map_err(|e| format!("{path}: {e}")),
        _ => Err(format!(
            "{path}: expected null, a boolean, a number or a string"
        )),
    }
}

/// A JSON number with a fraction or an exponent (`72.5`, `1e20`), or a
/// whole number past an Unsigned's or a Signed's range, as a REAL, refused
/// past a REAL's range.
fn real(path: &str, value: f64) -> Result<PropertyValue, String> {
    finite(PropertyValue::Real(value as f32)).map_err(|e| format!("{path}: {value:e}: {e}"))
}

/// A day's entries.
fn time_values(path: &str, value: &Value) -> Result<Vec<BACnetTimeValue>, String> {
    let entries = value
        .as_array()
        .ok_or_else(|| format!("{path}: expected a list of [time, value] entries"))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let path = format!("{path}[{index}]");
            let (time_json, value_json) = match entry {
                Value::Array(pair) if pair.len() == 2 => (&pair[0], &pair[1]),
                Value::Object(_) => {
                    let fields = object(&path, entry, &["time", "value"], &[])?;
                    (&fields["time"], &fields["value"])
                }
                _ => {
                    return Err(format!(
                        "{path}: expected [time, value] or {{\"time\": ..., \"value\": ...}}"
                    ));
                }
            };
            Ok(BACnetTimeValue {
                time: time(&format!("{path}.time"), time_json)?,
                value: primitive(&format!("{path}.value"), value_json)?,
            })
        })
        .collect()
}

fn week_n_day_field(path: &str, value: Option<&Value>) -> Result<u8, String> {
    match value {
        None | Some(Value::Null) => Ok(BACnetWeekNDay::ANY),
        Some(value) => integer(path, value),
    }
}

fn calendar_entry(path: &str, value: &Value) -> Result<BACnetCalendarEntry, String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path}: expected a \"kind\" of date, date_range or week_n_day"))?;
    let date = |fields: &Fields, key: &str| {
        let path = format!("{path}.{key}");
        parse_date(string(&path, &fields[key])?).map_err(|e| format!("{path}: {e}"))
    };
    match kind {
        "date" => {
            let fields = object(path, value, &["kind", "date"], &[])?;
            Ok(BACnetCalendarEntry::Date(date(fields, "date")?))
        }
        "date_range" => {
            let fields = object(path, value, &["kind", "start_date", "end_date"], &[])?;
            Ok(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: date(fields, "start_date")?,
                end_date: date(fields, "end_date")?,
            }))
        }
        "week_n_day" => {
            let keys = ["month", "week_of_month", "day_of_week"];
            let fields = object(path, value, &["kind"], &keys)?;
            let field = |key: &str| week_n_day_field(&format!("{path}.{key}"), fields.get(key));
            Ok(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: field("month")?,
                week_of_month: field("week_of_month")?,
                day_of_week: field("day_of_week")?,
            }))
        }
        kind => Err(format!(
            "{path}.kind: '{kind}' is not date, date_range or week_n_day"
        )),
    }
}

fn special_event(path: &str, value: &Value) -> Result<BACnetSpecialEvent, String> {
    let fields = object(path, value, &["period", "time_values", "priority"], &[])?;
    let period_path = format!("{path}.period");
    let period = match &fields["period"] {
        Value::String(_) => {
            let calendar = object_identifier(&period_path, &fields["period"])?;
            if calendar.object_type() != ObjectType::CALENDAR {
                return Err(format!(
                    "{period_path}: a referenced period is a Calendar object"
                ));
            }
            SpecialEventPeriod::CalendarReference(calendar)
        }
        entry => SpecialEventPeriod::CalendarEntry(calendar_entry(&period_path, entry)?),
    };
    Ok(BACnetSpecialEvent {
        period,
        list_of_time_values: time_values(&format!("{path}.time_values"), &fields["time_values"])?,
        event_priority: priority(&format!("{path}.priority"), &fields["priority"])?,
    })
}

/// A special event's priority, 1 (highest) to 16.
fn priority(path: &str, value: &Value) -> Result<u64, String> {
    match integer::<u64>(path, value) {
        Ok(priority @ 1..=16) => Ok(priority),
        _ => Err(format!(
            "{path}: expected a priority from 1 to 16, got {value}"
        )),
    }
}

fn recipient(path: &str, value: &Value) -> Result<BACnetRecipient, String> {
    if value.is_string() {
        return object_identifier(path, value).map(BACnetRecipient::Device);
    }
    match value.get("kind").and_then(Value::as_str) {
        Some("device") => {
            let fields = object(path, value, &["kind", "object_identifier"], &[])?;
            object_identifier(
                &format!("{path}.object_identifier"),
                &fields["object_identifier"],
            )
            .map(BACnetRecipient::Device)
        }
        Some("address") => {
            let fields = object(path, value, &["kind", "network_number", "mac_address"], &[])?;
            let mac_path = format!("{path}.mac_address");
            let mac = hex(&mac_path, string(&mac_path, &fields["mac_address"])?)?;
            Ok(BACnetRecipient::Address(BACnetAddress {
                network_number: integer(
                    &format!("{path}.network_number"),
                    &fields["network_number"],
                )?,
                mac_address: MacAddr::from_slice(&mac),
            }))
        }
        _ => Err(format!(
            "{path}: expected \"device:N\" or an object whose \"kind\" is device or address"
        )),
    }
}

/// Octets as hex digits, `0a000064bac0`; colons or spaces between them
/// are allowed.
fn hex(path: &str, text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(|c| !matches!(c, ':' | ' ')).collect();
    if !digits.len().is_multiple_of(2) || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{path}: expected hex octets, got '{text}'"));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|e| format!("{path}: {e}")))
        .collect()
}

/// Named flags from a list of names, or the bits as a number.
fn flags<I: Iterator<Item = (&'static str, u8)>>(
    path: &str,
    value: &Value,
    names: impl Fn() -> I,
    maximum: u8,
) -> Result<u8, String> {
    if value.is_number() {
        let bits: u8 = integer(path, value)?;
        return if bits <= maximum {
            Ok(bits)
        } else {
            Err(format!("{path}: bits above {maximum}"))
        };
    }
    let list = value
        .as_array()
        .ok_or_else(|| format!("{path}: expected a list of names"))?;
    let mut bits = 0;
    for item in list {
        let name = string(path, item)?;
        let folded = |text: &str| text.replace(['-', '_'], "").to_ascii_lowercase();
        let typed = folded(name);
        // A name or its first three letters or more: `mon`, `monday`.
        let bit = names()
            .find(|(known, _)| {
                let known = folded(known);
                known == typed || (typed.len() >= 3 && known.starts_with(&typed))
            })
            .map(|(_, bit)| bit)
            .ok_or_else(|| {
                let known: Vec<String> = names().map(|(n, _)| crate::label::kebab(n)).collect();
                format!(
                    "{path}: unknown name '{name}'; expected one of: {}",
                    known.join(", ")
                )
            })?;
        bits |= bit;
    }
    Ok(bits)
}

fn destination(path: &str, value: &Value) -> Result<BACnetDestination, String> {
    let optional = [
        "valid_days",
        "from_time",
        "to_time",
        "issue_confirmed_notifications",
        "transitions",
    ];
    let fields = object(path, value, &["recipient", "process_identifier"], &optional)?;
    let field = |key: &str| format!("{path}.{key}");
    let time_or = |key: &str, default: Time| {
        fields
            .get(key)
            .map_or(Ok(default), |value| time(&field(key), value))
    };
    let valid_days = match fields.get("valid_days") {
        None => DaysOfWeek::all(),
        Some(value) => DaysOfWeek::from_bits_truncate(flags(
            &field("valid_days"),
            value,
            || DaysOfWeek::all().iter_names().map(|(n, f)| (n, f.bits())),
            DaysOfWeek::all().bits(),
        )?),
    };
    let transitions = match fields.get("transitions") {
        None => EventTransitionBits::all(),
        Some(value) => EventTransitionBits::from_bits_truncate(flags(
            &field("transitions"),
            value,
            || {
                EventTransitionBits::all()
                    .iter_names()
                    .map(|(n, f)| (n, f.bits()))
            },
            EventTransitionBits::all().bits(),
        )?),
    };
    let issue_confirmed_notifications = match fields.get("issue_confirmed_notifications") {
        None => false,
        Some(Value::Bool(on)) => *on,
        Some(_) => {
            return Err(format!(
                "{}: expected true or false",
                field("issue_confirmed_notifications")
            ));
        }
    };
    Ok(BACnetDestination {
        valid_days,
        from_time: time_or(
            "from_time",
            Time {
                hour: 0,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        )?,
        to_time: time_or(
            "to_time",
            Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            },
        )?,
        recipient: recipient(&field("recipient"), &fields["recipient"])?,
        process_identifier: integer(&field("process_identifier"), &fields["process_identifier"])?,
        issue_confirmed_notifications,
        transitions,
    })
}
