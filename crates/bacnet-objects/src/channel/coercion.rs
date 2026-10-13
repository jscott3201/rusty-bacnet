//! Coercing a Channel's value to each member's datatype (Clause 12.53.5.1,
//! Table 12-63 and Coercion Rules 1 to 6 in Clauses 12.53.5.2 to 12.53.5.7).
//!
//! The table is keyed by the datatype written to Present_Value and the
//! datatype of the member property. A pair it marks as needing no coercion
//! passes the value on; a pair it marks as an invalid datatype, or a number
//! outside a rule's limits, is a coercion failure: the member isn't written
//! and the distribution ends FAILED (Clause 12.53.5.8).
//!
//! Choices this module makes where the clause leaves room:
//!
//! - A member's datatype is that of the value its property holds now (see
//!   [`MemberDatatype::of`]); a NULL or constructed value counts as unknown,
//!   which takes any value but a constructed one, as the table's first
//!   column does. The exceptions are the properties whose datatype is one of
//!   the constructed alternatives: Lighting_Command, Color_Command, and the
//!   xy colours of a Color object (Addendum 135-2020ca's rows and columns of
//!   the table, #1474). Each takes only its own alternative.
//! - Where the table passes a value on between two different datatypes
//!   (Unsigned and ENUMERATED, Unsigned and BACnetObjectIdentifier), the
//!   number is kept and only the datatype changes. A number the target can't
//!   hold is a failure.
//! - Rule 3 bounds the Unsigned or ENUMERATED value itself at 2147483647,
//!   whichever numeric target it goes to (INTEGER, REAL or Double); a larger
//!   one fails. Unsigned to ENUMERATED and back is the pass-through above,
//!   not Rule 3.
//! - A REAL or Double going to an integer type keeps its integer part
//!   (truncation toward zero) once the rule's range check passes.
//! - Rules 5 and 6 print the INTEGER range as -2147483000 to 214783000. The
//!   upper bound is read as 2147483000: one digit is missing, it would sit
//!   below the Unsigned bound of the same rules, and it then mirrors the
//!   lower bound.
//! - Rule 6's REAL bound of about 3.4 x 10^38 is taken as the largest finite
//!   REAL, `f32::MAX` (3.4028235 x 10^38), so every finite REAL is in range
//!   and anything larger fails.
//! - NaN and the infinities are inside no stated range, so they fail every
//!   conversion that has one: REAL or Double to an integer type, and Double
//!   to REAL. A REAL going to a Double has no range to check and passes as
//!   it is, as does a value going to its own datatype. Rule 1 turns NaN into
//!   TRUE, since it isn't zero.
//! - The seven-significant-digit REAL limit in Rules 3 and 4 is read the
//!   same way in both: the value is rounded to the nearest REAL, which is
//!   what that precision means, and the rounding never fails the write. Only
//!   the range bounds fail it.

use bacnet_encoding::constructed::{
    constructed_channel_value, decode_xy_color, ConstructedChannelValue,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// The datatype of a Channel member's property, as Table 12-63's columns
/// name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberDatatype {
    /// Not known: any primitive value passes on unchanged.
    Unknown,
    /// BOOLEAN.
    Boolean,
    /// Unsigned.
    Unsigned,
    /// INTEGER.
    Integer,
    /// REAL.
    Real,
    /// Double.
    Double,
    /// OCTET STRING.
    OctetString,
    /// CharacterString.
    CharacterString,
    /// BIT STRING.
    BitString,
    /// ENUMERATED.
    Enumerated,
    /// Date.
    Date,
    /// Time.
    Time,
    /// BACnetObjectIdentifier.
    ObjectIdentifier,
    /// BACnetLightingCommand.
    LightingCommand,
    /// BACnetColorCommand (Addendum 135-2020ca).
    ColorCommand,
    /// BACnetxyColor (Addendum 135-2020ca).
    XyColor,
}

impl MemberDatatype {
    /// The datatype of member property `property` of an object of type
    /// `object_type`, whose value is `current` (`None` when it can't be
    /// read).
    ///
    /// A few properties have a constructed datatype whatever form the object
    /// keeps them in: Lighting_Command is a BACnetLightingCommand,
    /// Color_Command a BACnetColorCommand, and Default_Color and a Color
    /// object's Present_Value and Tracking_Value a BACnetxyColor. Any other
    /// property takes the datatype of the primitive it holds; NULL,
    /// constructed values and unreadable properties are
    /// [`MemberDatatype::Unknown`].
    pub fn of(
        object_type: ObjectType,
        property: PropertyIdentifier,
        current: Option<&PropertyValue>,
    ) -> Self {
        use PropertyIdentifier as P;
        match property {
            P::LIGHTING_COMMAND => return Self::LightingCommand,
            P::COLOR_COMMAND => return Self::ColorCommand,
            P::DEFAULT_COLOR => return Self::XyColor,
            P::PRESENT_VALUE | P::TRACKING_VALUE if object_type == ObjectType::COLOR => {
                return Self::XyColor
            }
            _ => {}
        }
        match current {
            Some(PropertyValue::Boolean(_)) => Self::Boolean,
            Some(PropertyValue::Unsigned(_)) => Self::Unsigned,
            Some(PropertyValue::Signed(_)) => Self::Integer,
            Some(PropertyValue::Real(_)) => Self::Real,
            Some(PropertyValue::Double(_)) => Self::Double,
            Some(PropertyValue::OctetString(_)) => Self::OctetString,
            Some(PropertyValue::CharacterString(_)) => Self::CharacterString,
            Some(PropertyValue::BitString { .. }) => Self::BitString,
            Some(PropertyValue::Enumerated(_)) => Self::Enumerated,
            Some(PropertyValue::Date(_)) => Self::Date,
            Some(PropertyValue::Time(_)) => Self::Time,
            Some(PropertyValue::ObjectIdentifier(_)) => Self::ObjectIdentifier,
            _ => Self::Unknown,
        }
    }
}

/// A channel value that can't go to a member: the table names an invalid
/// datatype, or a number is outside its rule's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoercionFailure;

/// A number a channel value carries, by the coercion rule its datatype
/// follows.
#[derive(Clone, Copy)]
enum Number {
    /// BOOLEAN, as 0 or 1 (Rule 2).
    Boolean(bool),
    /// Unsigned or ENUMERATED (Rule 3).
    Unsigned(u64),
    /// INTEGER (Rule 4).
    Integer(i64),
    /// REAL (Rule 5).
    Real(f32),
    /// Double (Rule 6).
    Double(f64),
}

/// Rules 5 and 6 bound a REAL or Double going to an Unsigned or ENUMERATED
/// at this value, and to an INTEGER at it either way.
const FLOAT_TO_INTEGER_LIMIT: f64 = 2_147_483_000.0;

impl Number {
    fn of(value: &PropertyValue) -> Option<Self> {
        Some(match *value {
            PropertyValue::Boolean(value) => Self::Boolean(value),
            PropertyValue::Unsigned(value) => Self::Unsigned(value),
            PropertyValue::Enumerated(value) => Self::Unsigned(value.into()),
            PropertyValue::Signed(value) => Self::Integer(value.into()),
            PropertyValue::Real(value) => Self::Real(value),
            PropertyValue::Double(value) => Self::Double(value),
            _ => return None,
        })
    }

    /// Rule 1: zero is FALSE and anything else TRUE.
    fn to_boolean(self) -> bool {
        match self {
            Self::Boolean(value) => value,
            Self::Unsigned(value) => value != 0,
            Self::Integer(value) => value != 0,
            Self::Real(value) => value != 0.0,
            Self::Double(value) => value != 0.0,
        }
    }

    /// The value as an Unsigned or ENUMERATED, if the rule allows it.
    fn to_unsigned(self) -> Option<u64> {
        match self {
            Self::Boolean(value) => Some(value.into()),
            Self::Unsigned(value) => Some(value),
            // Rule 4's upper bound, 2147483647, is the top of the INTEGER
            // range, so only the sign can fail.
            Self::Integer(value) => u64::try_from(value).ok(),
            Self::Real(value) => float_to_unsigned(value.into()),
            Self::Double(value) => float_to_unsigned(value),
        }
    }

    /// The value as an INTEGER, if the rule allows it.
    fn to_integer(self) -> Option<i32> {
        match self {
            Self::Boolean(value) => Some(value.into()),
            Self::Unsigned(value) => rule_3_bound(value).map(|value| value as i32),
            Self::Integer(value) => i32::try_from(value).ok(),
            Self::Real(value) => float_to_integer(value.into()),
            Self::Double(value) => float_to_integer(value),
        }
    }

    /// The value as a REAL, if the rule allows it.
    fn to_real(self) -> Option<f32> {
        match self {
            Self::Boolean(value) => Some(u8::from(value).into()),
            // Rules 3 and 4: rounding to a REAL's precision isn't a failure,
            // but an Unsigned past 2147483647 is (Rule 3).
            Self::Unsigned(value) => rule_3_bound(value).map(|value| value as f32),
            Self::Integer(value) => Some(value as f32),
            Self::Real(value) => Some(value),
            // Rule 6: a magnitude past the REAL range fails.
            // Rule 6: the REAL bound is `f32::MAX`; NaN is inside no bound.
            Self::Double(value) => (value.abs() <= f64::from(f32::MAX)).then_some(value as f32),
        }
    }

    /// The value as a Double, if the rule allows it.
    fn to_double(self) -> Option<f64> {
        match self {
            Self::Boolean(value) => Some(u8::from(value).into()),
            Self::Unsigned(value) => rule_3_bound(value).map(f64::from),
            Self::Integer(value) => Some(value as f64),
            Self::Real(value) => Some(value.into()),
            Self::Double(value) => Some(value),
        }
    }
}

/// Rule 3 bounds an Unsigned or ENUMERATED going to INTEGER, REAL or Double
/// at 2147483647, the top of the INTEGER range.
fn rule_3_bound(value: u64) -> Option<u32> {
    i32::try_from(value).ok().map(|value| value as u32)
}

fn float_to_unsigned(value: f64) -> Option<u64> {
    (0.0..=FLOAT_TO_INTEGER_LIMIT)
        .contains(&value)
        .then(|| value.trunc() as u64)
}

fn float_to_integer(value: f64) -> Option<i32> {
    (-FLOAT_TO_INTEGER_LIMIT..=FLOAT_TO_INTEGER_LIMIT)
        .contains(&value)
        .then(|| value.trunc() as i32)
}

/// The value a member of datatype `target` gets when a Channel holding
/// `value` distributes it (Table 12-63).
///
/// A constructed channel value, held as its framed octets, reaches only a
/// member of its own datatype, the way a WriteProperty of that member's
/// property would carry it: a lighting or colour command as the SEQUENCE
/// inside its tag, and an xy colour as its two REALs.
pub fn coerce_channel_value(
    value: &PropertyValue,
    target: MemberDatatype,
) -> Result<PropertyValue, CoercionFailure> {
    use ConstructedChannelValue as C;
    use MemberDatatype as D;

    if let PropertyValue::ApplicationData(octets) = value {
        // A constructed value goes to a member of its own datatype and
        // nowhere else, not even an unknown one.
        let inner = || octets[1..octets.len() - 1].to_vec();
        return match (constructed_channel_value(octets), target) {
            (Some(C::LightingCommand), D::LightingCommand)
            | (Some(C::ColorCommand), D::ColorCommand) => {
                Ok(PropertyValue::ApplicationData(inner()))
            }
            (Some(C::XyColor), D::XyColor) => {
                let (color, _) = decode_xy_color(octets, 1).map_err(|_| CoercionFailure)?;
                Ok(PropertyValue::List(vec![
                    PropertyValue::Real(color.x),
                    PropertyValue::Real(color.y),
                ]))
            }
            _ => Err(CoercionFailure),
        };
    }
    match target {
        D::LightingCommand | D::ColorCommand | D::XyColor => return Err(CoercionFailure),
        D::Unknown => return Ok(value.clone()),
        _ => {}
    }
    if *value == PropertyValue::Null {
        return Ok(PropertyValue::Null);
    }
    if let Some(number) = Number::of(value) {
        let coerced = match target {
            D::Boolean => Some(PropertyValue::Boolean(number.to_boolean())),
            D::Unsigned => number.to_unsigned().map(PropertyValue::Unsigned),
            D::Integer => number.to_integer().map(PropertyValue::Signed),
            D::Real => number.to_real().map(PropertyValue::Real),
            D::Double => number.to_double().map(PropertyValue::Double),
            D::Enumerated => number
                .to_unsigned()
                .and_then(|raw| u32::try_from(raw).ok())
                .map(PropertyValue::Enumerated),
            // The table passes an Unsigned to an object identifier as the
            // same 32-bit number.
            D::ObjectIdentifier => match *value {
                PropertyValue::Unsigned(raw) => u32::try_from(raw)
                    .ok()
                    .and_then(|raw| ObjectIdentifier::decode(&raw.to_be_bytes()).ok())
                    .map(PropertyValue::ObjectIdentifier),
                _ => None,
            },
            _ => None,
        };
        return coerced.ok_or(CoercionFailure);
    }
    let passes = match (value, target) {
        (PropertyValue::OctetString(_), D::OctetString)
        | (PropertyValue::CharacterString(_), D::CharacterString)
        | (PropertyValue::BitString { .. }, D::BitString)
        | (PropertyValue::Date(_), D::Date)
        | (PropertyValue::Time(_), D::Time)
        | (PropertyValue::ObjectIdentifier(_), D::ObjectIdentifier) => true,
        (PropertyValue::ObjectIdentifier(oid), D::Unsigned) => {
            return Ok(PropertyValue::Unsigned(
                u32::from_be_bytes(oid.encode()).into(),
            ));
        }
        _ => false,
    };
    if passes {
        Ok(value.clone())
    } else {
        Err(CoercionFailure)
    }
}
