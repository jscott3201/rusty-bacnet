//! [`Value`]: a property as read, in both forms (design §5 of #2560): the
//! formatter's typed data and display text, and the octets the device sent.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{Datum, ValueFormat};

/// A property value as read: typed data for code, display text for people,
/// and the application-tagged octets the device sent, for anything else.
///
/// [`new`](Self::new) builds all three from the octets with one formatter;
/// the [`Device`](crate::Device) reads use one with every digit and every
/// list element ([`Floats::Exact`](super::Floats::Exact),
/// [`ValueFormat::whole_lists`]), and name units and states as the read
/// had them. A front end with its own display rules builds its own from
/// [`raw`](Self::raw).
#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    /// The value as typed data, the shape of the CLI's JSON ("Value JSON"
    /// in `docs/CLI.md`).
    pub data: Datum,
    /// The value as the formatter writes it: `21.5 °C` when the read had
    /// the object's Units, `2 (Heat)` when it had its State_Text.
    pub text: String,
    /// The application-tagged octets as the device sent them.
    pub raw: Vec<u8>,
    /// The text names a state by the object's own text, which is device
    /// text.
    named: bool,
}

impl Value {
    /// `raw`, a read of `property` at `index` (`None` for the whole
    /// property), as `format` writes it. A whole property's number takes
    /// `units` after it, as [`ValueFormat::with_units`] writes it; an
    /// array element's doesn't.
    pub fn new(
        format: &ValueFormat<'_>,
        property: PropertyIdentifier,
        index: Option<u32>,
        raw: Vec<u8>,
        units: Option<EngineeringUnits>,
    ) -> Self {
        let data = format.data_at(property, index, &raw);
        let text = match index {
            None => format.with_units(property, &raw, units),
            Some(_) => format.value_at(property, index, &raw),
        };
        let named = format.names_states(property, index);
        Self {
            data,
            text,
            raw,
            named,
        }
    }

    /// Whether [`text`](Self::text) may quote a character string a device
    /// wrote: the value holds one ([`Datum::may_hold_device_text`]), or a
    /// state is named by the object's own text. A front end shows such
    /// text as data, never as its own words.
    pub fn may_hold_device_text(&self) -> bool {
        self.named || self.data.may_hold_device_text()
    }

    /// The value as a number: a REAL or Double (a REAL as the shortest
    /// decimal that reads back as it, so 0.1 is 0.1, not
    /// 0.10000000149011612), an Unsigned or a Signed. `None` for anything
    /// else, a list included.
    pub fn as_f64(&self) -> Option<f64> {
        match &self.data {
            Datum::Real(value) => value.to_string().parse().ok(),
            Datum::Double(value) => Some(*value),
            Datum::NonFinite { float } => float.parse().ok(),
            Datum::Unsigned(value) => Some(*value as f64),
            Datum::Signed(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// The value as a truth value: a BOOLEAN, or a binary object's state
    /// (`active` is true, `inactive` false).
    pub fn as_bool(&self) -> Option<bool> {
        match &self.data {
            Datum::Boolean(value) => Some(*value),
            Datum::Enumerated {
                value,
                name: Some(name),
            } if matches!((*value, name.as_str()), (0, "inactive") | (1, "active")) => {
                Some(*value == 1)
            }
            _ => None,
        }
    }

    /// The value as an unsigned integer: an Unsigned, or a Signed that
    /// isn't negative. An enumeration's number is in
    /// [`Datum::Enumerated`] instead.
    pub fn as_u64(&self) -> Option<u64> {
        match &self.data {
            Datum::Unsigned(value) => Some(*value),
            Datum::Signed(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// The value as a signed integer: a Signed, or an Unsigned that fits.
    pub fn as_i64(&self) -> Option<i64> {
        match &self.data {
            Datum::Signed(value) => Some(*value),
            Datum::Unsigned(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// The value as text: a CharacterString, exactly as the device sent
    /// it. Device text: clean it before showing it
    /// ([`clean_name`](super::clean_name)).
    pub fn as_str(&self) -> Option<&str> {
        match &self.data {
            Datum::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Whether the value is NULL.
    pub fn is_null(&self) -> bool {
        self.data == Datum::Null
    }

    /// The octets as application values: one value as itself, and none or
    /// several as a [`PropertyValue::List`]. A constructed value's
    /// context-tagged parts come back as
    /// [`PropertyValue::ApplicationData`].
    ///
    /// # Errors
    ///
    /// The decoder's error for octets that aren't application values.
    pub fn decode(&self) -> Result<PropertyValue, Error> {
        let mut offset = 0;
        let mut values = Vec::new();
        while offset < self.raw.len() {
            let (value, next) = decode_application_value(&self.raw, offset)?;
            values.push(value);
            offset = next;
        }
        Ok(match <[PropertyValue; 1]>::try_from(values) {
            Ok([one]) => one,
            Err(values) => PropertyValue::List(values),
        })
    }
}

impl std::fmt::Display for Value {
    /// The display text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
