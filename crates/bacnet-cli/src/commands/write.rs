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
/// `property=value[@priority]` writes, which a comma separates (see
/// [`split_writes`] for which commas do).
pub fn parse_write_multiple(specs: &[String]) -> Result<Vec<ObjectWrites>, String> {
    let mut words = specs.iter();
    let mut objects = Vec::new();
    while let Some(object) = words.next() {
        let (object_type, instance) = parse::parse_object_specifier(object)?;
        let writes = words
            .next()
            .ok_or_else(|| format!("expected property=value after {object}"))?;
        let props = split_writes(writes)
            .into_iter()
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

/// Cut one object's writes word into its `property=value[@priority]`
/// pieces. A comma ends a write only when the text after it reads as a
/// property (a name or number with an optional `[index]`, as
/// `parse::parse_property` takes it) and then an `=`; any other comma
/// belongs to the value, so `object:ai,1` and `in-alarm,fault` stay whole.
/// A value that opens with a double quote runs to the next one, and a comma
/// inside it never ends a write, whatever follows it; that applies when the
/// closing quote ends the write or is followed by `,` or `@`, so a stray
/// quote elsewhere is plain text, as it was.
fn split_writes(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut scan = value_start(text, 0);
    while let Some(found) = text[scan..].find(',') {
        let comma = scan + found;
        if starts_write(&text[comma + 1..]) {
            pieces.push(&text[start..comma]);
            start = comma + 1;
            scan = value_start(text, start);
        } else {
            scan = comma + 1;
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// Where the commas of the write that begins at `start` may end it: past
/// its `=`, and past a quoted value's closing quote.
fn value_start(text: &str, start: usize) -> usize {
    let Some(eq) = text[start..].find('=') else {
        return start;
    };
    let value = start + eq + 1;
    let rest = &text[value..];
    let Some(quoted) = rest.trim_start().strip_prefix('"') else {
        return value;
    };
    let Some(close) = quoted.find('"') else {
        return value;
    };
    let after = &quoted[close + 1..];
    if after.is_empty() || after.starts_with([',', '@']) {
        text.len() - after.len()
    } else {
        value
    }
}

/// Whether `text`, the rest of the word after a comma, begins another
/// write: a property specifier up to the next `=`.
fn starts_write(text: &str) -> bool {
    text.split_once('=')
        .is_some_and(|(property, _)| parse::parse_property(property).is_ok())
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

    /// An object word in the coming BACnet RDF encoding's `type,instance`
    /// form names the same object, and so does a value: the comma ends a
    /// write only before a property and an `=` (#2482).
    #[test]
    fn writem_objects_take_the_comma_form_and_values_keep_their_commas() {
        assert_eq!(
            parse_write_multiple(&words("analog-value,1 pv=72.5@8,ol[2]=3 bv,2 pv=active")),
            parse_write_multiple(&words("av:1 pv=72.5@8,ol[2]=3 bv:2 pv=active"))
        );
        assert_eq!(
            parse_write_multiple(&words("nc:1 pv=object:ai,1")),
            parse_write_multiple(&words("nc:1 pv=object:ai:1"))
        );
        assert_eq!(
            parse_write_multiple(&words("nc:1 pv=object:ai,1,desc=x")),
            parse_write_multiple(&words("nc:1 pv=object:ai:1,desc=x"))
        );
    }

    fn pieces(text: &str) -> Vec<&str> {
        split_writes(text)
    }

    #[test]
    fn a_comma_ends_a_write_only_before_a_property_and_an_equals_sign() {
        assert_eq!(pieces("pv=1,desc=x"), ["pv=1", "desc=x"]);
        assert_eq!(pieces("pv=72.5@8,ol[2]=3"), ["pv=72.5@8", "ol[2]=3"]);
        assert_eq!(pieces("pv=1, 85=2"), ["pv=1", " 85=2"]);
        // A comma inside a value: an object, bits, an enumeration list, a
        // date form.
        for whole in [
            "pv=object:ai,1",
            "pv=object:128,5",
            "ef=in-alarm,fault",
            "ee=to-offnormal,to-normal",
            "pv=2026-10-19,Mon",
            "st=*,*,1",
            "desc=a,b,c",
            "desc=a,",
            "desc=,x",
        ] {
            assert_eq!(pieces(whole), [whole], "{whole}");
        }
        // `fan` is no property, so the text after it stays in the value.
        assert!(parse::parse_property("fan").is_err());
        assert_eq!(pieces("desc=heat, fan=auto"), ["desc=heat, fan=auto"]);
        // `mode` is one, so the same sentence splits there.
        assert!(parse::parse_property("mode").is_ok());
        assert_eq!(pieces("desc=heat, mode=auto"), ["desc=heat", " mode=auto"]);
    }

    #[test]
    fn a_quoted_value_is_never_split() {
        assert_eq!(
            pieces("desc=\"heat, mode=auto\",pv=1"),
            ["desc=\"heat, mode=auto\"", "pv=1"]
        );
        assert_eq!(pieces("desc=\"a, pv=1\"@8"), ["desc=\"a, pv=1\"@8"]);
        assert_eq!(
            pieces("on=\"x,y\", desc=\"p,q\""),
            ["on=\"x,y\"", " desc=\"p,q\""]
        );
        // An unbalanced or mid-value quote is plain text, as before.
        assert_eq!(pieces("desc=\"a,on=\"b\""), ["desc=\"a", "on=\"b\""]);
        assert_eq!(pieces("desc=\"a, mode=auto"), ["desc=\"a", " mode=auto"]);
        let (_, _, writes) =
            &parse_write_multiple(&words("ai:1 desc=\"heat,mode=auto\"")).unwrap()[0];
        assert_eq!(
            writes[0].2,
            PropertyValue::CharacterString("heat,mode=auto".into())
        );
    }

    /// The splitter this replaced: every comma ended a write.
    fn split_at_every_comma(specs: &[String]) -> Result<Vec<ObjectWrites>, String> {
        let mut words = specs.iter();
        let mut objects = Vec::new();
        while let Some(object) = words.next() {
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let writes = words.next().ok_or("missing writes")?;
            let props = writes
                .split(',')
                .map(|pair| {
                    let (property, value) = pair.trim().split_once('=').ok_or("no =")?;
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

    use proptest::prelude::*;

    proptest! {
        /// Whatever the old splitter read, the new one reads the same.
        #[test]
        fn every_writem_valid_before_splits_into_the_same_writes(
            pairs in proptest::collection::vec(
                prop_oneof![
                    Just("pv=72.5".to_string()),
                    Just("pv=10@8".to_string()),
                    Just("ol[2]=3".to_string()),
                    Just("desc=\"Zone 3\"".to_string()),
                    Just("description=Zone".to_string()),
                    Just("85=4.5@16".to_string()),
                    Just("oos=true".to_string()),
                    "[a-z0-9\\[\\]=@ .\":-]{0,12}",
                ],
                1..5,
            ),
            join in prop_oneof![Just(","), Just(", "), Just(" ,")],
        ) {
            let specs = vec!["av:1".to_string(), pairs.join(join)];
            let before = split_at_every_comma(&specs);
            if before.is_ok() {
                prop_assert_eq!(parse_write_multiple(&specs), before);
            }
        }
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
