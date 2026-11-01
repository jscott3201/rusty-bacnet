//! What a write does with a value's octets before the object takes it: the
//! array-index gate, then the decoding into the value the object receives.
use super::{protocol_error, relinquish};
use alloc::vec::Vec;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// At the existing indexed-write gate, effective absence takes precedence over
/// array classification (local error-order policy for Clauses 15.9/15.10).
pub fn check_write_array_index(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<(), Error> {
    if index.is_none() {
        return Ok(());
    }
    if !object
        .property_metadata()
        .iter()
        .any(|row| row.property_identifier == property)
    {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ));
    }
    if !object.is_array_property(property) {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }
    Ok(())
}

/// What WriteProperty does with a value's octets before `object` takes it:
/// [`check_write_array_index`], then [`decode_write_property_value`] with the
/// object's list classification. WriteProperty, WritePropertyMultiple,
/// `write_local_encoded` and a Command's or Channel's local writes share it,
/// so each answers an index the same way and ahead of the value's decoding.
/// The typed local paths, which have no octets, run the index check alone.
pub fn gate_and_decode_write(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    check_write_array_index(object, property, array_index)?;
    decode_write_property_value(
        property,
        array_index,
        object.is_list_property(property),
        bytes,
    )
}

/// PROPERTY / INVALID_DATA_ENCODING for an undecodable propertyValue payload.
fn invalid_data_encoding_error() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
}

/// Decode the complete propertyValue payload handed to an object write arm.
///
/// A few properties come first and reach their object as it decodes them
/// itself: raw octets in `PropertyValue::ApplicationData` (Recipient_List,
/// Subscribed_Recipients, List_Of_Object_Property_References and the other
/// cases below), or one chunk per array element. Each such arm says what an
/// empty value means; a Setpoint_Reference with no octets, for one, holds no
/// reference.
///
/// Everything else goes through the generic loop. There, `list` says
/// whether the target object holds `property` as a BACnetLIST
/// ([`BACnetObject::is_list_property`]). Such a property written whole
/// reaches the object as a `PropertyValue::List` whatever its length, so an
/// empty value clears it and one element arrives as a list of one rather
/// than as that element alone. For a property that isn't a list, an empty
/// value is INVALID_DATA_ENCODING.
///
/// [`BACnetObject::is_list_property`]: bacnet_objects::traits::BACnetObject::is_list_property
pub fn decode_write_property_value(
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
        return decode_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_port_permission(data, offset).map(|(_, end)| end)
        });
    }
    if array_index != Some(0) && property == PropertyIdentifier::STAGES {
        return decode_structured_array(bytes, array_index, |data, offset| {
            bacnet_encoding::constructed::decode_stage_limit_value(data, offset).map(|(_, end)| end)
        });
    }
    // Staging targets reach the object as raw reference bytes too, decoded
    // by the same helpers, once the handler has put those naming this device
    // in their local form (#1136, #1313); index 0, the array size, stays an
    // Unsigned.
    if array_index != Some(0) && property == PropertyIdentifier::TARGET_REFERENCES {
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

fn decode_structured_array<F>(
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
