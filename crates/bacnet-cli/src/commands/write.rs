//! Write commands: WriteProperty (WP) and WritePropertyMultiple (WPM).

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_ops::destination::Destination;
use bacnet_ops::parse;
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

/// The target and value of a single-property write.
pub struct WritePropertyArgs {
    /// Type of the object to write.
    pub object_type: ObjectType,
    /// Instance number of the object to write.
    pub instance: u32,
    /// Property to write.
    pub property: PropertyIdentifier,
    /// Array index within the property, if any.
    pub index: Option<u32>,
    /// Value to write.
    pub value: PropertyValue,
    /// Write priority (1-16), if any.
    pub priority: Option<u8>,
}

/// One property write inside a WritePropertyMultiple object entry:
/// property, array index, value and priority.
pub type PropertyWrite = (PropertyIdentifier, Option<u32>, PropertyValue, Option<u8>);

/// All property writes for one object: object type, instance and its property writes.
pub type ObjectWrites = (ObjectType, u32, Vec<PropertyWrite>);

/// Parse `writem`'s specs: each object specifier is followed by its
/// comma-separated `property=value[@priority]` writes.
pub fn parse_write_multiple(specs: &[String]) -> Result<Vec<ObjectWrites>, String> {
    let mut words = specs.iter();
    let mut objects = Vec::new();
    while let Some(object) = words.next() {
        let (object_type, instance) = parse::parse_object_specifier(object)?;
        let writes = words
            .next()
            .ok_or_else(|| format!("expected property=value after {object}"))?;
        let props = writes
            .split(',')
            .map(|pair| {
                let pair = pair.trim();
                let (property, value) = pair
                    .split_once('=')
                    .ok_or_else(|| format!("expected 'property=value', got '{pair}'"))?;
                let (property, index) = parse::parse_property(property)?;
                let (value, priority) =
                    parse::parse_value_for_with_priority(object_type, property, index, value)?;
                Ok((property, index, value, priority))
            })
            .collect::<Result<_, String>>()?;
        objects.push((object_type, instance, props));
    }
    Ok(objects)
}

/// Write a single property value.
pub async fn write_property_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    args: WritePropertyArgs,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let WritePropertyArgs {
        object_type,
        instance,
        property,
        index,
        value,
        priority,
    } = args;
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let encoded = encode_value(&value)?;

    to.write_property(client, oid, property, index, encoded, priority)
        .await?;

    output::print_success("OK", format);
    Ok(())
}

/// Write multiple properties to one or more objects.
pub async fn write_property_multiple_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    specs: Vec<ObjectWrites>,
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

    to.write_property_multiple(client, access_specs).await?;

    output::print_success("OK", format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn writem_specs_are_objects_each_with_its_writes() {
        let specs = parse_write_multiple(&words("av:1 pv=72.5@8,ol[2]=3 bv:2 pv=active")).unwrap();
        assert_eq!(specs.len(), 2);
        let (object_type, instance, writes) = &specs[0];
        assert_eq!((*object_type, *instance), (ObjectType::ANALOG_VALUE, 1));
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].0, PropertyIdentifier::PRESENT_VALUE);
        assert_eq!(writes[0].3, Some(8));
        assert_eq!(writes[1].1, Some(2));
        assert_eq!(specs[1].0, ObjectType::BINARY_VALUE);
    }

    #[test]
    fn a_malformed_writem_spec_is_refused() {
        for (text, error) in [
            ("av:1", "expected property=value after av:1"),
            ("av:1 pv", "expected 'property=value', got 'pv'"),
            ("nope:1 pv=1", ""),
        ] {
            let refused = parse_write_multiple(&words(text)).unwrap_err();
            assert!(refused.contains(error), "{text}: {refused}");
        }
    }
}
