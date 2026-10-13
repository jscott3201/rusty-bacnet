//! Read commands: ReadProperty (RP) and ReadPropertyMultiple (RPM).
//!
//! ReadRange is in [`super::read_range`].

use bacnet_client::client::BACnetClient;
use bacnet_client::tags::{decode_tags_read, TagsRead};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::PropertyValue;

use crate::output::{self, OutputFormat};
use crate::parse;

/// Decode the application-tagged values in `data` and format each one, with
/// the hex of whatever follows the last value that decoded.
pub(super) fn format_application_values(data: &[u8]) -> (Vec<String>, Option<String>) {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(output::format_property_value(&value));
                offset = next;
            }
            Err(_) => return (values, Some(hex(&data[offset..]))),
        }
    }
    (values, None)
}

/// Decode all application-tagged values from raw bytes and format them.
pub(super) fn decode_and_format(data: &[u8]) -> String {
    let (mut values, undecoded) = format_application_values(data);
    if let Some(hex) = undecoded {
        values.push(format!("[raw: {hex}]"));
    }
    values.join(", ")
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
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;

    // If the property is ALL, use RPM instead.
    if property == PropertyIdentifier::ALL {
        return read_all_properties(client, mac, object_type, instance, format).await;
    }

    let ack = client.read_property(mac, oid, property, index).await?;
    let decoded = format_read_value(
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    );

    output::print_read_result(
        &format!(
            "{}:{}",
            ack.object_identifier.object_type(),
            ack.object_identifier.instance_number()
        ),
        &format!("{}", ack.property_identifier),
        ack.property_array_index,
        &decoded,
        format,
    );
    Ok(())
}

/// Read all properties of an object using RPM with PropertyIdentifier::ALL.
async fn read_all_properties<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
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

    let ack = client.read_property_multiple(mac, specs).await?;
    print_rpm_results(&ack.list_of_read_access_results, format);
    Ok(())
}

/// Read multiple properties from CLI string specs.
///
/// Specs format: alternating object specifiers and comma-separated property lists.
/// Example: `["ai:1", "pv,object-name", "ao:1", "pv"]`
pub async fn read_multiple_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
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

    let ack = client.read_property_multiple(mac, access_specs).await?;
    print_rpm_results(&ack.list_of_read_access_results, format);
    Ok(())
}

/// Print RPM results.
fn print_rpm_results(results: &[bacnet_services::rpm::ReadAccessResult], format: OutputFormat) {
    let mut entries = Vec::new();
    for result in results {
        let obj_str = format!(
            "{}:{}",
            result.object_identifier.object_type(),
            result.object_identifier.instance_number()
        );
        for elem in &result.list_of_results {
            let value_str = if let Some(ref value_bytes) = elem.property_value {
                format_read_value(
                    elem.property_identifier,
                    elem.property_array_index,
                    value_bytes,
                )
            } else if let Some((class, code)) = elem.error {
                format!("ERROR: {class}:{code}")
            } else {
                "???".to_string()
            };
            entries.push((
                obj_str.clone(),
                format!("{}", elem.property_identifier),
                elem.property_array_index,
                value_str,
            ));
        }
    }
    output::print_rpm_table(&entries, format);
}

/// Interpret Tags only at RP/RPM callsites; ReadRange keeps its generic codec.
fn format_read_value(property: PropertyIdentifier, index: Option<u32>, data: &[u8]) -> String {
    if property != PropertyIdentifier::TAGS {
        return decode_and_format(data);
    }
    match decode_tags_read(index, data) {
        Ok(TagsRead::Whole(tags)) => format!(
            "[{}]",
            tags.iter().map(format_tag).collect::<Vec<_>>().join(", ")
        ),
        Ok(TagsRead::Element(tag)) => format_tag(&tag),
        Ok(TagsRead::Size(size)) => size.to_string(),
        Err(_) => format!("[invalid Tags; raw: {}]", hex(data)),
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
                value => output::format_property_value(value),
            };
            format!("{name}={value}")
        }
    }
}

#[cfg(test)]
#[path = "read_tags_tests.rs"]
mod tags_tests;
