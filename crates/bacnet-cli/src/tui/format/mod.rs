//! The TUI's one value formatter: property values as read, turned into the
//! text the browser, the watch list and the write previews show.
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
//! the write previews, the ledger and the CSV log, which keep every digit.
//!
//! The one-shot CLI's `output::format_property_value` is a separate
//! contract and isn't used here.

mod constructed;
mod primitive;
mod units;

use bacnet_encoding::constructed::{property_element, ConstructedElement, ElementShape};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

use units::unit_symbol;

/// A property as read: its raw application-tagged value, or an error.
pub(crate) type RawRow = (PropertyIdentifier, Result<Vec<u8>, String>);

/// Values shown before the rest of a list is summarised.
const MAX_LIST_SHOWN: usize = 8;

/// Every application-tagged value in `data`, or `None` if any part of it
/// doesn't decode. A context tag decodes as `ApplicationData`.
pub(crate) fn decode_all(data: &[u8]) -> Option<Vec<PropertyValue>> {
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
pub(crate) enum Floats {
    /// At most six significant digits, trailing zeros trimmed.
    Display,
    /// Every digit: the shortest text that reads back as the same value.
    Exact,
}

/// The object's own names for its states, when they were read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StateTexts {
    /// A binary object's Active_Text.
    pub(crate) active: Option<String>,
    /// A binary object's Inactive_Text.
    pub(crate) inactive: Option<String>,
    /// A multi-state object's State_Text, state 1 first.
    pub(crate) states: Vec<String>,
}

impl StateTexts {
    /// The properties that hold an object type's state texts, to read with
    /// the value; none for a type without them.
    pub(crate) fn properties(object_type: ObjectType) -> &'static [PropertyIdentifier] {
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

    /// The texts among properties as read.
    pub(crate) fn from_rows(rows: &[RawRow]) -> Self {
        let values = |property| {
            rows.iter()
                .find(|(p, _)| *p == property)
                .and_then(|(_, value)| value.as_deref().ok())
                .and_then(decode_all)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| match value {
                    PropertyValue::CharacterString(text) => Some(text),
                    _ => None,
                })
        };
        Self {
            active: values(PropertyIdentifier::ACTIVE_TEXT).next(),
            inactive: values(PropertyIdentifier::INACTIVE_TEXT).next(),
            states: values(PropertyIdentifier::STATE_TEXT).collect(),
        }
    }
}

/// The binary object types, whose state is a BACnetBinaryPV.
fn is_binary(object_type: ObjectType) -> bool {
    [
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
    ]
    .contains(&object_type)
}

/// The multi-state object types, whose state is an Unsigned from 1.
fn is_multistate(object_type: ObjectType) -> bool {
    [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ]
    .contains(&object_type)
}

/// The properties that hold a binary or multi-state object's state.
fn holds_state(property: PropertyIdentifier) -> bool {
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
pub(crate) struct ValueFormat<'a> {
    object_type: ObjectType,
    texts: &'a StateTexts,
    floats: Floats,
}

impl<'a> ValueFormat<'a> {
    /// A formatter for an object of `object_type`.
    pub(crate) fn new(object_type: ObjectType, texts: &'a StateTexts, floats: Floats) -> Self {
        Self {
            object_type,
            texts,
            floats,
        }
    }

    /// A property's value as display text: decoded as the element the table
    /// names for it when it has a renderer and the octets decode as those
    /// elements to the end, and otherwise as application values.
    pub(crate) fn value(&self, property: PropertyIdentifier, data: &[u8]) -> String {
        if let Some((element, shape)) = property_element(self.object_type, property) {
            if let Some(texts) = constructed::elements(self, element, data) {
                // Schedule days hold lists of their own, so `; ` parts them.
                let separator = if element == ConstructedElement::DailySchedule {
                    "; "
                } else {
                    ", "
                };
                return match (shape, texts.as_slice()) {
                    (ElementShape::Single, [one]) => one.clone(),
                    _ => list(texts, separator),
                };
            }
        }
        let Some(values) = decode_all(data) else {
            return primitive::raw(data);
        };
        match values.as_slice() {
            [] => "(empty)".into(),
            [one] => self.element(property, one),
            [PropertyValue::Date(date), PropertyValue::Time(time)] => {
                primitive::date_time(date, time)
            }
            [PropertyValue::Date(start), PropertyValue::Date(end)] => {
                primitive::date_range(start, end)
            }
            many => list(
                many.iter()
                    .map(|value| self.element(property, value))
                    .collect(),
                ", ",
            ),
        }
    }

    /// A value with the units after it, as the compact places show it:
    /// `72.5 °F`. Only numbers take units.
    pub(crate) fn with_units(
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
    pub(crate) fn plain(&self, property: PropertyIdentifier, data: &[u8]) -> String {
        match decode_all(data).as_deref() {
            Some([PropertyValue::CharacterString(text)]) => text.clone(),
            _ => self.value(property, data),
        }
    }

    /// One application value of `property`.
    pub(crate) fn element(&self, property: PropertyIdentifier, value: &PropertyValue) -> String {
        if holds_state(property) {
            if let Some(state) = self.state(value) {
                return state;
            }
        }
        primitive::text(property, value, self.floats)
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

/// Element texts as a list, the first [`MAX_LIST_SHOWN`] in full.
fn list(texts: Vec<String>, separator: &str) -> String {
    let count = texts.len();
    let shown: Vec<String> = texts.into_iter().take(MAX_LIST_SHOWN).collect();
    match count.saturating_sub(MAX_LIST_SHOWN) {
        0 => format!("[{}]", shown.join(separator)),
        more => format!(
            "[{}{separator}+{more} more] ({count} items)",
            shown.join(separator)
        ),
    }
}

/// Longest name kept, in characters; a device's longer name is cut.
pub(crate) const MAX_NAME_CHARS: usize = 128;

/// The Object_Name among properties as read; `None` when the device
/// answered it with an error or left it out. See [`name_text`].
pub(crate) fn object_name(rows: &[RawRow]) -> Option<String> {
    let (_, value) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::OBJECT_NAME)?;
    name_text(value.as_deref().ok()?)
}

/// An encoded Object_Name as text (see [`clean_name`]); `None` unless it
/// is one character string.
pub(crate) fn name_text(data: &[u8]) -> Option<String> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::CharacterString(text)] => Some(clean_name(text)),
        _ => None,
    }
}

/// A device's name for display: each control character as a space, so it
/// can't move the cursor or break a row, and at most [`MAX_NAME_CHARS`].
pub(crate) fn clean_name(text: &str) -> String {
    text.chars()
        .take(MAX_NAME_CHARS)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// The Units among properties as read.
pub(crate) fn units(rows: &[RawRow]) -> Option<EngineeringUnits> {
    let (_, value) = rows.iter().find(|(p, _)| *p == PropertyIdentifier::UNITS)?;
    match decode_all(value.as_deref().ok()?)?.as_slice() {
        [PropertyValue::Enumerated(raw)] => Some(EngineeringUnits::from_raw(*raw)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
