//! The one value formatter the TUI, the one-shot CLI and the MCP server
//! share: property values as read, turned into the text the browser, the
//! watch list, the write previews and the CLI's tables show, and into the
//! typed [`Datum`] the CLI's JSON carries.
//!
//! It knows the object type, so a constructed value (timestamps, schedule
//! days, references) is split with the element table from
//! `bacnet_encoding::constructed::property_element` and shown decoded, and
//! a binary or multi-state value is named by its state, with the object's
//! own state text when that was read.
//!
//! Style, the same on every screen (decision D4 of #1887): enumerations,
//! bit names and units in kebab case, objects as `analog-input 1`. Floats
//! come in two forms (decision D2): [`Floats::Display`] for the browser and
//! the watch list, at most six significant digits, and [`Floats::Exact`] for
//! the write previews, the ledger, the CSV log and the one-shot CLI, which
//! keep every digit. The compact places cut a long list short; the CLI
//! shows it whole ([`ValueFormat::whole_lists`]).

pub mod alarms;
mod clean;
mod commands;
mod constructed;
mod data;
mod event_parameter;
mod fields;
pub mod log_records;
mod logged;
pub mod notification;
mod primitive;
mod recipients;
mod tagged;
mod units;
mod wire;

pub use clean::{MAX_NAME_CHARS, clean_name, clean_text};
pub(crate) use constructed::timestamp_text;
pub use data::{Datum, Fields, PLAIN_ELEMENTS};
pub use logged::{LogColumns, LoggedProperty, MAX_STATE_TEXT_OBJECTS};
pub use wire::format_property_value;

use bacnet_encoding::constructed::{ConstructedElement, ElementShape, property_element};
use bacnet_encoding::primitives::{decode_application_value, decode_priority_date_time};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{Date, PropertyValue, Time};

use units::unit_symbol;

/// A property as read: its raw application-tagged value, or why the device
/// gave none.
pub type RawRow = (PropertyIdentifier, Result<Vec<u8>, RowError>);

/// Why one property of a read has no value. Structured, so a front end can
/// report the device's error class and code as data, not only as text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowError {
    /// The device answered this property with an error, a reject or an
    /// abort.
    Remote(RemoteError),
    /// The device's answer for this property held neither a value nor an
    /// error.
    NoValue,
    /// Any other failure of this property's read, as text.
    Other(String),
}

impl RowError {
    /// The device's error class and code, when it answered with an error.
    pub fn class_code(&self) -> Option<(ErrorClass, ErrorCode)> {
        match self {
            Self::Remote(RemoteError::Error { class, code }) => Some((*class, *code)),
            _ => None,
        }
    }
}

impl From<Error> for RowError {
    /// The device's error, reject or abort as [`RowError::Remote`], with a
    /// structured Error PDU reduced to its class and code; anything else
    /// as text.
    fn from(error: Error) -> Self {
        match error.remote() {
            Some(remote) => Self::Remote(remote),
            None => Self::Other(error.to_string()),
        }
    }
}

impl std::error::Error for RowError {}

impl std::fmt::Display for RowError {
    /// An error as `CLASS:CODE` (`PROPERTY:UNKNOWN_PROPERTY`), the form
    /// the TUI shows; a reject or abort, or any other failure, as its own
    /// text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote(RemoteError::Error { class, code }) => write!(f, "{class}:{code}"),
            Self::Remote(remote) => write!(f, "{remote}"),
            Self::NoValue => f.write_str("no value"),
            Self::Other(text) => f.write_str(text),
        }
    }
}

/// Values shown before the rest of a list is summarised.
const MAX_LIST_SHOWN: usize = 8;

/// Every application-tagged value in `data`, or `None` if any part of it
/// doesn't decode. A context tag decodes as `ApplicationData`.
pub fn decode_all(data: &[u8]) -> Option<Vec<PropertyValue>> {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        let (value, next) = decode_application_value(data, offset).ok()?;
        values.push(value);
        offset = next;
    }
    Some(values)
}

/// How floats are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floats {
    /// At most six significant digits, trailing zeros trimmed.
    Display,
    /// Every digit: the shortest text that reads back as the same value.
    Exact,
}

/// The object's own names for its states, when they were read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateTexts {
    /// A binary object's Active_Text.
    pub active: Option<String>,
    /// A binary object's Inactive_Text.
    pub inactive: Option<String>,
    /// A multi-state object's State_Text, state 1 first.
    pub states: Vec<String>,
}

impl StateTexts {
    /// The properties that hold an object type's state texts, to read with
    /// the value; none for a type without them.
    pub fn properties(object_type: ObjectType) -> &'static [PropertyIdentifier] {
        if is_binary(object_type) {
            &[
                PropertyIdentifier::ACTIVE_TEXT,
                PropertyIdentifier::INACTIVE_TEXT,
            ]
        } else if is_multistate(object_type) {
            &[PropertyIdentifier::STATE_TEXT]
        } else {
            &[]
        }
    }

    /// Whether an object of `object_type`'s state texts would name the
    /// value of `property` (at `index`, for an array): a binary or
    /// multi-state object's state, in its Present_Value, Priority_Array
    /// slots, Relinquish_Default and the like. An array's size, index 0,
    /// never is. A front end reads the texts only when this holds.
    pub fn name(object_type: ObjectType, property: PropertyIdentifier, index: Option<u32>) -> bool {
        (is_binary(object_type) || is_multistate(object_type))
            && holds_state(property)
            && index != Some(0)
    }

    /// The same texts cleaned for display ([`clean_name`]): device text
    /// with its control characters turned to spaces and invisible format
    /// characters dropped.
    pub fn cleaned(&self) -> Self {
        Self {
            active: self.active.as_deref().map(clean_name),
            inactive: self.inactive.as_deref().map(clean_name),
            states: self.states.iter().map(|text| clean_name(text)).collect(),
        }
    }

    /// The texts among properties as read. A State_Text entry that isn't
    /// a string reads as an empty text, which names nothing, so later
    /// entries keep their state numbers.
    pub fn from_rows(rows: &[RawRow]) -> Self {
        let values = |property| {
            rows.iter()
                .find(|(p, _)| *p == property)
                .and_then(|(_, value)| value.as_deref().ok())
                .and_then(decode_all)
                .unwrap_or_default()
                .into_iter()
                // A value that isn't a string holds its place as no text,
                // so the states after it keep their positions.
                .map(|value| match value {
                    PropertyValue::CharacterString(text) => text,
                    _ => String::new(),
                })
        };
        let text = |property| values(property).next().filter(|text| !text.is_empty());
        Self {
            active: text(PropertyIdentifier::ACTIVE_TEXT),
            inactive: text(PropertyIdentifier::INACTIVE_TEXT),
            states: values(PropertyIdentifier::STATE_TEXT).collect(),
        }
    }
}

/// The binary object types, whose state is a BACnetBinaryPV.
pub(crate) fn is_binary(object_type: ObjectType) -> bool {
    [
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
    ]
    .contains(&object_type)
}

/// The multi-state object types, whose state is an Unsigned from 1.
pub(crate) fn is_multistate(object_type: ObjectType) -> bool {
    [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ]
    .contains(&object_type)
}

/// The DateTime Value object types, whose commanded value is a
/// BACnetDateTime.
pub(crate) fn is_date_time_value(object_type: ObjectType) -> bool {
    [
        ObjectType::DATETIME_VALUE,
        ObjectType::DATETIMEPATTERN_VALUE,
    ]
    .contains(&object_type)
}

/// The properties that hold a binary or multi-state object's state.
pub(crate) fn holds_state(property: PropertyIdentifier) -> bool {
    [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::FEEDBACK_VALUE,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        PropertyIdentifier::PRIORITY_ARRAY,
        PropertyIdentifier::ALARM_VALUE,
        PropertyIdentifier::ALARM_VALUES,
        PropertyIdentifier::FAULT_VALUES,
    ]
    .contains(&property)
}

/// Formats the values of one object.
#[derive(Debug, Clone, Copy)]
pub struct ValueFormat<'a> {
    object_type: ObjectType,
    texts: &'a StateTexts,
    floats: Floats,
    /// Elements of a list shown before the rest is summarised.
    max_shown: usize,
}

impl<'a> ValueFormat<'a> {
    /// A formatter for an object of `object_type`.
    pub fn new(object_type: ObjectType, texts: &'a StateTexts, floats: Floats) -> Self {
        Self {
            object_type,
            texts,
            floats,
            max_shown: MAX_LIST_SHOWN,
        }
    }

    /// The same formatter, showing every element of a list instead of the
    /// first few and a count.
    pub fn whole_lists(self) -> Self {
        Self {
            max_shown: usize::MAX,
            ..self
        }
    }

    /// The same settings for another object, as a value that names one
    /// (an action's target, a group member) needs.
    pub fn for_object<'b>(
        &self,
        object_type: ObjectType,
        texts: &'b StateTexts,
    ) -> ValueFormat<'b> {
        ValueFormat {
            object_type,
            texts,
            floats: self.floats,
            max_shown: self.max_shown,
        }
    }

    /// A REAL in this formatter's float style.
    pub fn real(&self, value: f32) -> String {
        primitive::text(
            PropertyIdentifier::PRESENT_VALUE,
            &PropertyValue::Real(value),
            self.floats,
        )
    }

    /// The text of each element of a constructed value, when the element
    /// table names the property's element and the octets decode as those
    /// elements to the end; `None` otherwise.
    pub fn constructed_items(
        &self,
        property: PropertyIdentifier,
        data: &[u8],
    ) -> Option<Vec<String>> {
        let (element, _) = property_element(self.object_type, property)?;
        constructed::elements(self, element, data, 0)
    }

    /// A property's value as display text: decoded as the element the table
    /// names for it when it has a renderer and the octets decode as those
    /// elements to the end, and otherwise as application values.
    pub fn value(&self, property: PropertyIdentifier, data: &[u8]) -> String {
        self.value_at(property, None, data)
    }

    /// The element and shape of a read of `property` at `index`, and the
    /// place (from 0) of its first element: an indexed read of a collection
    /// is the one element at index less one, and index 0 (an array's size)
    /// none.
    fn element_at(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Option<(ConstructedElement, ElementShape, usize)> {
        let (element, shape) = property_element(self.object_type, property)?;
        match index {
            None => Some((element, shape, 0)),
            Some(0) => None,
            Some(index) => Some((
                element,
                ElementShape::Single,
                usize::try_from(index - 1).ok()?,
            )),
        }
    }

    /// The elements of a read at `index`, when the table names the
    /// property's element and the octets decode as it.
    fn elements_at(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
        data: &[u8],
    ) -> Option<(ConstructedElement, ElementShape, Vec<String>)> {
        let (element, shape, first) = self.element_at(property, index)?;
        let texts = constructed::elements(self, element, data, first)?;
        Some((element, shape, texts))
    }

    /// [`Self::value`] of a read at an array index: one element of a
    /// collection for an index above 0, the array's size for index 0.
    pub fn value_at(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
        data: &[u8],
    ) -> String {
        if let Some((element, shape, texts)) = self.elements_at(property, index, data) {
            return match (shape, texts.as_slice()) {
                (ElementShape::Single, [one]) => one.clone(),
                _ => self.list(texts, constructed::separator(element)),
            };
        }
        let Some(values) = decode_all(data) else {
            return primitive::raw(data);
        };
        match values.as_slice() {
            [] => "(empty)".into(),
            // An array's size is a count, never a state.
            [one] if index == Some(0) => primitive::text(property, one, self.floats),
            [one] => self.element(property, one),
            [PropertyValue::Date(date), PropertyValue::Time(time)] => {
                primitive::date_time(date, time)
            }
            [PropertyValue::Date(start), PropertyValue::Date(end)] => {
                primitive::date_range(start, end)
            }
            many => self.list(
                many.iter()
                    .map(|value| self.element(property, value))
                    .collect(),
                ", ",
            ),
        }
    }

    /// A value with the units after it, as the compact places show it:
    /// `72.5 °F`. Only numbers take units.
    pub fn with_units(
        &self,
        property: PropertyIdentifier,
        data: &[u8],
        units: Option<EngineeringUnits>,
    ) -> String {
        let text = self.value(property, data);
        let numeric = matches!(
            decode_all(data).as_deref(),
            Some([PropertyValue::Real(_)
                | PropertyValue::Double(_)
                | PropertyValue::Unsigned(_)
                | PropertyValue::Signed(_)])
        );
        match units {
            Some(units) if numeric && units != EngineeringUnits::NO_UNITS => {
                format!("{text} {}", unit_symbol(units))
            }
            _ => text,
        }
    }

    /// A property's value as the device card shows it: a string without
    /// its quotes.
    pub fn plain(&self, property: PropertyIdentifier, data: &[u8]) -> String {
        match decode_all(data).as_deref() {
            Some([PropertyValue::CharacterString(text)]) => text.clone(),
            _ => self.value(property, data),
        }
    }

    /// One application value of `property`.
    pub fn element(&self, property: PropertyIdentifier, value: &PropertyValue) -> String {
        if holds_state(property)
            && let Some(state) = self.state(value)
        {
            return state;
        }
        if let Some((date, time)) = self.priority_date_time(property, value) {
            return primitive::date_time(&date, &time);
        }
        primitive::text(property, value, self.floats)
    }

    /// A DateTime Value's priority slot holding a date-time: the
    /// BACnetPriorityValue choice under context tag 1, which the generic
    /// decoder leaves as its octets (#2098). `None` for anything else.
    fn priority_date_time(
        &self,
        property: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<(Date, Time)> {
        let PropertyValue::ApplicationData(octets) = value else {
            return None;
        };
        if !is_date_time_value(self.object_type) || property != PropertyIdentifier::PRIORITY_ARRAY {
            return None;
        }
        match decode_priority_date_time(octets, 0) {
            Ok((date_time, end)) if end == octets.len() => Some(date_time),
            _ => None,
        }
    }

    /// A binary or multi-state object's state, named.
    fn state(&self, value: &PropertyValue) -> Option<String> {
        let named = |name: String, text: Option<&String>| match text {
            Some(text) if !text.is_empty() => format!("{name} ({text})"),
            _ => name,
        };
        match value {
            PropertyValue::Enumerated(raw @ (0 | 1)) if is_binary(self.object_type) => {
                Some(if *raw == 1 {
                    named("active".into(), self.texts.active.as_ref())
                } else {
                    named("inactive".into(), self.texts.inactive.as_ref())
                })
            }
            PropertyValue::Unsigned(state) if is_multistate(self.object_type) => {
                let text = usize::try_from(*state)
                    .ok()
                    .and_then(|state| state.checked_sub(1))
                    .and_then(|index| self.texts.states.get(index));
                Some(named(state.to_string(), text))
            }
            _ => None,
        }
    }
}

impl ValueFormat<'_> {
    /// Element texts as a list, the first `max_shown` in full.
    fn list(&self, texts: Vec<String>, separator: &str) -> String {
        let count = texts.len();
        let shown: Vec<String> = texts.into_iter().take(self.max_shown).collect();
        match count.saturating_sub(self.max_shown) {
            0 => format!("[{}]", shown.join(separator)),
            more => format!(
                "[{}{separator}+{more} more] ({count} items)",
                shown.join(separator)
            ),
        }
    }
}

/// Octets that don't decode, as every front end writes them: `[raw 0e 0f]`.
pub fn raw_text(data: &[u8]) -> String {
    primitive::raw(data)
}

/// The Object_Name among properties as read; `None` when the device
/// answered it with an error or left it out. See [`name_text`].
pub fn object_name(rows: &[RawRow]) -> Option<String> {
    let (_, value) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::OBJECT_NAME)?;
    name_text(value.as_deref().ok()?)
}

/// An encoded Object_Name as text (see [`clean_name`]); `None` unless it
/// is one character string.
pub fn name_text(data: &[u8]) -> Option<String> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::CharacterString(text)] => Some(clean_name(text)),
        _ => None,
    }
}

/// The Units among properties as read.
pub fn units(rows: &[RawRow]) -> Option<EngineeringUnits> {
    let (_, value) = rows.iter().find(|(p, _)| *p == PropertyIdentifier::UNITS)?;
    match decode_all(value.as_deref().ok()?)?.as_slice() {
        [PropertyValue::Enumerated(raw)] => Some(EngineeringUnits::from_raw(*raw)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod render_tests;
