//! Write commands: WriteProperty (WP) and WritePropertyMultiple (WPM).

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use crate::output::{self, OutputFormat};

/// Encode a `PropertyValue` into raw application-tagged bytes.
fn encode_value(value: &PropertyValue) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, value)?;
    Ok(buf.to_vec())
}

/// Write a single property value.
#[allow(clippy::too_many_arguments)]
pub async fn write_property_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let encoded = encode_value(&value)?;

    client
        .write_property(mac, oid, property, index, encoded, priority)
        .await?;

    output::print_success("OK", format);
    Ok(())
}

/// Write multiple properties to one or more objects.
#[allow(clippy::type_complexity)]
pub async fn write_property_multiple_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    specs: Vec<(
        ObjectType,
        u32,
        Vec<(PropertyIdentifier, Option<u32>, PropertyValue, Option<u8>)>,
    )>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let access_specs: Vec<WriteAccessSpecification> = specs
        .into_iter()
        .map(|(obj_type, instance, props)| {
            let oid = ObjectIdentifier::new(obj_type, instance)?;
            let prop_values = props
                .into_iter()
                .map(|(prop, idx, pv, priority)| {
                    let encoded = encode_value(&pv)
                        .map_err(|e| bacnet_types::error::Error::Encoding(e.to_string()))?;
                    Ok(BACnetPropertyValue {
                        property_identifier: prop,
                        property_array_index: idx,
                        value: encoded,
                        priority,
                    })
                })
                .collect::<Result<Vec<_>, bacnet_types::error::Error>>()?;
            Ok(WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: prop_values,
            })
        })
        .collect::<Result<Vec<_>, bacnet_types::error::Error>>()?;

    client.write_property_multiple(mac, access_specs).await?;

    output::print_success("OK", format);
    Ok(())
}
