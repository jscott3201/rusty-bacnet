//! A log's values named by the property it samples (#2091).
//!
//! A Trend Log record carries a bare value: an enumeration is a number and
//! a bit string its bits, with nothing to say which property they came
//! from. The log's Log_DeviceObjectProperty does say: one reference for a
//! Trend Log (Clause 12.25), one per member for a Trend Log Multiple
//! (Clause 12.30), each in the order of the record's values. Read once
//! per read of the log, it lets the formatter name each column's
//! values as it would name the monitored property itself: `offnormal`
//! rather than `enum 2`, `in-alarm` rather than `bits(1000)`, and a binary
//! object's state as `active`.
//!
//! A column that samples a binary or multi-state object's state (its
//! Present_Value, Relinquish_Default, a Priority_Array slot and the like)
//! is named further by that object's own state texts, read once per read
//! of the log after the reference ([`LogColumns::read_state_texts`],
//! #2122): `active (Running)` and `2 (Low)`, as the TUI names an object's
//! state. At most [`MAX_STATE_TEXT_OBJECTS`] objects are read, and only
//! objects on the log's own device.
//!
//! Naming never fails a read. A reference that can't be read, doesn't
//! decode or names nothing leaves its column as the log gives it, and so
//! does a value whose datatype the property doesn't name (a REAL logged
//! where an enumeration was expected prints as the REAL).

use std::future::Future;
use std::ops::ControlFlow;

use bacnet_encoding::constructed::decode_device_object_property_reference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{Datum, StateTexts, ValueFormat, holds_state, is_binary, is_multistate};

/// The property a log column samples, from its Log_DeviceObjectProperty
/// reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggedProperty {
    /// The monitored object. Its type names a binary or multi-state
    /// object's states.
    pub object: ObjectIdentifier,
    /// The monitored property.
    pub property: PropertyIdentifier,
    /// The array index the reference names, if any.
    pub index: Option<u32>,
    /// The device instance the reference names, if it names one; `None`
    /// means the log's own device.
    pub device: Option<u32>,
}

impl LoggedProperty {
    /// Whether this column samples a binary or multi-state object's state,
    /// which that object's state texts name.
    pub fn holds_states(&self) -> bool {
        let object_type = self.object.object_type();
        (is_binary(object_type) || is_multistate(object_type)) && holds_state(self.property)
    }

    /// Whether the monitored object is on the log's device, `log_device`
    /// when it is known: always for a reference that names no device.
    fn on(&self, log_device: Option<u32>) -> bool {
        self.device.is_none_or(|device| log_device == Some(device))
    }
}

/// The most objects one read of a log reads state texts for: a Trend Log
/// needs one, and a Trend Log Multiple with more state members than this
/// names the rest by number, as before (#2122).
pub const MAX_STATE_TEXT_OBJECTS: usize = 8;

/// No state texts, for a column without them.
static NO_TEXTS: StateTexts = StateTexts {
    active: None,
    inactive: None,
    states: Vec::new(),
};

/// What each column of a log samples: one column for a Trend Log, one per
/// member for a Trend Log Multiple, `None` where the reference is empty;
/// and, once read, the state texts of the objects whose states columns
/// sample. The default names no column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogColumns {
    columns: Vec<Option<LoggedProperty>>,
    /// The state texts of each column as the device sent them, by column;
    /// shorter when the last columns have none.
    raw: Vec<StateTexts>,
    /// The same, cleaned for display ([`clean_name`](super::clean_name)).
    texts: Vec<StateTexts>,
}

impl LogColumns {
    /// Whether a log of `object_type` has a Log_DeviceObjectProperty to
    /// name its values by: a Trend Log or a Trend Log Multiple.
    pub fn applies(object_type: ObjectType) -> bool {
        object_type == ObjectType::TREND_LOG || object_type == ObjectType::TREND_LOG_MULTIPLE
    }

    /// The columns a Log_DeviceObjectProperty value names, read whole:
    /// its references back to back. Octets that don't decode to the end
    /// name no column, so the values print as the log gives them.
    pub fn decode(data: &[u8]) -> Self {
        let mut columns = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let Ok((reference, next)) = decode_device_object_property_reference(data, offset)
            else {
                return Self::default();
            };
            columns.push((!reference.is_unset()).then(|| {
                LoggedProperty {
                    object: reference.object_identifier,
                    property: PropertyIdentifier::from_raw(reference.property_identifier),
                    index: reference.property_array_index,
                    // The wildcard device instance stands for the log's own.
                    device: reference
                        .device_identifier
                        .map(|device| device.instance_number())
                        .filter(|device| *device != ObjectIdentifier::WILDCARD_INSTANCE),
                }
            }));
            offset = next;
        }
        columns.into_iter().collect()
    }

    /// The property column `column` (from 0) samples, if named.
    pub fn column(&self, column: usize) -> Option<LoggedProperty> {
        self.columns.get(column).copied().flatten()
    }

    /// Whether no column is named.
    pub fn is_empty(&self) -> bool {
        self.columns.iter().all(Option::is_none)
    }

    /// The state texts of the object column `column` samples, cleaned for
    /// display ([`StateTexts::cleaned`]); none unless they were read. A
    /// formatter made with them names the column's states
    /// ([`ValueFormat::new`]). Device-supplied text: a front end shows it
    /// as data.
    pub fn state_texts(&self, column: usize) -> &StateTexts {
        self.texts.get(column).unwrap_or(&NO_TEXTS)
    }

    /// [`Self::state_texts`] as the device sent them, uncleaned and uncut,
    /// for a front end that cleans device text its own way (the MCP
    /// server's envelopes, which also say when they cut it).
    pub fn raw_state_texts(&self, column: usize) -> &StateTexts {
        self.raw.get(column).unwrap_or(&NO_TEXTS)
    }

    /// The objects whose state texts would name a column, in column order,
    /// each once, on the log's device only, at most
    /// [`MAX_STATE_TEXT_OBJECTS`].
    fn state_objects(&self, log_device: Option<u32>) -> Vec<ObjectIdentifier> {
        let mut objects = Vec::new();
        for column in self.columns.iter().flatten() {
            if column.holds_states() && column.on(log_device) && !objects.contains(&column.object) {
                if objects.len() == MAX_STATE_TEXT_OBJECTS {
                    break;
                }
                objects.push(column.object);
            }
        }
        objects
    }

    /// Give every column that samples `object`'s states on the log's
    /// device these texts, cleaned for display.
    fn set_state_texts(
        &mut self,
        log_device: Option<u32>,
        object: ObjectIdentifier,
        texts: &StateTexts,
    ) {
        let cleaned = texts.cleaned();
        for (index, column) in self.columns.iter().enumerate() {
            if column.is_some_and(|column| {
                column.object == object && column.holds_states() && column.on(log_device)
            }) {
                if self.texts.len() <= index {
                    self.texts.resize(index + 1, StateTexts::default());
                    self.raw.resize(index + 1, StateTexts::default());
                }
                self.texts[index] = cleaned.clone();
                self.raw[index] = texts.clone();
            }
        }
    }

    /// Read the state texts of the objects whose states the columns
    /// sample, each once, through `read`, and keep them to name those
    /// columns. `log_device` is the log's device instance, when known: an
    /// object on another device is not read. `read` gives an object's
    /// texts (empty ones for a refused read, which leaves its columns as
    /// they were) or breaks off the reading (a device gone silent, a spent
    /// budget). Nothing is read when no column samples a state.
    pub async fn read_state_texts<F, Fut>(&mut self, log_device: Option<u32>, mut read: F)
    where
        F: FnMut(ObjectIdentifier) -> Fut,
        Fut: Future<Output = ControlFlow<(), StateTexts>>,
    {
        for object in self.state_objects(log_device) {
            match read(object).await {
                ControlFlow::Continue(texts) => self.set_state_texts(log_device, object, &texts),
                ControlFlow::Break(()) => break,
            }
        }
    }
}

impl FromIterator<Option<LoggedProperty>> for LogColumns {
    /// The columns in order, `None` for one nothing names.
    fn from_iter<I: IntoIterator<Item = Option<LoggedProperty>>>(columns: I) -> Self {
        Self {
            columns: columns.into_iter().collect(),
            raw: Vec::new(),
            texts: Vec::new(),
        }
    }
}

/// The property a log's own values belong to when no column is named: one
/// with no enumeration, named bits or constructed element.
const UNNAMED: PropertyIdentifier = PropertyIdentifier::LOG_BUFFER;

impl ValueFormat<'_> {
    /// This formatter for the object `column` monitors, and the property to
    /// name its values by.
    fn for_column(&self, column: Option<LoggedProperty>) -> (Self, PropertyIdentifier) {
        match column {
            Some(column) => (
                Self {
                    object_type: column.object.object_type(),
                    ..*self
                },
                column.property,
            ),
            None => (*self, UNNAMED),
        }
    }

    /// One logged value as text, named for the property `column` samples,
    /// and a state by this formatter's state texts: make it with the
    /// column's ([`LogColumns::state_texts`]).
    /// An enumeration nothing names shows as `enum 3`, so it isn't taken
    /// for a count; a bit string nothing names shows as its bits.
    ///
    /// A primitive value is named by its property alone, whether the log
    /// samples the whole property or one element of it (a Priority_Array
    /// slot names its state as the property does), so `column.index` only
    /// matters to the octet forms, which decode one element of a collection.
    pub fn logged(&self, column: Option<LoggedProperty>, value: &PropertyValue) -> String {
        let (format, property) = self.for_column(column);
        let text = format.element(property, value);
        match value {
            PropertyValue::Enumerated(raw) if text == raw.to_string() => format!("enum {raw}"),
            _ => text,
        }
    }

    /// [`Self::logged`] as typed data: an enumeration with its name and a
    /// bit string with its set bits' names, when the property names them.
    pub fn logged_data(&self, column: Option<LoggedProperty>, value: &PropertyValue) -> Datum {
        let (format, property) = self.for_column(column);
        format.element_data(property, value)
    }

    /// A logged value of another datatype, as its tagged octets, as text:
    /// decoded as a read of the property `column` samples would be.
    pub fn logged_octets(&self, column: Option<LoggedProperty>, octets: &[u8]) -> String {
        let (format, property) = self.for_column(column);
        format.value_at(property, column.and_then(|column| column.index), octets)
    }

    /// [`Self::logged_octets`] as typed data.
    pub fn logged_octets_data(&self, column: Option<LoggedProperty>, octets: &[u8]) -> Datum {
        let (format, property) = self.for_column(column);
        format.data_at(property, column.and_then(|column| column.index), octets)
    }
}

#[cfg(test)]
#[path = "logged_tests.rs"]
mod tests;
