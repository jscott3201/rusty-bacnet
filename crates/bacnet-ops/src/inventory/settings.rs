//! A record's settings and State_Text judged against the object's type
//! (#2765): a value of another datatype than the property's on that type
//! is a problem and stays out of the record, and a value a point list
//! builder would refuse is a problem kept beside the value.

use bacnet_types::datatype::{Datatype, ValueShape, standard_value_type};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue as V;

use super::{PointRecord, Problem};
use crate::format::{PropertyError, Value, decode_all, is_binary, is_multistate};

/// What is wrong with `value`, read as `property` of an object of
/// `object_type`, when it isn't one value of the datatype the standard
/// gives the property on that type: `not a Real`, `not an Unsigned`.
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
    // Octets that don't decode are of no datatype.
    let decoded = value.decode().unwrap_or(V::Null);
    let (holds, name) = judged(value_type.datatype, &decoded)?;
    (!holds).then(|| PropertyError::Other(format!("not {name}")))
}

/// Whether `value` is of `datatype`, and the datatype's name in a problem;
/// `None` for a datatype no setting has.
fn judged(datatype: Datatype, value: &V) -> Option<(bool, &'static str)> {
    let string = "a character string";
    Some(match datatype {
        Datatype::Real => (matches!(value, V::Real(_)), "a Real"),
        Datatype::Double => (matches!(value, V::Double(_)), "a Double"),
        Datatype::Unsigned => (matches!(value, V::Unsigned(_)), "an Unsigned"),
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

/// What is wrong with a State_Text as read: the first entry that isn't a
/// character string, which the record holds as an empty text so the
/// states after it keep their numbers, or octets that don't decode.
pub(super) fn state_text(value: &Value) -> Option<PropertyError> {
    let Some(entries) = decode_all(&value.raw) else {
        return Some(PropertyError::Other(
            "not an array of character strings".into(),
        ));
    };
    let state = entries
        .iter()
        .position(|entry| !matches!(entry, V::CharacterString(_)))?;
    Some(PropertyError::Other(format!(
        "state {} is not a character string",
        state + 1
    )))
}

/// The settings `record` holds that a point list builder
/// (`bacnet_objects::points::build_database`) refuses, as problems. Each
/// stays in the record as the device gave it, so a list made from the
/// record keeps what the device has; the problem says at inventory time
/// what a rebuild would refuse. On the types a point list holds:
///
/// - an analog object's COV_Increment, Min_Pres_Value, Max_Pres_Value or
///   Relinquish_Default that isn't a finite number, and a COV_Increment
///   below zero;
/// - Min_Pres_Value above Max_Pres_Value, on an analog object or an
///   Integer Value (the problem is Min_Pres_Value's, as the builder's is);
/// - a binary object's Relinquish_Default other than INACTIVE and ACTIVE,
///   and a Polarity other than NORMAL and REVERSE;
/// - a multi-state object's Relinquish_Default that names no state: 0, or
///   above the object's Number_Of_States when it was read.
pub(super) fn refused(record: &PointRecord) -> Vec<Problem> {
    let object_type = record.object.object_type();
    let analog = [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
    ]
    .contains(&object_type);
    let decoded = |field: &Option<Value>| field.as_ref().and_then(|value| value.decode().ok());
    let (min, max) = (
        decoded(&record.min_present_value),
        decoded(&record.max_present_value),
    );
    let mut problems = Vec::new();
    let settings = [
        (P::COV_INCREMENT, decoded(&record.cov_increment)),
        (P::MIN_PRES_VALUE, min.clone()),
        (P::MAX_PRES_VALUE, max.clone()),
        (P::POLARITY, decoded(&record.polarity)),
        (P::RELINQUISH_DEFAULT, decoded(&record.relinquish_default)),
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
            (P::POLARITY, Some(V::Enumerated(polarity)))
                if is_binary(object_type) && *polarity > 1 =>
            {
                Some("neither normal nor reverse".to_string())
            }
            (P::RELINQUISH_DEFAULT, Some(V::Unsigned(state))) if is_multistate(object_type) => {
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
            problems.push(Problem::new(*property, PropertyError::Other(why)));
        }
    }
    let above = match (min, max) {
        (Some(V::Real(min)), Some(V::Real(max))) => {
            analog && min.is_finite() && max.is_finite() && min > max
        }
        (Some(V::Signed(min)), Some(V::Signed(max))) => {
            object_type == ObjectType::INTEGER_VALUE && min > max
        }
        _ => false,
    };
    if above {
        let why = PropertyError::Other("above max-pres-value".into());
        problems.push(Problem::new(P::MIN_PRES_VALUE, why));
    }
    problems
}
