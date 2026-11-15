//! What a [`Device`](crate::Device) read names and returns (#2560): a
//! [`PropertyRef`], from text or a tuple ([`IntoProperty`]), and a
//! [`Reading`] a property, gathered into [`Readings`] by a bulk read and
//! into an [`ObjectRead`] by an object's read.

use std::collections::HashMap;
use std::str::FromStr;

use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::client::ReadVia;
use crate::error::OpsError;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat, units};
use crate::label::kebab;
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
        let object = object(parts.next().unwrap_or_default(), text)?;
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

/// The object `word` names, in the grammar of [`parse_object_specifier`];
/// an error quotes `text`, the whole of what was given.
fn object(word: &str, text: &str) -> Result<ObjectIdentifier, OpsError> {
    let (object_type, instance) =
        parse_object_specifier(word).map_err(|reason| OpsError::parse("object", text, reason))?;
    ObjectIdentifier::new(object_type, instance)
        .map_err(|error| OpsError::parse("object", text, error.to_string()))
}

/// What names an object: an [`ObjectIdentifier`], or its text in the
/// grammar of [`parse_object_specifier`] (`"av:1"`, `"analog-value,1"`).
pub trait IntoObject {
    /// The object named.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] (`what` is `"object"`) for text that doesn't
    /// name one.
    fn into_object(self) -> Result<ObjectIdentifier, OpsError>;
}

impl IntoObject for ObjectIdentifier {
    fn into_object(self) -> Result<ObjectIdentifier, OpsError> {
        Ok(self)
    }
}

impl IntoObject for &str {
    fn into_object(self) -> Result<ObjectIdentifier, OpsError> {
        object(self.trim(), self)
    }
}

impl IntoObject for &String {
    fn into_object(self) -> Result<ObjectIdentifier, OpsError> {
        self.as_str().into_object()
    }
}

impl IntoObject for String {
    fn into_object(self) -> Result<ObjectIdentifier, OpsError> {
        self.as_str().into_object()
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

impl IntoProperty for &PropertyRef {
    fn into_property(self) -> Result<PropertyRef, OpsError> {
        Ok(*self)
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

/// What [`Device::read_many`](crate::Device::read_many) read: a
/// [`Reading`] each property, in the order asked for, and whether the
/// device stopped answering part way. A property the device refused is an
/// error in its own row, not a stop.
///
/// Not `Clone`: `stopped` holds the low-level client's error, which can
/// hold the operating system's.
#[derive(Debug, Default)]
pub struct Readings {
    /// The properties read, each its value or its own error. A row the
    /// device's answer left out reads as [`PropertyError::NoValue`].
    pub readings: Vec<Reading>,
    /// How the values were read: [`ReadVia::Mixed`] when some went by
    /// ReadPropertyMultiple and some by ReadProperty; `None` when nothing
    /// was read.
    pub via: Option<ReadVia>,
    /// The read ended after it began, so the properties after `readings`
    /// weren't read: the device stopped answering
    /// ([`OpsError::unreachable`]), or the client or the link failed a
    /// later request itself ([`OpsError::Client`]: a route lost, a link
    /// refusing the send). `readings` keeps what was read before it,
    /// possibly nothing, an object's properties read before it included.
    pub stopped: Option<OpsError>,
}

impl Readings {
    /// The first reading of the whole of `property` of `object`, when the
    /// read asked for it more than once.
    pub fn get(&self, object: ObjectIdentifier, property: PropertyIdentifier) -> Option<&Reading> {
        self.readings
            .iter()
            .find(|r| r.object == object && r.property == property && r.index.is_none())
    }
}

/// What [`Device::read_object`](crate::Device::read_object) read of one
/// object: a [`Reading`] each property, a property the device refused an
/// error in its own row.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRead {
    /// The object read.
    pub object: ObjectIdentifier,
    /// Its properties as read: those asked for, or for ALL those the
    /// device gave (or its Property_List names, by ReadProperty).
    pub readings: Vec<Reading>,
    /// Whether the read went by ReadPropertyMultiple or ReadProperty.
    pub via: ReadVia,
}

impl ObjectRead {
    /// The reading of `property`, the first when the device gave it twice.
    pub fn get(&self, property: PropertyIdentifier) -> Option<&Reading> {
        self.readings
            .iter()
            .find(|r| r.property == property && r.index.is_none())
    }
}

/// The properties no one read names: ALL, REQUIRED and OPTIONAL stand for
/// many, which only an object's whole read takes.
pub(crate) fn one_property(wanted: PropertyRef) -> Result<PropertyRef, OpsError> {
    let many = [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ];
    if !many.contains(&wanted.property) {
        return Ok(wanted);
    }
    let name = kebab(wanted.property);
    Err(OpsError::Parse {
        what: "property",
        input: name.clone(),
        reason: format!(
            "'{name}' names many properties; read every property of an object with read_object"
        ),
    })
}

/// A raw row of `object`, read at `index`.
pub(crate) type RawReading = (ObjectIdentifier, Option<u32>, RawRow);

/// Each raw row as a [`Reading`], in order: its value as the
/// [`Device`](crate::Device) reads write it, every digit and every
/// element, with the object's Units and state texts when the same read
/// holds them (the whole property, not an element).
pub(crate) fn readings(raw: Vec<RawReading>) -> Vec<Reading> {
    // One pass for the rows that name values, one for the values.
    let mut naming: HashMap<ObjectIdentifier, Vec<RawRow>> = HashMap::new();
    for (object, index, row @ (property, _)) in &raw {
        let names = *property == PropertyIdentifier::UNITS
            || StateTexts::properties(object.object_type()).contains(property);
        if index.is_none() && names {
            naming.entry(*object).or_default().push(row.clone());
        }
    }
    let names: HashMap<ObjectIdentifier, (Option<EngineeringUnits>, StateTexts)> = naming
        .into_iter()
        .map(|(object, rows)| {
            (
                object,
                (units(&rows), StateTexts::from_rows(&rows).cleaned()),
            )
        })
        .collect();
    let unnamed = (None, StateTexts::default());
    raw.into_iter()
        .map(|(object, index, (property, value))| {
            let (units, texts) = names.get(&object).unwrap_or(&unnamed);
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
