//! The change test of COV reporting (Clause 13.1), shared by the server's
//! COV notifications and a Trend Log that logs by COV (#1480).
//!
//! A numeric value reports when it has moved by at least the increment in
//! use; with no increment, or one of zero or less, any change reports. Every
//! other value reports when it differs at all. Which increment is in use is
//! [`effective_increment`]'s rule: one the client supplied wins, otherwise
//! only a numeric Present_Value takes the object's COV_Increment (Table 13-1
//! for the objects it lists, Table 13-1a for any other property).

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;

use crate::traits::BACnetObject;

/// Whether `value`, read from `property` (at `index`) of `object`, is a
/// numeric quantity the increment applies to: a REAL, Double, Unsigned or
/// INTEGER that is not a whole array or an array's size.
pub fn is_numeric(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: &PropertyValue,
) -> bool {
    let array = object.is_array_property(property);
    !(array && (index.is_none() || index == Some(0)))
        && matches!(
            value,
            PropertyValue::Real(_)
                | PropertyValue::Double(_)
                | PropertyValue::Signed(_)
                | PropertyValue::Unsigned(_)
        )
}

/// The increment in use: an `explicit` one (a SubscribeCOVProperty
/// increment, or a Trend Log's Client_COV_Increment) wins; otherwise only a
/// numeric Present_Value inherits the object's current COV_Increment.
///
/// The explicit increment is a REAL on the wire; the object's own may be
/// wider (a Large Analog Value's Double), so the result is an `f64`.
pub fn effective_increment(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    numeric: bool,
    explicit: Option<f32>,
) -> Option<f64> {
    let explicit = explicit.map(f64::from);
    if numeric && property == PropertyIdentifier::PRESENT_VALUE {
        explicit.or_else(|| object.cov_increment())
    } else {
        explicit
    }
}

/// Whether `current` is a change from `previous` worth reporting. A
/// `numeric` pair of the same datatype is held to `increment`; a
/// non-finite REAL or Double reports when its bits differ. Anything else
/// reports when [`same_value`] says the two differ.
pub fn reports_change(
    current: &PropertyValue,
    previous: &PropertyValue,
    increment: Option<f64>,
    numeric: bool,
) -> bool {
    if !numeric {
        return !same_value(current, previous);
    }
    use PropertyValue::*;
    match (current, previous) {
        (Unsigned(a), Unsigned(b)) => integer_delta(a.abs_diff(*b), increment),
        (Signed(a), Signed(b)) => integer_delta(u64::from(a.abs_diff(*b)), increment),
        (Real(a), Real(b)) => {
            if !a.is_finite() || !b.is_finite() {
                a.to_bits() != b.to_bits()
            } else {
                float_delta((f64::from(*a) - f64::from(*b)).abs(), increment)
            }
        }
        (Double(a), Double(b)) => {
            if !a.is_finite() || !b.is_finite() {
                a.to_bits() != b.to_bits()
            } else {
                float_delta((a - b).abs(), increment)
            }
        }
        _ => !same_value(current, previous),
    }
}

/// Value equality for change detection: REALs and Doubles compare by their
/// bits (so NaN equals itself and 0.0 differs from -0.0), lists element by
/// element, everything else as `==` does.
pub fn same_value(a: &PropertyValue, b: &PropertyValue) -> bool {
    use PropertyValue::*;
    match (a, b) {
        (Real(a), Real(b)) => a.to_bits() == b.to_bits(),
        (Double(a), Double(b)) => a.to_bits() == b.to_bits(),
        (List(a), List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b)),
        _ => a == b,
    }
}

fn integer_delta(delta: u64, increment: Option<f64>) -> bool {
    match increment {
        None => delta != 0,
        Some(i) if i <= 0.0 => delta != 0,
        Some(i) if i.is_nan() || i >= 18_446_744_073_709_551_616.0 => false,
        Some(i) => delta >= i.ceil() as u64,
    }
}

fn float_delta(delta: f64, increment: Option<f64>) -> bool {
    match increment {
        None => delta != 0.0,
        Some(i) if i <= 0.0 => delta != 0.0,
        Some(i) if !i.is_finite() => false,
        Some(i) => delta >= i,
    }
}
