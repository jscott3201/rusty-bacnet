//! Before/after table test for #2177: the decoding match
//! `decode_write_property_value` held before it read the property's value
//! type, frozen here, against the value-type decoding. Equal for every
//! catalogue object (with the B/IP and B/IPv6 Network Ports) and every
//! property, named or numbered up to 1023, at every index shape, over a set
//! of payloads; and again by the standard value type alone, as the durable
//! writes decode before any object is at hand.

use super::decode_write_property_value;
use crate::write::relinquish;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::property_metadata::{ValueType, object_value_type, standard_value_type};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

fn invalid_data_encoding_error() -> Error {
    super::protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
}

/// Every property: 0..=1023 and each named one above.
fn properties() -> impl Iterator<Item = PropertyIdentifier> {
    (0..=1023).map(PropertyIdentifier::from_raw).chain(
        PropertyIdentifier::ALL_NAMED
            .iter()
            .map(|&(_, property)| property)
            .filter(|property| property.to_raw() > 1023),
    )
}

const INDEXES: [Option<u32>; 4] = [None, Some(0), Some(1), Some(2)];

/// Payloads that take each branch: empty, a lone NULL, one and two
/// application values, a context-tagged constructed value, a whole and an
/// empty opaque event parameter, an encoded BACnetPortPermission and
/// BACnetStageLimitValue, a device Recipient (valid, past the instance
/// range, and with trailing octets), a well-formed address Recipient, and
/// octets that decode as nothing.
fn payloads() -> Vec<Vec<u8>> {
    vec![
        vec![],
        vec![0x00],
        vec![0x21, 0x05],
        vec![0x21, 0x05, 0x91, 0x01],
        vec![0x0e, 0x21, 0x05, 0x0f],
        vec![0xfe, 0xff, 0x01, 0x02, 0x03, 0xff, 0xff],
        vec![0xfe, 0xff, 0xff, 0xff],
        vec![0x09, 0x01, 0x19, 0x01],
        vec![0x09, 0x01, 0x19, 0x01, 0x09, 0x02, 0x19, 0x00],
        vec![
            0x44, 0x41, 0x20, 0x00, 0x00, 0x82, 0x07, 0x80, 0x44, 0x40, 0xa0, 0x00, 0x00,
        ],
        vec![0x0c, 0x02, 0x00, 0x00, 0x07],
        vec![0x0c, 0x02, 0x3f, 0xff, 0xff],
        vec![0x0c, 0x02, 0x00, 0x00, 0x07, 0x21, 0x05],
        vec![0x1e, 0x21, 0x01, 0x61, 0x07, 0x1f],
        vec![0xff, 0xff],
    ]
}

/// The payloads reach the success arms the comparison relies on: the opaque
/// event parameter, and a Recipient by address and by device; trailing
/// octets after a Recipient are refused.
#[test]
fn the_payloads_reach_each_decoding_arm() {
    use PropertyIdentifier as P;
    let decode = |property, bytes: &[u8]| {
        let value_type = standard_value_type(ObjectType::DEVICE, property);
        decode_write_property_value(property, value_type, None, bytes)
    };
    for opaque in [
        &[0xfe, 0xff, 0x01, 0x02, 0x03, 0xff, 0xff][..],
        &[0xfe, 0xff, 0xff, 0xff],
    ] {
        assert!(
            matches!(
                decode(P::EVENT_PARAMETERS, opaque),
                Ok(PropertyValue::OctetString(_))
            ),
            "{opaque:02x?}"
        );
    }
    for recipient in [
        &[0x0c, 0x02, 0x00, 0x00, 0x07][..],
        &[0x1e, 0x21, 0x01, 0x61, 0x07, 0x1f],
    ] {
        assert_eq!(
            decode(P::AUDIT_NOTIFICATION_RECIPIENT, recipient).unwrap(),
            PropertyValue::ApplicationData(recipient.to_vec())
        );
    }
    assert!(
        decode(
            P::AUDIT_NOTIFICATION_RECIPIENT,
            &[0x0c, 0x02, 0x00, 0x00, 0x07, 0x21, 0x05]
        )
        .is_err()
    );
}

/// One decode's answer, comparable.
fn answer(result: Result<PropertyValue, Error>) -> String {
    format!("{result:?}")
}

#[test]
fn value_type_decoding_matches_the_old_match_on_every_catalogue_row() {
    let mut objects = bacnet_objects::catalogue::representatives();
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let payloads = payloads();
    let mut differences = Vec::new();
    for object in &objects {
        let object: &dyn BACnetObject = &**object;
        for property in properties() {
            let value_type = object_value_type(object, property);
            compare(property, value_type, &payloads, &mut differences, || {
                format!("{:?}", object.object_identifier())
            });
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn value_type_decoding_matches_the_old_match_on_every_standard_type() {
    let payloads = payloads();
    let mut differences = Vec::new();
    for object_type in (0..=64).chain([128, 1023]).map(ObjectType::from_raw) {
        for property in properties() {
            let value_type = standard_value_type(object_type, property);
            compare(property, value_type, &payloads, &mut differences, || {
                format!("{object_type:?}")
            });
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

fn compare(
    property: PropertyIdentifier,
    value_type: Option<ValueType>,
    payloads: &[Vec<u8>],
    differences: &mut Vec<String>,
    target: impl Fn() -> String,
) {
    let list = value_type.is_some_and(ValueType::is_list);
    for index in INDEXES {
        for bytes in payloads {
            let old = answer(old_decode(property, index, list, bytes));
            let new = answer(decode_write_property_value(
                property, value_type, index, bytes,
            ));
            if old != new {
                differences.push(format!(
                    "{} {property:?} {index:?} {bytes:02x?}: was {old}, now {new}",
                    target()
                ));
            }
        }
    }
}

/// `decode_write_property_value` before #2177 part 2, as it was, keyed on
/// the property and told only whether the object held it as a list.
fn old_decode(
    property: PropertyIdentifier,
    array_index: Option<u32>,
    list: bool,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    if property == PropertyIdentifier::EVENT_PARAMETERS && bytes.starts_with(&[0xfe, 0xff]) {
        use bacnet_types::constructed::BACnetEventParameter;

        return match bacnet_encoding::constructed::decode_event_parameter(bytes, 0) {
            Ok((BACnetEventParameter::Opaque { tag, data }, consumed))
                if tag == u8::MAX && consumed == bytes.len() =>
            {
                Ok(PropertyValue::OctetString(data))
            }
            _ => Err(invalid_data_encoding_error()),
        };
    }
    // The Schedule decodes its list of references itself, an empty list
    // included (#1088), once the handler has put members naming this device
    // in their local form (#1122). So does a Channel, whose references are an
    // array: index 0, the array size, stays an Unsigned (#1151). A Lighting
    // Output decodes its Lighting_Command whole too, so a command keeps its
    // fields together and any other datatype is the object's to refuse
    // (#1263), and so do Color and Color Temperature their Color_Command
    // (#1386), and a Network Port its BDT (#1578), a foreign device's
    // BBMD address (#1825) and a BBMD's global address behind NAT (#940).
    if matches!(
        property,
        PropertyIdentifier::RECIPIENT_LIST
            | PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE
            | PropertyIdentifier::FD_BBMD_ADDRESS
            | PropertyIdentifier::BACNET_IP_GLOBAL_ADDRESS
            | PropertyIdentifier::SUBSCRIBED_RECIPIENTS
            | PropertyIdentifier::VALUE_SOURCE
            | PropertyIdentifier::EFFECTIVE_PERIOD
            | PropertyIdentifier::LIGHTING_COMMAND
            | PropertyIdentifier::COLOR_COMMAND
    ) || (property == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
        && array_index != Some(0))
    {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // The Schedule's arrays of constructed elements, an Access Rights
    // object's two rule arrays, and Tags, reach the object as raw bytes,
    // which it splits and decodes with the shared codecs; index 0, the array
    // size, stays an Unsigned (#1057, #1330, #1553).
    if array_index != Some(0)
        && matches!(
            property,
            PropertyIdentifier::WEEKLY_SCHEDULE
                | PropertyIdentifier::EXCEPTION_SCHEDULE
                | PropertyIdentifier::POSITIVE_ACCESS_RULES
                | PropertyIdentifier::NEGATIVE_ACCESS_RULES
                | PropertyIdentifier::TAGS
        )
    {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT {
        use bacnet_types::constructed::BACnetRecipient;

        // Clause 15.9/15.10 relinquishment is an operation, not a Recipient
        // value or disabled sentinel. The object owner must leave state intact.
        if let Ok((tag, end)) = bacnet_encoding::tags::decode_tag(bytes, 0)
            && tag.class == bacnet_encoding::tags::TagClass::Application
            && tag.number == bacnet_encoding::tags::app_tag::NULL
            && tag.length == 0
            && end == bytes.len()
        {
            return Ok(PropertyValue::Null);
        }
        // An address MAC past the bound does not decode, so it is refused
        // with the other undecodable values (#1124).
        let (recipient, consumed) = bacnet_encoding::constructed::decode_recipient(bytes, 0)
            .map_err(|_| invalid_data_encoding_error())?;
        if consumed != bytes.len() {
            return Err(invalid_data_encoding_error());
        }
        if let BACnetRecipient::Device(oid) = recipient
            && (oid.object_type() != ObjectType::DEVICE
                || oid.instance_number() == ObjectIdentifier::MAX_INSTANCE)
        {
            return Err(invalid_data_encoding_error());
        }
        // This validates the single Recipient value, not its deliverability.
        // Address routing and broadcast policy belong to the mutation owner.
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // Log_DeviceObjectProperty and an Averaging object's
    // Object_Property_Reference reach the object as raw reference bytes, which
    // it decodes with the shared device-reference helpers, after the handler
    // has put any reference naming this device in its local form; a Trend Log
    // Multiple's index 0, the array size, stays an Unsigned (#1234, #1313).
    // An application Null alone reaches it as Null, which it refuses as the
    // wrong datatype, so the write is judged as a relinquish (`relinquish`,
    // #1417).
    if array_index != Some(0)
        && matches!(
            property,
            PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY
                | PropertyIdentifier::OBJECT_PROPERTY_REFERENCE
        )
    {
        if bytes == [0x00] {
            return Ok(PropertyValue::Null);
        }
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // The Loop and Pulse Converter references reach the object as their raw
    // octets too, which it decodes with the shared codecs: an empty value is
    // a Setpoint_Reference holding no reference (#1312). An application Null
    // alone reaches the object as Null, judged as above (#1417).
    if matches!(
        property,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE
            | PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE
            | PropertyIdentifier::SETPOINT_REFERENCE
            | PropertyIdentifier::INPUT_REFERENCE
    ) {
        if bytes == [0x00] {
            return Ok(PropertyValue::Null);
        }
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    // A Notification Forwarder's Port_Filter goes one BACnetPortPermission
    // per chunk (#1225).
    if array_index != Some(0) && property == PropertyIdentifier::PORT_FILTER {
        return old_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_port_permission(data, offset).map(|(_, end)| end)
        });
    }
    if array_index != Some(0) && property == PropertyIdentifier::STAGES {
        return old_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_stage_limit_value(data, offset).map(|(_, end)| end)
        });
    }
    // Staging targets reach the object as raw reference bytes too, decoded
    // by the same helpers, once the handler has put those naming this device
    // in their local form (#1136, #1313), and so does a Structured View's
    // Subordinate_List, which CreateObject sets (#2208); index 0, the array
    // size, stays an Unsigned.
    if array_index != Some(0)
        && matches!(
            property,
            PropertyIdentifier::TARGET_REFERENCES | PropertyIdentifier::SUBORDINATE_LIST
        )
    {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(bytes, offset)
                .map_err(|_| invalid_data_encoding_error())?;
        values.push(value);
        offset = new_offset;
    }
    // A list's encoding is its elements back to back, nothing at all for an
    // empty one (Clause 20.2.17). The other datatypes reaching this loop have
    // no empty encoding, so no octets is INVALID_DATA_ENCODING for them
    // (Clause 15.9.1.3).
    if list && array_index.is_none() {
        return Ok(PropertyValue::List(values));
    }
    match values.len() {
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

fn old_structured_array<F>(
    bytes: &[u8],
    array_index: Option<u32>,
    mut decode: F,
) -> Result<PropertyValue, Error>
where
    F: FnMut(&[u8], usize) -> Result<usize, Error>,
{
    // A lone NULL is no element of these arrays. It reaches the object as
    // Null, which refuses it as the wrong datatype, so the write is judged
    // as a relinquish (`relinquish`) rather than as undecodable octets.
    if relinquish::is_null_octets(bytes) {
        return Ok(PropertyValue::Null);
    }
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if values.len() >= 10_000 {
            return Err(invalid_data_encoding_error());
        }
        let start = offset;
        offset = decode(bytes, offset).map_err(|_| invalid_data_encoding_error())?;
        if offset <= start || offset > bytes.len() {
            return Err(invalid_data_encoding_error());
        }
        values.push(PropertyValue::ApplicationData(
            bytes[start..offset].to_vec(),
        ));
    }
    if array_index.is_some() {
        if values.len() == 1 {
            return Ok(values.pop().unwrap());
        }
        return Err(invalid_data_encoding_error());
    }
    Ok(PropertyValue::List(values))
}
