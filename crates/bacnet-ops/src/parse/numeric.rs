//! The numeric datatype of the value properties of the analog and integer
//! objects (the property tables of Clause 12), so a plain number typed for
//! one is sent as that datatype: `72` for an Analog Value's Present_Value
//! is REAL 72.0, not Unsigned 72.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

/// A numeric datatype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Number {
    Real,
    Double,
    Signed,
    Unsigned,
}

/// The numeric datatype of `property` at `index`, when the table knows it.
pub(super) fn numeric(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Option<Number> {
    type O = ObjectType;
    type P = PropertyIdentifier;
    // The value itself, and the limits that share its datatype.
    let value = match property {
        P::PRESENT_VALUE | P::RELINQUISH_DEFAULT | P::HIGH_LIMIT | P::LOW_LIMIT => index.is_none(),
        P::PRIORITY_ARRAY => index.is_some_and(|slot| slot > 0),
        P::COV_INCREMENT | P::DEADBAND => {
            // An Integer Value counts these as Unsigned (Table 12-50).
            return match object_type {
                O::ANALOG_INPUT | O::ANALOG_OUTPUT | O::ANALOG_VALUE => Some(Number::Real),
                O::LARGE_ANALOG_VALUE => Some(Number::Double),
                O::INTEGER_VALUE | O::POSITIVE_INTEGER_VALUE => Some(Number::Unsigned),
                _ => None,
            }
            .filter(|_| index.is_none());
        }
        _ => false,
    };
    if !value {
        return None;
    }
    match object_type {
        O::ANALOG_INPUT | O::ANALOG_OUTPUT | O::ANALOG_VALUE => Some(Number::Real),
        O::LARGE_ANALOG_VALUE => Some(Number::Double),
        O::INTEGER_VALUE => Some(Number::Signed),
        O::POSITIVE_INTEGER_VALUE => Some(Number::Unsigned),
        _ => None,
    }
}

/// A REAL or Double that is finite; one past its datatype's range (`1e39`
/// as a REAL, `1e999` as a Double) is refused rather than sent as infinity.
pub(super) fn finite(value: PropertyValue) -> Result<PropertyValue, String> {
    match value {
        PropertyValue::Real(v) if !v.is_finite() => {
            Err("the number is out of a REAL's range".into())
        }
        PropertyValue::Double(v) if !v.is_finite() => {
            Err("the number is out of a Double's range".into())
        }
        value => Ok(value),
    }
}

/// A number parsed by the untyped grammar, as `number`; anything else is
/// left as it is. A fraction for an integer, or a negative number for an
/// Unsigned, is refused.
pub(super) fn coerce(number: Number, value: PropertyValue) -> Result<PropertyValue, String> {
    use PropertyValue as V;
    let whole = |what: &str| format!("{what} takes a whole number");
    finite(match (number, value) {
        (Number::Real, V::Unsigned(n)) => V::Real(n as f32),
        (Number::Real, V::Signed(n)) => V::Real(n as f32),
        (Number::Double, V::Unsigned(n)) => V::Double(n as f64),
        (Number::Double, V::Signed(n)) => V::Double(f64::from(n)),
        (Number::Double, V::Real(n)) => V::Double(f64::from(n)),
        (Number::Signed, V::Unsigned(n)) => {
            V::Signed(i32::try_from(n).map_err(|_| format!("{n} is out of an INTEGER's range"))?)
        }
        (Number::Signed, V::Real(_)) => return Err(whole("this INTEGER property")),
        (Number::Unsigned, V::Signed(n)) => {
            return Err(format!("{n} is negative; this property is Unsigned"));
        }
        (Number::Unsigned, V::Real(_)) => return Err(whole("this Unsigned property")),
        (_, value) => value,
    })
}
