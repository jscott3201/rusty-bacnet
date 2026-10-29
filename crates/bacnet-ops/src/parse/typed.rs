//! Values typed for a known property: its enumeration by name, its bit
//! string by named bits, a date-time or a timestamp, each in the form the
//! formatter prints, so a value read can be pasted back as a write.

use bacnet_encoding::constructed::{ConstructedElement, ElementShape, property_element};
use bacnet_encoding::primitives::{encode_priority_date_time, encode_timestamp_choice};
use bacnet_types::bitstring::{EventTransitionBits, LimitEnable, StatusFlags};
use bacnet_types::enums::{
    BinaryPV, LifeSafetyState, ObjectType, PropertyIdentifier, ResolvedBits, ResolvedEnum,
    ServiceSupported,
};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use bytes::BytesMut;

use super::datetime::{parse_date_time, parse_time, parse_timestamp};
use super::numeric::{Number, coerce, finite, numeric};
use super::{json, parse_value, split_priority};
use crate::format::{holds_state, is_binary, is_date_time_value, is_multistate};
use crate::label::kebab;

/// Names listed in full when a typed name is wrong; a longer list gives
/// the nearest few.
const MAX_LISTED: usize = 12;

/// Nearest names offered for a long list.
const NEAREST: usize = 5;

/// An enumeration: its type name and named values.
struct Enumeration {
    type_name: &'static str,
    names: Vec<(&'static str, u32)>,
}

/// A named bit string: each name's bit, and how many bits it has.
struct NamedBits {
    type_name: &'static str,
    names: Vec<(&'static str, usize)>,
    length: usize,
}

/// What a property holds, as far as typed input needs to know.
enum Kind {
    /// An enumeration; `states` when an object's state text may follow
    /// the name, `active (On)`.
    Enum {
        enumeration: Enumeration,
        states: bool,
    },
    /// A multi-state object's state number, maybe with its text after it.
    State,
    Bits(NamedBits),
    /// A BACnetDateTime: a date then a time.
    DateTime,
    /// A DateTime Value's Priority_Array slot: the BACnetPriorityValue
    /// choice of a date-time, under context tag 1.
    PriorityDateTime,
    /// A number sent as the property's numeric datatype.
    Number(Number),
    /// One BACnetTimeStamp.
    TimeStamp,
    /// A constructed element typed as JSON, or a list of them when
    /// `whole`.
    Json {
        element: ConstructedElement,
        whole: bool,
    },
    /// Nothing known: the untyped grammar.
    Untyped,
}

/// The enumeration a resolved value belongs to.
fn enumeration(value: ResolvedEnum) -> Option<Enumeration> {
    value
        .enumeration()
        .map(|(type_name, names)| Enumeration { type_name, names })
}

/// The bit names of a bitflags type, in declaration order, which is their
/// order on the wire (bit 0 first).
fn flag_names<I: Iterator<Item = (&'static str, F)>, F>(names: I) -> Vec<(&'static str, usize)> {
    names
        .enumerate()
        .map(|(bit, (name, _))| (name, bit))
        .collect()
}

/// The named bit string a property holds, if any.
fn named_bits(property: PropertyIdentifier) -> Option<NamedBits> {
    let (type_name, names) = match ResolvedBits::from_property(property, 0, &[]) {
        ResolvedBits::StatusFlags(_) => {
            ("StatusFlags", flag_names(StatusFlags::all().iter_names()))
        }
        ResolvedBits::EventTransitionBits(_) => (
            "EventTransitionBits",
            flag_names(EventTransitionBits::all().iter_names()),
        ),
        ResolvedBits::LimitEnable(_) => {
            ("LimitEnable", flag_names(LimitEnable::all().iter_names()))
        }
        ResolvedBits::ServicesSupported(_) => (
            "ServicesSupported",
            ServiceSupported::ALL_NAMED
                .iter()
                .map(|&(name, value)| (name, usize::from(value.to_raw())))
                .collect(),
        ),
        ResolvedBits::ObjectTypesSupported(_) => (
            "ObjectTypesSupported",
            ObjectType::ALL_NAMED
                .iter()
                .map(|&(name, value)| (name, value.to_raw() as usize))
                .collect(),
        ),
        ResolvedBits::Unknown { .. } => return None,
    };
    let length = names.iter().map(|&(_, bit)| bit + 1).max().unwrap_or(0);
    Some(NamedBits {
        type_name,
        names,
        length,
    })
}

/// The properties that hold a BACnetDateTime, by object type (the
/// property tables of Clause 12).
fn holds_date_time(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    type O = ObjectType;
    type P = PropertyIdentifier;
    match property {
        P::CHANGE_OF_STATE_TIME | P::TIME_OF_STATE_COUNT_RESET | P::TIME_OF_ACTIVE_TIME_RESET => {
            is_binary(object_type)
        }
        P::START_TIME => [
            O::TREND_LOG,
            O::TREND_LOG_MULTIPLE,
            O::EVENT_LOG,
            O::LOAD_CONTROL,
        ]
        .contains(&object_type),
        P::STOP_TIME => [O::TREND_LOG, O::TREND_LOG_MULTIPLE, O::EVENT_LOG].contains(&object_type),
        P::MODIFICATION_DATE => object_type == O::FILE,
        P::UPDATE_TIME => [O::PULSE_CONVERTER, O::TIMER].contains(&object_type),
        P::COUNT_CHANGE_TIME => object_type == O::PULSE_CONVERTER,
        P::EXPIRATION_TIME => [O::TIMER, O::ACCESS_CREDENTIAL].contains(&object_type),
        P::ACTIVATION_TIME => object_type == O::ACCESS_CREDENTIAL,
        P::VALUE_CHANGE_TIME => object_type == O::ACCUMULATOR,
        P::PRESENT_VALUE | P::RELINQUISH_DEFAULT => is_date_time_value(object_type),
        _ => false,
    }
}

/// Whether an object type is a life safety point or zone.
fn is_life_safety(object_type: ObjectType) -> bool {
    [ObjectType::LIFE_SAFETY_POINT, ObjectType::LIFE_SAFETY_ZONE].contains(&object_type)
}

/// The life safety properties whose values are a BACnetLifeSafetyState.
fn holds_life_safety_state(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    is_life_safety(object_type)
        && [
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::ALARM_VALUES,
            PropertyIdentifier::FAULT_VALUES,
            PropertyIdentifier::LIFE_SAFETY_ALARM_VALUES,
        ]
        .contains(&property)
}

fn kind(object_type: ObjectType, property: PropertyIdentifier, index: Option<u32>) -> Kind {
    // Index 0 is an array's size, a count.
    if index == Some(0) {
        return Kind::Untyped;
    }
    if let Some(number) = numeric(object_type, property, index) {
        return Kind::Number(number);
    }
    if property == PropertyIdentifier::PRIORITY_ARRAY
        && index.is_some()
        && is_date_time_value(object_type)
    {
        return Kind::PriorityDateTime;
    }
    if holds_state(property) && is_binary(object_type) {
        let enumeration = enumeration(ResolvedEnum::BinaryPV(BinaryPV::INACTIVE));
        return enumeration.map_or(Kind::Untyped, |enumeration| Kind::Enum {
            enumeration,
            states: true,
        });
    }
    if holds_state(property) && is_multistate(object_type) {
        return Kind::State;
    }
    if holds_life_safety_state(object_type, property) {
        let state = ResolvedEnum::LifeSafetyState(LifeSafetyState::QUIET);
        if let Some(enumeration) = enumeration(state) {
            return Kind::Enum {
                enumeration,
                states: false,
            };
        }
    }
    // Tracking_Value is a BACnetLifeSafetyState only on the life safety
    // objects; a Lighting Output's is a REAL.
    let tracking = property == PropertyIdentifier::TRACKING_VALUE;
    if (!tracking || is_life_safety(object_type))
        && let Some(enumeration) = enumeration(ResolvedEnum::from_property(property, 0))
    {
        return Kind::Enum {
            enumeration,
            states: false,
        };
    }
    if let Some(bits) = named_bits(property) {
        return Kind::Bits(bits);
    }
    if holds_date_time(object_type, property) {
        return Kind::DateTime;
    }
    match property_element(object_type, property) {
        Some((ConstructedElement::TimeStamp, shape))
            if shape == ElementShape::Single || index.is_some() =>
        {
            Kind::TimeStamp
        }
        Some((element, shape)) if json::takes_json(element) => Kind::Json {
            element,
            whole: shape == ElementShape::Collection && index.is_none(),
        },
        _ => Kind::Untyped,
    }
}

/// Whether `text` is one of the untyped grammar's explicit forms, which
/// say their own datatype whatever the property.
fn explicit(text: &str) -> bool {
    ["null", "true", "false"]
        .iter()
        .any(|word| text.eq_ignore_ascii_case(word))
        || text.starts_with('"')
        || ["enumerated:", "date:", "time:", "object:", "bits("]
            .iter()
            .any(|prefix| text.starts_with(prefix))
}

/// A name with its separators dropped and its case folded, so
/// `IN_ALARM`, `in-alarm` and `InAlarm` compare equal.
fn folded(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_' && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Edit distance between two names.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

/// The error for a name that isn't one of `names`: every name when there
/// are few, the nearest few otherwise, all in kebab case.
fn unknown_name<'a>(
    what: &str,
    typed: &str,
    names: impl Iterator<Item = &'a str>,
    number: &str,
) -> String {
    let names: Vec<&str> = names.collect();
    let (label, shown): (&str, Vec<&str>) = if names.len() <= MAX_LISTED {
        ("expected one of", names)
    } else {
        let typed = folded(typed);
        let mut ranked: Vec<(bool, usize, &str)> = names
            .into_iter()
            .map(|name| {
                let name_folded = folded(name);
                (
                    !name_folded.contains(&typed),
                    distance(&typed, &name_folded),
                    name,
                )
            })
            .collect();
        ranked.sort();
        (
            "nearest",
            ranked
                .into_iter()
                .take(NEAREST)
                .map(|(_, _, n)| n)
                .collect(),
        )
    };
    let shown: Vec<String> = shown.into_iter().map(kebab).collect();
    format!(
        "unknown {what} '{typed}'; {label}: {}{number}",
        shown.join(", ")
    )
}

/// An enumeration value by name in any common spelling, or its number.
fn enum_value(enumeration: &Enumeration, text: &str) -> Result<u32, String> {
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return text
            .parse::<u32>()
            .map_err(|_| format!("{text} is out of an enumeration's range (0-4294967295)"));
    }
    let typed = folded(text);
    enumeration
        .names
        .iter()
        .find(|(name, _)| folded(name) == typed)
        .map(|&(_, value)| value)
        .ok_or_else(|| {
            unknown_name(
                enumeration.type_name,
                text,
                enumeration.names.iter().map(|(name, _)| *name),
                " (or a number)",
            )
        })
}

/// A state name with the object's text after it, `active (On)`, as the
/// name alone. The first ` (` starts the text, since no state name or
/// number holds one; the text itself may (`2 (Low (eco))`).
fn without_state_text(text: &str) -> &str {
    match text.find(" (") {
        Some(at) if text.ends_with(')') => text[..at].trim_end(),
        _ => text,
    }
}

/// A list's items split at the commas outside parentheses, so a state's
/// text keeps its own commas: `active (On, Auto), inactive`.
fn list_items(inner: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (at, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                items.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    items.push(&inner[start..]);
    items
}

/// An enumeration value, or a bracketed list of them as the formatter
/// prints a list, `[high-limit, low-limit]`.
fn parse_enum(
    enumeration: &Enumeration,
    states: bool,
    text: &str,
) -> Result<PropertyValue, String> {
    let one = |text: &str| {
        let text = text.trim();
        let text = if states {
            without_state_text(text)
        } else {
            text
        };
        enum_value(enumeration, text.trim()).map(PropertyValue::Enumerated)
    };
    match text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        Some(inner) if inner.trim().is_empty() => Ok(PropertyValue::List(Vec::new())),
        Some(inner) => list_items(inner)
            .into_iter()
            .map(one)
            .collect::<Result<_, _>>()
            .map(PropertyValue::List),
        None => one(text),
    }
}

/// A bit string from its set bits' names (`in-alarm,fault` or
/// `in-alarm | fault`), its bits as 0/1 digits first bit first, or
/// `none` for no bit set.
fn parse_bits(bits: &NamedBits, text: &str) -> Result<PropertyValue, String> {
    let mut set = vec![false; bits.length];
    // An empty value is refused rather than read as no bit set: a blank
    // field would otherwise clear every bit.
    if text.is_empty() {
        return Err(format!(
            "a {} value is needed: its bits' names, the bits as 0/1 digits, or none",
            bits.type_name
        ));
    }
    let digits = text.bytes().all(|b| b == b'0' || b == b'1');
    if digits {
        set = text.bytes().map(|b| b == b'1').collect();
    } else if !text.eq_ignore_ascii_case("none") {
        for name in text.split([',', '|']).map(str::trim) {
            if name.is_empty() {
                continue;
            }
            let typed = folded(name);
            let bit = bits
                .names
                .iter()
                .find(|(known, _)| folded(known) == typed)
                .map(|&(_, bit)| bit)
                .ok_or_else(|| {
                    unknown_name(
                        &format!("{} bit", bits.type_name),
                        name,
                        bits.names.iter().map(|(known, _)| *known),
                        " (or the bits as 0/1 digits, or none)",
                    )
                })?;
            set[bit] = true;
        }
    }
    Ok(bit_string(&set))
}

/// Bits, first bit first, as a BitString.
pub(super) fn bit_string(set: &[bool]) -> PropertyValue {
    let mut data = vec![0u8; set.len().div_ceil(8)];
    for (bit, _) in set.iter().enumerate().filter(|(_, on)| **on) {
        data[bit / 8] |= 0x80 >> (bit % 8);
    }
    PropertyValue::BitString {
        unused_bits: (data.len() * 8 - set.len()) as u8,
        data,
    }
}

/// A date and a time, the time unspecified when none is typed.
fn parse_date_time_value(text: &str) -> Result<PropertyValue, String> {
    let (date, time) = parse_date_and_time(text)?;
    Ok(PropertyValue::List(vec![
        PropertyValue::Date(date),
        PropertyValue::Time(time),
    ]))
}

/// The typed date with its time, which is unspecified when only the date is given.
fn parse_date_and_time(text: &str) -> Result<(Date, Time), String> {
    let (date, time) = parse_date_time(text)?;
    let time: Time = match time {
        Some(time) => time,
        None => parse_time("any time")?,
    };
    Ok((date, time))
}

/// Parse a value typed for `property` of an object of `object_type`, at
/// `index` when the write names one, knowing the property's datatype:
///
/// - an enumeration by name, in kebab case, UPPER_SNAKE or any case
///   (`offnormal`, `HIGH_LIMIT`), or by number; a binary object's state as
///   `active`/`inactive`, with or without the object's text after it
///   (`active (On)`);
/// - a named bit string by its set bits (`in-alarm,fault`,
///   `in-alarm | fault`), as 0/1 digits first bit first, or `none` for
///   no bit set;
/// - a plain number for the value of an analog or integer object (its
///   Present_Value, Relinquish_Default, a Priority_Array slot, COV_Increment,
///   High_Limit, Low_Limit and Deadband) as that property's datatype: REAL
///   for the analog objects, Double for a Large Analog Value, INTEGER or
///   Unsigned for the integer objects;
/// - a BACnetDateTime as `2026-10-19 07:00` (a date alone leaves the time
///   unspecified), and a timestamp as `seq 42`, a time, or a date and time;
/// - a Weekly_Schedule day, an Exception_Schedule special event, a
///   Date_List calendar entry or a Recipient_List destination as JSON, or
///   the whole list of them for a write of the whole property, in the
///   shapes of the Python bindings' typed constructors (`docs/CLI.md`,
///   "Constructed values as JSON");
/// - anything else, and the explicit forms (`null`, `true`, a quoted
///   string, `enumerated:N`, `date:`, `time:`, `object:`, `bits(0101)`),
///   as [`parse_value`] reads them.
///
/// A mistyped name is refused with the valid names, or the nearest few
/// when there are many.
pub fn parse_value_for(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    text: &str,
) -> Result<PropertyValue, String> {
    let text = text.trim();
    if explicit(text) {
        return parse_value(text);
    }
    match kind(object_type, property, index) {
        Kind::Enum {
            enumeration,
            states,
        } => parse_enum(&enumeration, states, text),
        Kind::State => {
            let state = without_state_text(text);
            state.parse::<u64>().map(PropertyValue::Unsigned).map_err(|_| {
                format!("'{state}' is not a state number; a multi-state state is its number, from 1, its text allowed after it: `2 (Heat)`")
            })
        }
        Kind::Number(Number::Double) => match text.parse::<f64>() {
            // Read straight as a Double, so no digit is lost through a REAL.
            Ok(double) if text.contains(['.', 'e', 'E']) => finite(PropertyValue::Double(double)),
            _ => coerce(Number::Double, parse_value(text)?),
        },
        Kind::Number(number) => coerce(number, parse_value(text)?),
        Kind::PriorityDateTime => {
            let (date, time) = parse_date_and_time(text)?;
            let mut buf = BytesMut::new();
            encode_priority_date_time(&mut buf, &date, &time);
            Ok(PropertyValue::ApplicationData(buf.to_vec()))
        }
        Kind::Bits(bits) => parse_bits(&bits, text),
        Kind::DateTime => parse_date_time_value(text),
        Kind::TimeStamp => {
            let stamp = parse_timestamp(text)?;
            let mut buf = BytesMut::new();
            encode_timestamp_choice(&mut buf, &stamp)
                .map_err(|e| format!("cannot encode the timestamp: {e}"))?;
            Ok(PropertyValue::ApplicationData(buf.to_vec()))
        }
        Kind::Json { element, whole } if text.starts_with(['[', '{']) => {
            json::parse(element, whole, text)
        }
        Kind::Json { .. } | Kind::Untyped => parse_value(text),
    }
}

/// [`parse_value_for`] with an optional `@priority` suffix, as
/// [`parse_value_with_priority`](super::parse_value_with_priority) splits
/// it.
pub fn parse_value_for_with_priority(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    text: &str,
) -> Result<(PropertyValue, Option<u8>), String> {
    let (value, priority) = split_priority(text)?;
    Ok((
        parse_value_for(object_type, property, index, value)?,
        priority,
    ))
}
