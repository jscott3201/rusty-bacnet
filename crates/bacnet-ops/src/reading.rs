//! What a [`Device`](crate::Device) read names and returns (#2560): a
//! [`PropertyRef`], from text or a tuple ([`IntoProperty`]), and a
//! [`Reading`] a property, gathered into [`Readings`] by the bulk reads.

use std::str::FromStr;

use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::client::ReadVia;
use crate::error::OpsError;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat, units};
use crate::parse::{parse_object_specifier, parse_property};

/// One property of one object, or one element of an array property.
///
/// ```
/// use bacnet_ops::PropertyRef;
/// use bacnet_types::enums::{ObjectType, PropertyIdentifier};
///
/// let slot: PropertyRef = "ao:2 priority-array[8]".parse().unwrap();
/// assert_eq!(slot.object.object_type(), ObjectType::ANALOG_OUTPUT);
/// assert_eq!(slot.property, PropertyIdentifier::PRIORITY_ARRAY);
/// assert_eq!(slot.index, Some(8));
/// // An object alone is its Present_Value.
/// let pv: PropertyRef = "ai:1".parse().unwrap();
/// assert_eq!(pv, PropertyRef::new(pv.object, PropertyIdentifier::PRESENT_VALUE));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropertyRef {
    /// The object.
    pub object: ObjectIdentifier,
    /// The property.
    pub property: PropertyIdentifier,
    /// The array element, `Some(0)` for an array's size; `None` for the
    /// whole property.
    pub index: Option<u32>,
}

impl PropertyRef {
    /// The whole of `property` of `object`.
    pub fn new(object: ObjectIdentifier, property: PropertyIdentifier) -> Self {
        Self {
            object,
            property,
            index: None,
        }
    }

    /// The same property's element `index` (0 for an array's size).
    pub fn at(self, index: u32) -> Self {
        Self {
            index: Some(index),
            ..self
        }
    }
}

impl FromStr for PropertyRef {
    type Err = OpsError;

    /// An object, a space and a property, each in the grammar of
    /// [`parse_object_specifier`] and [`parse_property`]: `ai:1 pv`,
    /// `analog-output:2 priority-array[8]`. An object alone names its
    /// Present_Value.
    fn from_str(text: &str) -> Result<Self, OpsError> {
        let mut parts = text.split_whitespace();
        let object_text = parts.next().unwrap_or_default();
        let (object_type, instance) = parse_object_specifier(object_text)
            .map_err(|reason| OpsError::parse("object", text, reason))?;
        let object = ObjectIdentifier::new(object_type, instance)
            .map_err(|error| OpsError::parse("object", text, error.to_string()))?;
        let (property, index) = match (parts.next(), parts.next()) {
            (None, _) => (PropertyIdentifier::PRESENT_VALUE, None),
            (Some(property), None) => parse_property(property)
                .map_err(|reason| OpsError::parse("property", text, reason))?,
            (Some(_), Some(_)) => {
                return Err(OpsError::parse(
                    "property",
                    text,
                    format!("'{text}' names more than an object and a property"),
                ));
            }
        };
        Ok(Self {
            object,
            property,
            index,
        })
    }
}

/// What names a property to read: a [`PropertyRef`], its text (`"ai:1
/// pv"`, see [`PropertyRef::from_str`]), or an `(object, property)` or
/// `(object, property, index)` tuple.
pub trait IntoProperty {
    /// The property named.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] for text that doesn't name one.
    fn into_property(self) -> Result<PropertyRef, OpsError>;
}

impl IntoProperty for PropertyRef {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        Ok(self)
    }
}

impl IntoProperty for &str {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        self.parse()
    }
}

impl IntoProperty for &String {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        self.parse()
    }
}

impl IntoProperty for String {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        self.parse()
    }
}

impl IntoProperty for (ObjectIdentifier, PropertyIdentifier) {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        Ok(PropertyRef::new(self.0, self.1))
    }
}

impl IntoProperty for (ObjectIdentifier, PropertyIdentifier, u32) {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        Ok(PropertyRef::new(self.0, self.1).at(self.2))
    }
}

/// One property as read: its value, or why the device gave none.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The object read.
    pub object: ObjectIdentifier,
    /// The property read.
    pub property: PropertyIdentifier,
    /// The array element read, `None` for the whole property.
    pub index: Option<u32>,
    /// The value, or the device's refusal of this property (an unknown
    /// property, an unknown object, ...), which leaves the rest of the
    /// read standing.
    pub value: Result<Value, PropertyError>,
}

/// What a bulk read read: a [`Reading`] each property, in the order asked
/// for, and why it stopped early, if it did. It never fails as a whole.
#[derive(Debug, Default)]
pub struct Readings {
    /// The properties read, each its value or its own error.
    pub readings: Vec<Reading>,
    /// How the values were read: [`ReadVia::Mixed`] when some went by
    /// ReadPropertyMultiple and some by ReadProperty; `None` when nothing
    /// was read.
    pub via: Option<ReadVia>,
    /// Why the read ended before every property was read: the device
    /// stopped answering ([`OpsError::unreachable`]), it couldn't be
    /// placed, or a property's text didn't parse (then nothing was sent).
    /// `readings` holds what was read before it.
    pub stopped: Option<OpsError>,
}

impl Readings {
    /// Nothing read, for `why`.
    pub(crate) fn nothing_read(why: OpsError) -> Self {
        Self {
            readings: Vec::new(),
            via: None,
            stopped: Some(why),
        }
    }

    /// The first reading of the whole of `property` of `object`.
    pub fn get(&self, object: ObjectIdentifier, property: PropertyIdentifier) -> Option<&Reading> {
        self.readings
            .iter()
            .find(|r| r.object == object && r.property == property && r.index.is_none())
    }
}

/// A raw row of `object`, read at `index`.
pub(crate) type RawReading = (ObjectIdentifier, Option<u32>, RawRow);

/// Each raw row as a [`Reading`], in order: its value as the
/// [`Device`](crate::Device) reads write it, every digit and every
/// element, with the object's Units and state texts when the same read
/// holds them (the whole property, not an element).
pub(crate) fn readings(raw: Vec<RawReading>) -> Vec<Reading> {
    let mut names: Vec<(ObjectIdentifier, Option<EngineeringUnits>, StateTexts)> = Vec::new();
    for (object, _, _) in &raw {
        if names.iter().any(|(named, ..)| named == object) {
            continue;
        }
        let wanted = StateTexts::properties(object.object_type());
        let rows: Vec<RawRow> = raw
            .iter()
            .filter(|(o, index, (p, _))| {
                o == object
                    && index.is_none()
                    && (*p == PropertyIdentifier::UNITS || wanted.contains(p))
            })
            .map(|(_, _, row)| row.clone())
            .collect();
        names.push((
            *object,
            units(&rows),
            StateTexts::from_rows(&rows).cleaned(),
        ));
    }
    raw.into_iter()
        .map(|(object, index, (property, value))| {
            let (_, units, texts) = names
                .iter()
                .find(|(named, ..)| *named == object)
                .expect("every object is named above");
            let format = ValueFormat::new(object.object_type(), texts, Floats::Exact).whole_lists();
            Reading {
                object,
                property,
                index,
                value: value.map(|raw| Value::new(&format, property, index, raw, *units)),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "reading_tests.rs"]
mod tests;
