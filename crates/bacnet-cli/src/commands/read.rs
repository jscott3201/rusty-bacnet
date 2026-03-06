//! Read commands: ReadProperty (RP) and ReadPropertyMultiple (RPM).

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::output::{self, OutputFormat};
use crate::parse;

/// Decode all application-tagged values from raw bytes and format them.
fn decode_and_format(data: &[u8]) -> String {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(output::format_property_value(&value));
                offset = next;
            }
            Err(_) => {
                let hex: String = data[offset..]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                values.push(format!("[raw: {hex}]"));
                break;
            }
        }
    }
    values.join(", ")
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
    let decoded = decode_and_format(&ack.property_value);

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

/// Read a range of items from a list or log-buffer property.
pub async fn read_range_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;

    let ack = client.read_range(mac, oid, property, index, None).await?;

    // Display results
    let obj_str = format!("{}:{}", object_type, instance);
    let prop_str = format!("{}", property);

    match format {
        OutputFormat::Table => {
            println!(
                "ReadRange {}  {}  count={}",
                obj_str, prop_str, ack.item_count
            );
            // Decode items from item_data
            let mut offset = 0;
            let mut item_num = 0;
            while offset < ack.item_data.len() {
                match decode_application_value(&ack.item_data, offset) {
                    Ok((value, next)) => {
                        item_num += 1;
                        println!(
                            "  [{}] {}",
                            item_num,
                            crate::output::format_property_value(&value)
                        );
                        offset = next;
                    }
                    Err(_) => {
                        let hex: String = ack.item_data[offset..]
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        println!("  [raw] {hex}");
                        break;
                    }
                }
            }
        }
        OutputFormat::Json => {
            let mut items = Vec::new();
            let mut offset = 0;
            while offset < ack.item_data.len() {
                match decode_application_value(&ack.item_data, offset) {
                    Ok((value, next)) => {
                        items.push(crate::output::format_property_value(&value));
                        offset = next;
                    }
                    Err(_) => {
                        let hex: String = ack.item_data[offset..]
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        items.push(format!("[raw: {hex}]"));
                        break;
                    }
                }
            }
            let json = serde_json::json!({
                "object": format!("{}:{}", object_type, instance),
                "property": format!("{}", property),
                "item_count": ack.item_count,
                "items": items,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
    }
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
                decode_and_format(value_bytes)
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
