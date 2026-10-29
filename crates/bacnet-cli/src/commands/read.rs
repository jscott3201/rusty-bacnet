//! Read commands: ReadProperty (RP) and ReadPropertyMultiple (RPM).
//!
//! ReadRange is in [`super::read_range`].
//!
//! A value that holds a binary or multi-state object's state shows it with
//! the object's own text, `2 (Low)` or `active (Running)`, as `read-range`
//! and the TUI do (#2132). The texts cost one more request an object, sent
//! only once the read is answered and only for such a value, unless the
//! read already holds them (`read msv:1 all`). A binary object's pair goes
//! by ReadPropertyMultiple; on a device without it, after one refused RPM a
//! command, by ReadProperty for each text. A refused read, an empty
//! text or a state past the end keeps the plain form, and a device that
//! stops answering keeps it for the rest of the command without failing it.

use std::cell::Cell;
use std::collections::HashMap;

use bacnet_client::client::BACnetClient;
use bacnet_client::tags::{TagsRead, decode_tags_read};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_ops::destination::Destination;
use bacnet_ops::format::{Floats, RawRow, RowError, StateTexts, ValueFormat, raw_text};
use bacnet_ops::parse;
use bacnet_services::rpm::ReadAccessResult;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::PropertyValue;

use crate::output::{self, OutputFormat};

/// The one-shot CLI's value formatter for an object of `object_type`:
/// the shared formatter with every digit and every list element.
pub(super) fn formatter(object_type: ObjectType) -> ValueFormat<'static> {
    formatter_with(object_type, &output::NO_STATE_TEXTS)
}

/// [`formatter`], naming states with `texts`.
fn formatter_with(object_type: ObjectType, texts: &StateTexts) -> ValueFormat<'_> {
    ValueFormat::new(object_type, texts, Floats::Exact).whole_lists()
}

/// The state texts one command names states with, each object's read at
/// most once and cleaned for display.
#[derive(Default)]
struct Texts {
    by_object: HashMap<ObjectIdentifier, StateTexts>,
    /// Set once a text read found the device not answering: no more are
    /// sent, so a silent device costs one timeout.
    silent: bool,
    /// Set once the device refuses ReadPropertyMultiple: the rest of the
    /// command reads each text by ReadProperty, so a device without RPM
    /// costs one refused RPM a command, not one an object.
    no_rpm: Cell<bool>,
}

impl Texts {
    /// Keep the texts `results` hold for each object they read them of, so
    /// those cost no request: an `all` read, or a list that names them.
    fn keep_read(&mut self, results: &[ReadAccessResult]) {
        // An object may come back in several results (`readm` names it
        // twice): gather its rows first.
        let mut rows: HashMap<ObjectIdentifier, Vec<RawRow>> = HashMap::new();
        for result in results {
            let object = result.object_identifier;
            let wanted = StateTexts::properties(object.object_type());
            for element in &result.list_of_results {
                if element.property_array_index.is_none()
                    && wanted.contains(&element.property_identifier)
                {
                    let value = element.property_value.clone().ok_or(RowError::NoValue);
                    rows.entry(object)
                        .or_default()
                        .push((element.property_identifier, value));
                }
            }
        }
        // Only all of them: `readm bv:1 pv,active-text` still reads the pair.
        for (object, rows) in rows {
            let wanted = StateTexts::properties(object.object_type());
            if wanted
                .iter()
                .all(|p| rows.iter().any(|(read, _)| read == p))
            {
                self.by_object
                    .insert(object, StateTexts::from_rows(&rows).cleaned());
            }
        }
    }

    /// Read `object`'s texts if a value of `property` at `index` needs
    /// them and they aren't known yet.
    async fn want<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        to: &Destination,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) {
        if self.silent
            || self.by_object.contains_key(&object)
            || !StateTexts::name(object.object_type(), property, index)
        {
            return;
        }
        let texts = match to.state_texts(client, object, &self.no_rpm).await {
            Ok(texts) => texts.cleaned(),
            Err(_) => {
                self.silent = true;
                StateTexts::default()
            }
        };
        self.by_object.insert(object, texts);
    }

    /// The texts known for `object`; none when they weren't read.
    fn of(&self, object: ObjectIdentifier) -> &StateTexts {
        self.by_object
            .get(&object)
            .unwrap_or(&output::NO_STATE_TEXTS)
    }
}

/// The items of a list or array value as text, one an element, with
/// whatever follows the last item that decoded as `[raw …]`: constructed
/// elements when the element table names the property's element, and
/// otherwise application values.
pub(super) fn format_items(
    format: &ValueFormat<'_>,
    property: PropertyIdentifier,
    data: &[u8],
) -> (Vec<String>, Option<String>) {
    if let Some(items) = format.constructed_items(property, data) {
        return (items, None);
    }
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(format.element(property, &value));
                offset = next;
            }
            Err(_) => return (values, Some(raw_text(&data[offset..]))),
        }
    }
    (values, None)
}

/// Octets as space-separated lowercase hex.
pub(super) fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a single property and print its value.
pub async fn read_property_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;

    // If the property is ALL, use RPM instead.
    if property == PropertyIdentifier::ALL {
        return read_all_properties(client, to, object_type, instance, format).await;
    }

    let ack = to.read_property(client, oid, property, index).await?;
    let object_type = ack.object_identifier.object_type();
    let mut texts = Texts::default();
    texts
        .want(
            client,
            to,
            ack.object_identifier,
            ack.property_identifier,
            ack.property_array_index,
        )
        .await;
    let values = formatter_with(object_type, texts.of(ack.object_identifier));
    let decoded = format_read_value(
        &values,
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    );
    let data = values.data_at(
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    );

    output::print_read_result(
        &format!(
            "{}:{}",
            object_type,
            ack.object_identifier.instance_number()
        ),
        &format!("{}", ack.property_identifier),
        ack.property_array_index,
        &decoded,
        &data,
        format,
    );
    Ok(())
}

/// Read all properties of an object using RPM with PropertyIdentifier::ALL.
async fn read_all_properties<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let specs = vec![ReadAccessSpecification {
        object_identifier: oid,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::ALL,
            property_array_index: None,
        }],
    }];

    let ack = to.read_property_multiple(client, specs).await?;
    print_rpm_results(client, to, &ack.list_of_read_access_results, format).await;
    Ok(())
}

/// Read multiple properties from CLI string specs.
///
/// Specs format: alternating object specifiers and comma-separated property lists.
/// Example: `["ai:1", "pv,object-name", "ao:1", "pv"]`
pub async fn read_multiple_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    specs: &[String],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut access_specs = Vec::new();
    let mut i = 0;
    while i < specs.len() {
        // Try to parse as object specifier.
        let (obj_type, instance) = parse::parse_object_specifier(&specs[i])?;
        let oid = ObjectIdentifier::new(obj_type, instance)?;

        // Next arg should be comma-separated property list.
        let mut props = Vec::new();
        if i + 1 < specs.len() {
            i += 1;
            for prop_str in specs[i].split(',') {
                let (prop, idx) = parse::parse_property(prop_str.trim())?;
                props.push(PropertyReference {
                    property_identifier: prop,
                    property_array_index: idx,
                });
            }
        } else {
            // Default to PRESENT_VALUE if no properties specified.
            eprintln!(
                "Note: no properties specified for {}:{}, defaulting to present-value",
                obj_type, instance
            );
            props.push(PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            });
        }

        access_specs.push(ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: props,
        });
        i += 1;
    }

    let ack = to.read_property_multiple(client, access_specs).await?;
    print_rpm_results(client, to, &ack.list_of_read_access_results, format).await;
    Ok(())
}

/// Print RPM results, each state named by its object's texts: those the
/// results hold, or else one more read an object that needs them.
async fn print_rpm_results<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    results: &[ReadAccessResult],
    format: OutputFormat,
) {
    let mut texts = Texts::default();
    texts.keep_read(results);
    for result in results {
        for elem in &result.list_of_results {
            if elem.property_value.is_some() {
                texts
                    .want(
                        client,
                        to,
                        result.object_identifier,
                        elem.property_identifier,
                        elem.property_array_index,
                    )
                    .await;
            }
        }
    }
    let mut entries = Vec::new();
    for result in results {
        let obj_str = format!(
            "{}:{}",
            result.object_identifier.object_type(),
            result.object_identifier.instance_number()
        );
        let values = formatter_with(
            result.object_identifier.object_type(),
            texts.of(result.object_identifier),
        );
        for elem in &result.list_of_results {
            let (value, data) = if let Some(ref value_bytes) = elem.property_value {
                let text = format_read_value(
                    &values,
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                );
                let data = values.data_at(
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                );
                (text, Some(data))
            } else if let Some((class, code)) = elem.error {
                (format!("ERROR: {class}:{code}"), None)
            } else {
                ("???".to_string(), None)
            };
            entries.push(output::RpmEntry {
                object: obj_str.clone(),
                property: format!("{}", elem.property_identifier),
                index: elem.property_array_index,
                value,
                data,
            });
        }
    }
    output::print_rpm_table(&entries, format);
}

/// A read value as text: Tags as named values (the form `read` documents),
/// any other property through the shared formatter.
fn format_read_value(
    format: &ValueFormat<'_>,
    property: PropertyIdentifier,
    index: Option<u32>,
    data: &[u8],
) -> String {
    if property != PropertyIdentifier::TAGS {
        return format.value_at(property, index, data);
    }
    match decode_tags_read(index, data) {
        Ok(TagsRead::Whole(tags)) => format!(
            "[{}]",
            tags.iter().map(format_tag).collect::<Vec<_>>().join(", ")
        ),
        Ok(TagsRead::Element(tag)) => format_tag(&tag),
        Ok(TagsRead::Size(size)) => size.to_string(),
        Err(_) => format!("[invalid Tags] {}", raw_text(data)),
    }
}

fn format_tag(tag: &BACnetNameValue) -> String {
    let name = serde_json::to_string(&tag.name).expect("serialize tag name");
    match &tag.value {
        None => name,
        Some(value) => {
            let value = match value {
                PropertyValue::CharacterString(text) => {
                    serde_json::to_string(text).expect("serialize tag text")
                }
                value => bacnet_ops::format::format_property_value(value),
            };
            format!("{name}={value}")
        }
    }
}

#[cfg(test)]
#[path = "read_tags_tests.rs"]
mod tags_tests;
