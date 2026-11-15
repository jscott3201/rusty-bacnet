//! An [`Inventory`] as JSON and as CSV: the one writer of each, which the
//! `bacnet inventory` command prints.
//!
//! The JSON is the serde form of [`Inventory`] and [`PointRecord`]; this
//! module holds how their fields are written. Its field names are stable.
//! A value is `{"value": text, "decoded": data}`: its display text and its
//! typed form, the shape of the CLI's other JSON (`docs/CLI.md`, "Value
//! output").
//!
//! The CSV is one row a record under [`CSV_HEADER`], in that fixed column
//! order, as RFC 4180 writes fields: a field holding a comma, a double
//! quote, a carriage return or a line feed is put in double quotes, with
//! each double quote in it doubled; every other field is written bare.
//! Rows end with a line feed. The device summary and
//! [`Inventory::stopped`] are not in it.
//!
//! Both keep device text exactly as the device sent it, control
//! characters included. A spreadsheet may take a field that starts with
//! `=`, `+`, `-` or `@` for a formula: import the columns as text.

use std::fmt::{self, Write};

use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use super::{Inventory, PointRecord};
use crate::client::DeviceSummary;
use crate::error::OpsError;
use crate::format::{Datum, StateTexts, Value, unit_symbol};
use crate::label;

/// The CSV's header row, its columns in order:
///
/// - `device`: the device's address as a target (`192.168.1.10:47808`,
///   `2:0x05`);
/// - `device_instance`;
/// - `object`: `analog-input:1`, the type in kebab case;
/// - `name`, `description`: exactly as the device sent them;
/// - `units`: the unit's name in kebab case (`degrees-fahrenheit`);
/// - `state_texts`: a binary object's Inactive_Text then Active_Text, or
///   a multi-state object's State_Text from state 1, joined with `|`; a
///   `|` or a `\` inside a text is written `\|` or `\\`, and a text the
///   device didn't give is empty;
/// - `number_of_states`;
/// - `commandable`: `true` or `false`;
/// - `relinquish_default`, `cov_increment`, `min_present_value`,
///   `max_present_value`, `present_value`, `status_flags`: each value's
///   text ([`Value::text`]);
/// - `problems`: each as `property: error`, joined with `; `.
///
/// A field with nothing to say is empty.
pub const CSV_HEADER: &str = "device,device_instance,object,name,description,units,state_texts,\
    number_of_states,commandable,relinquish_default,cov_increment,min_present_value,\
    max_present_value,present_value,status_flags,problems";

impl Inventory {
    /// The inventory as JSON, indented, as `bacnet inventory --format
    /// json` prints it: the serde form, one object with `address`,
    /// `device`, `cov`, `points`, `unread` and `stopped`.
    pub fn to_json(&self) -> String {
        // Every key is a string and every number finite.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The records as CSV under [`CSV_HEADER`], as `bacnet inventory
    /// --format csv` prints it.
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        // Writing to a String can't fail.
        let _ = self.write_csv(&mut out);
        out
    }

    /// Write the records as CSV to `out`: [`CSV_HEADER`], then one row a
    /// record.
    pub fn write_csv(&self, out: &mut impl Write) -> fmt::Result {
        writeln!(out, "{CSV_HEADER}")?;
        self.points
            .iter()
            .try_for_each(|point| point.write_csv(out))
    }
}

impl PointRecord {
    /// Write the record as one CSV row to `out`, its fields in
    /// [`CSV_HEADER`]'s order.
    pub fn write_csv(&self, out: &mut impl Write) -> fmt::Result {
        let text = |value: &Option<Value>| value.as_ref().map(|value| value.text.clone());
        let number = self.number_of_states.map(|states| states.to_string());
        let problems: Vec<String> = self
            .problems
            .iter()
            .map(|problem| format!("{}: {}", label::kebab(problem.property), problem.error))
            .collect();
        let object = format!(
            "{}:{}",
            label::kebab(self.object.object_type()),
            self.object.instance_number()
        );
        let fields = [
            Some(self.device.to_string()),
            Some(self.device_instance.to_string()),
            Some(object),
            self.name.clone(),
            self.description.clone(),
            self.units.map(label::kebab),
            csv_state_texts(self),
            number,
            Some(self.commandable.to_string()),
            text(&self.relinquish_default),
            text(&self.cov_increment),
            text(&self.min_present_value),
            text(&self.max_present_value),
            text(&self.present_value),
            text(&self.status_flags),
            (!problems.is_empty()).then(|| problems.join("; ")),
        ];
        for (at, field) in fields.iter().enumerate() {
            if at > 0 {
                out.write_char(',')?;
            }
            csv_field(field.as_deref().unwrap_or_default(), out)?;
        }
        out.write_char('\n')
    }
}

/// One field as RFC 4180 writes it.
fn csv_field(field: &str, out: &mut impl Write) -> fmt::Result {
    if !field.contains([',', '"', '\r', '\n']) {
        return out.write_str(field);
    }
    out.write_char('"')?;
    for ch in field.chars() {
        if ch == '"' {
            out.write_char('"')?;
        }
        out.write_char(ch)?;
    }
    out.write_char('"')
}

/// The `state_texts` column: none for an object without any.
fn csv_state_texts(record: &PointRecord) -> Option<String> {
    let texts = &record.state_texts;
    let binary = texts.active.is_some() || texts.inactive.is_some();
    let states: Vec<&str> = if binary {
        vec![
            texts.inactive.as_deref().unwrap_or_default(),
            texts.active.as_deref().unwrap_or_default(),
        ]
    } else {
        texts.states.iter().map(String::as_str).collect()
    };
    if states.is_empty() {
        return None;
    }
    let escaped: Vec<String> = states
        .iter()
        .map(|text| text.replace('\\', "\\\\").replace('|', "\\|"))
        .collect();
    Some(escaped.join("|"))
}

/// A name in kebab case.
pub(super) fn kebab<S: Serializer>(
    property: &PropertyIdentifier,
    to: S,
) -> Result<S::Ok, S::Error> {
    to.serialize_str(&label::kebab(property))
}

/// An error as its text.
pub(super) fn display<S: Serializer>(error: &impl fmt::Display, to: S) -> Result<S::Ok, S::Error> {
    to.collect_str(error)
}

/// An object as the typed form of a value writes one.
fn object_datum(object: ObjectIdentifier) -> Datum {
    Datum::Object {
        object_type: label::kebab(object.object_type()),
        type_number: object.object_type().to_raw(),
        instance: object.instance_number(),
    }
}

pub(super) fn object<S: Serializer>(object: &ObjectIdentifier, to: S) -> Result<S::Ok, S::Error> {
    object_datum(*object).serialize(to)
}

pub(super) fn objects<S: Serializer>(
    objects: &[ObjectIdentifier],
    to: S,
) -> Result<S::Ok, S::Error> {
    let mut list = to.serialize_seq(Some(objects.len()))?;
    for object in objects {
        list.serialize_element(&object_datum(*object))?;
    }
    list.end()
}

/// Units as their number, their name in kebab case, and what a value's
/// text writes after the number: the unit's symbol, or its name when it
/// has none.
pub(super) fn units<S: Serializer>(
    units: &Option<EngineeringUnits>,
    to: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Units {
        value: u32,
        name: String,
        symbol: String,
    }
    units
        .map(|units| Units {
            value: units.to_raw(),
            name: label::kebab(units),
            symbol: unit_symbol(units),
        })
        .serialize(to)
}

pub(super) fn state_texts<S: Serializer>(texts: &StateTexts, to: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Texts<'a> {
        active: &'a Option<String>,
        inactive: &'a Option<String>,
        states: &'a [String],
    }
    Texts {
        active: &texts.active,
        inactive: &texts.inactive,
        states: &texts.states,
    }
    .serialize(to)
}

/// A value's text and typed form.
#[derive(Serialize)]
struct Both<'a> {
    value: &'a str,
    decoded: &'a Datum,
}

impl<'a> From<&'a Value> for Both<'a> {
    fn from(value: &'a Value) -> Self {
        Self {
            value: &value.text,
            decoded: &value.data,
        }
    }
}

pub(super) fn value<S: Serializer>(value: &Option<Value>, to: S) -> Result<S::Ok, S::Error> {
    value.as_ref().map(Both::from).serialize(to)
}

/// The summary: the instance, the Object_Name exactly as sent, and each
/// property read, by its name in kebab case, as its value or its error.
pub(super) fn summary<S: Serializer>(summary: &DeviceSummary, to: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(untagged)]
    enum Read<'a> {
        Value(Both<'a>),
        Error { error: String },
    }
    struct Properties<'a>(&'a DeviceSummary);
    impl Serialize for Properties<'_> {
        fn serialize<S: Serializer>(&self, to: S) -> Result<S::Ok, S::Error> {
            let mut map = to.serialize_map(Some(self.0.readings.len()))?;
            for reading in &self.0.readings {
                let read = match &reading.value {
                    Ok(value) => Read::Value(value.into()),
                    Err(error) => Read::Error {
                        error: error.to_string(),
                    },
                };
                map.serialize_entry(&label::kebab(reading.property), &read)?;
            }
            map.end()
        }
    }
    #[derive(Serialize)]
    struct Summary<'a> {
        instance: u32,
        name: Option<&'a str>,
        properties: Properties<'a>,
    }
    Summary {
        instance: summary.device,
        name: summary
            .value(PropertyIdentifier::OBJECT_NAME)
            .and_then(Value::as_str),
        properties: Properties(summary),
    }
    .serialize(to)
}

pub(super) fn stopped<S: Serializer>(stopped: &Option<OpsError>, to: S) -> Result<S::Ok, S::Error> {
    stopped.as_ref().map(ToString::to_string).serialize(to)
}
