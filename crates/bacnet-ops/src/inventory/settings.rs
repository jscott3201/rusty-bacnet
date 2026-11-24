//! A record's values judged against the object's type (#2765): a setting
//! of another datatype than the property's on that type is a problem and
//! stays out of the record; a State_Text entry that isn't a string is a
//! problem; and a value a point list builder would refuse is a problem
//! kept beside the value.

use bacnet_encoding::primitives::{charset, decode_application_value};
use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_types::datatype::{Datatype, ValueShape, standard_value_type};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue as V;

use super::{PointRecord, Problem};
use crate::export::s223::EXPORTED_TYPES;
use crate::format::{
    Floats, PropertyError, StateTexts, Value, ValueFormat, clean_name, is_binary, is_multistate,
};

/// The most states a point list builder takes: the value of
/// `bacnet_objects::multistate::MAX_NUMBER_OF_STATES`, which this crate
/// uses only in its tests, where one holds the two equal.
pub(super) const MAX_STATES: u32 = 1024;

/// What is wrong with `value`, read as `property` of an object of
/// `object_type`, when it isn't one value of the datatype the standard
/// gives the property on that type: `not a Real (got an Unsigned, 5)`, or
/// `can't be decoded` for octets that aren't a value.
///
/// The datatype is `bacnet_types::datatype::standard_value_type`'s, the
/// stack's one reading of the Clause 12 tables, so a property whose
/// datatype depends on the type is judged by the type: a Relinquish_Default
/// is a Real on an analog output or value, an Enumerated on a binary one,
/// an Unsigned on a multi-state one and a Double on a Large Analog Value;
/// an Integer Value's COV_Increment is an Unsigned and its range ends are
/// Signed. A property with no datatype there (the Relinquish_Default of a
/// type this release doesn't know) and a datatype not judged here take
/// any value.
pub(super) fn mistyped(
    object_type: ObjectType,
    property: P,
    value: &Value,
) -> Option<PropertyError> {
    let value_type = standard_value_type(object_type, property)?;
    if value_type.shape != ValueShape::Single {
        return None;
    }
    let decoded = match values(&value.raw) {
        Ok(values) => match <[Entry<'_>; 1]>::try_from(values) {
            Ok([(one, _)]) => one,
            Err(values) => V::List(values.into_iter().map(|(value, _)| value).collect()),
        },
        Err((_, at)) => return Some(other(undecodable(&value.raw, at))),
    };
    let (holds, name) = judged(value_type.datatype, &decoded)?;
    (!holds).then(|| other(format!("not {name} ({})", got(&decoded, value))))
}

/// Whether `value` is of `datatype`, and the datatype's name in a problem;
/// `None` for a datatype no property the inventory judges has.
fn judged(datatype: Datatype, value: &V) -> Option<(bool, &'static str)> {
    let string = "a character string";
    Some(match datatype {
        Datatype::Real => (matches!(value, V::Real(_)), "a Real"),
        Datatype::Double => (matches!(value, V::Double(_)), "a Double"),
        Datatype::Unsigned => (matches!(value, V::Unsigned(_)), "an Unsigned"),
        // Current_Command_Priority's: NULL when no priority holds a
        // command.
        Datatype::OptionalUnsigned => (
            matches!(value, V::Unsigned(_) | V::Null),
            "an Unsigned or NULL",
        ),
        Datatype::Integer => (matches!(value, V::Signed(_)), "a Signed"),
        Datatype::Enumerated => (matches!(value, V::Enumerated(_)), "an Enumerated"),
        Datatype::CharacterString => (matches!(value, V::CharacterString(_)), string),
        Datatype::BitString => (matches!(value, V::BitString { .. }), "a bit string"),
        Datatype::OctetString => (matches!(value, V::OctetString(_)), "an octet string"),
        Datatype::Date => (matches!(value, V::Date(_)), "a Date"),
        Datatype::Time => (matches!(value, V::Time(_)), "a Time"),
        Datatype::DateTime => {
            // A BACnetDateTime is a Date and then a Time.
            let parts = match value {
                V::List(parts) => parts.as_slice(),
                _ => &[],
            };
            (matches!(parts, [V::Date(_), V::Time(_)]), "a date and time")
        }
        _ => return None,
    })
}

/// What the device gave instead, for a problem: its datatype and its text
/// as `shown` writes it, cleaned and cut as a name is, since a string or a
/// state's name in it is device text. A number is written bare: the
/// object's Units, which a read lends the text of every number, aren't
/// what came for an Event_State (#2620). A float far from one is written
/// in scientific form, so a Double of 1e300 stays short.
fn got(decoded: &V, shown: &Value) -> String {
    let number = match decoded {
        V::Unsigned(n) => Some(n.to_string()),
        V::Signed(n) => Some(n.to_string()),
        V::Real(n) => Some(float(f64::from(*n), n.to_string(), format!("{n:e}"))),
        V::Double(n) => Some(float(*n, n.to_string(), format!("{n:e}"))),
        _ => None,
    };
    let kind = match decoded {
        V::Null => return "got NULL".into(),
        V::List(values) if values.is_empty() => return "got no value".into(),
        V::List(_) => "several values",
        V::Boolean(_) => "a Boolean",
        V::Unsigned(_) => "an Unsigned",
        V::Signed(_) => "a Signed",
        V::Real(_) => "a Real",
        V::Double(_) => "a Double",
        V::OctetString(_) => "an octet string",
        V::CharacterString(_) => "a character string",
        V::BitString { .. } => "a bit string",
        V::Enumerated(_) => "an Enumerated",
        V::Date(_) => "a Date",
        V::Time(_) => "a Time",
        V::ObjectIdentifier(_) => "an object identifier",
        _ => "a constructed value",
    };
    let text = number.unwrap_or_else(|| clean_name(&shown.text));
    format!("got {kind}, {text}")
}

/// A float's text for a problem: `plain`, its shortest decimal, unless the
/// number is 1e15 or more, or under 1e-6 and not zero, when it is
/// `scientific`.
fn float(value: f64, plain: String, scientific: String) -> String {
    let magnitude = value.abs();
    let far = magnitude >= 1e15 || (magnitude > 0.0 && magnitude < 1e-6);
    if value.is_finite() && far {
        scientific
    } else {
        plain
    }
}

/// One application value as read, with its octets.
type Entry<'a> = (V, &'a [u8]);

/// The index and the offset of the first value that doesn't decode.
type Undecoded = (usize, usize);

/// Each application value in `data` with its octets, or where the first
/// that doesn't decode starts.
fn values(data: &[u8]) -> Result<Vec<Entry<'_>>, Undecoded> {
    let mut offset = 0;
    let mut out = Vec::new();
    while offset < data.len() {
        let (value, next) =
            decode_application_value(data, offset).map_err(|_| (out.len(), offset))?;
        out.push((value, &data[offset..next]));
        offset = next;
    }
    Ok(out)
}

/// Why the value at `at` in `data` doesn't decode, for a problem: a
/// character string names its character set, such as one this stack
/// doesn't convert (IBM/Microsoft DBCS, JIS X 0208, UCS-4).
pub(super) fn undecodable(data: &[u8], at: usize) -> String {
    let set = match decode_tag(data, at) {
        Ok((tag, start)) if tag.class == TagClass::Application && tag.number == 7 => {
            data.get(start).copied()
        }
        _ => None,
    };
    let Some(set) = set else {
        return "can't be decoded".into();
    };
    let name = match set {
        charset::UTF8 => "UTF-8".to_string(),
        charset::IBM_MICROSOFT_DBCS => "IBM/Microsoft DBCS".into(),
        charset::JIS_X_0208 => "JIS X 0208".into(),
        charset::UCS4 => "UCS-4".into(),
        charset::UCS2 => "UCS-2".into(),
        charset::ISO_8859_1 => "ISO 8859-1".into(),
        other => format!("character set {other}"),
    };
    format!("can't be decoded: a character string in {name}")
}

/// What is wrong with a State_Text as read from an object of
/// `object_type`: the first entry that isn't a character string, which
/// the record holds as an empty text so the states after it keep their
/// numbers, or the first that doesn't decode, which leaves the record no
/// state texts at all.
pub(super) fn state_text(object_type: ObjectType, value: &Value) -> Option<PropertyError> {
    let entries = match values(&value.raw) {
        Ok(entries) => entries,
        Err((index, at)) => {
            let why = undecodable(&value.raw, at);
            return Some(other(format!("state {} {why}", index + 1)));
        }
    };
    let (index, (entry, octets)) = entries
        .iter()
        .enumerate()
        .find(|(_, (entry, _))| !matches!(entry, V::CharacterString(_)))?;
    let state = index + 1;
    let texts = StateTexts::default();
    let format = ValueFormat::new(object_type, &texts, Floats::Exact);
    let shown = Value::new(
        &format,
        P::STATE_TEXT,
        u32::try_from(state).ok(),
        octets.to_vec(),
        None,
    );
    let got = got(entry, &shown);
    Some(other(format!(
        "state {state} is not a character string ({got})"
    )))
}

/// The values `record` holds that a point list builder
/// (`bacnet_objects::points::build_database`) refuses, as problems: for a
/// record of one of the [`EXPORTED_TYPES`], each refusal the builder can
/// give one point on its own. Each value stays in the record as the device
/// gave it, so a list made from the record keeps what the device has; the
/// problem says at inventory time what a rebuild would refuse. A name or
/// identifier another point also has is the builder's to find, across the
/// list.
///
/// - an empty Object_Name;
/// - an analog object's or Integer Value's Units past 65535;
/// - a multi-state object's Number_Of_States outside 1 to [`MAX_STATES`],
///   and State_Text with other than one label per state;
/// - an analog object's COV_Increment, Min_Pres_Value, Max_Pres_Value or
///   Relinquish_Default that isn't a finite number, and a COV_Increment
///   below zero;
/// - Min_Pres_Value above Max_Pres_Value, on an analog object or an
///   Integer Value (the problem is Min_Pres_Value's, as the builder's is);
/// - a Binary Input's or Output's Polarity other than NORMAL and REVERSE;
/// - a binary object's Relinquish_Default other than INACTIVE and ACTIVE,
///   and a multi-state one's that names no state: 0, or above the
///   object's Number_Of_States when it was read.
///
/// A relinquish default counts only where the point holds one: on an
/// output, and on a value with a Priority_Array
/// ([`PointRecord::commandable`]); the export leaves any other out.
pub(super) fn refused(record: &PointRecord) -> Vec<Problem> {
    type O = ObjectType;
    let object_type = record.object.object_type();
    if !EXPORTED_TYPES.contains(&object_type) {
        return Vec::new();
    }
    let analog = [O::ANALOG_INPUT, O::ANALOG_OUTPUT, O::ANALOG_VALUE].contains(&object_type);
    let multistate = is_multistate(object_type);
    let mut problems = Vec::new();
    let mut refuse = |property, why: String| {
        problems.push(Problem::new(property, PropertyError::Other(why)));
    };

    if record.name.as_deref() == Some("") {
        refuse(P::OBJECT_NAME, "empty".into());
    }
    if let Some(units) = record.units
        && units.to_raw() > 65_535
    {
        refuse(P::UNITS, format!("{} is above 65535", units.to_raw()));
    }
    if multistate && let Some(states) = record.number_of_states {
        if !(1..=MAX_STATES).contains(&states) {
            refuse(P::NUMBER_OF_STATES, format!("outside 1 to {MAX_STATES}"));
        }
        let labels = record.state_texts.states.len();
        if labels > 0 && labels != states as usize {
            refuse(
                P::STATE_TEXT,
                format!("{labels} labels for {states} states"),
            );
        }
    }

    let decoded = |field: &Option<Value>| field.as_ref().and_then(|value| value.decode().ok());
    let (min, max) = (
        decoded(&record.min_present_value),
        decoded(&record.max_present_value),
    );
    let polarity = [O::BINARY_INPUT, O::BINARY_OUTPUT].contains(&object_type);
    let settings = [
        (P::COV_INCREMENT, decoded(&record.cov_increment)),
        (P::MIN_PRES_VALUE, min.clone()),
        (P::MAX_PRES_VALUE, max.clone()),
        (P::POLARITY, decoded(&record.polarity).filter(|_| polarity)),
        (
            P::RELINQUISH_DEFAULT,
            decoded(&record.relinquish_default).filter(|_| record.commandable),
        ),
    ];
    for (property, value) in &settings {
        let why = match (*property, value) {
            (_, Some(V::Real(real))) if analog && !real.is_finite() => {
                Some("not a finite number".to_string())
            }
            (P::COV_INCREMENT, Some(V::Real(real))) if analog && *real < 0.0 => {
                Some("negative".to_string())
            }
            (P::RELINQUISH_DEFAULT, Some(V::Enumerated(state)))
                if is_binary(object_type) && *state > 1 =>
            {
                Some("neither inactive nor active".to_string())
            }
            (P::POLARITY, Some(V::Enumerated(polarity))) if *polarity > 1 => {
                Some("neither normal nor reverse".to_string())
            }
            (P::RELINQUISH_DEFAULT, Some(V::Unsigned(state))) if multistate => {
                match record.number_of_states.filter(|&states| states > 0) {
                    Some(states) if *state == 0 || *state > u64::from(states) => {
                        Some(format!("not one of the states 1 to {states}"))
                    }
                    None if *state == 0 => Some("not a state; states start at 1".to_string()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(why) = why {
            refuse(*property, why);
        }
    }
    let above = match (min, max) {
        (Some(V::Real(min)), Some(V::Real(max))) => {
            analog && min.is_finite() && max.is_finite() && min > max
        }
        (Some(V::Signed(min)), Some(V::Signed(max))) => {
            object_type == O::INTEGER_VALUE && min > max
        }
        _ => false,
    };
    if above {
        refuse(P::MIN_PRES_VALUE, "above max-pres-value".into());
    }
    problems
}

/// `why` as a problem's error.
fn other(why: String) -> PropertyError {
    PropertyError::Other(why)
}
