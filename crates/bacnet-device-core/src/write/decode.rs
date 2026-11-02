//! What a write does with a value's octets before the object takes it: the
//! array-index gate, then the decoding into the value the object receives.
use super::{protocol_error, relinquish};
use alloc::vec::Vec;
use bacnet_objects::property_metadata::{
    Datatype, ValueShape, ValueType, check_write_array_index, object_value_type,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// What WriteProperty does with a value's octets before `object` takes it:
/// [`check_write_array_index`], then [`decode_write_property_value`] with the
/// property's value type on the object ([`object_value_type`]).
/// WriteProperty, WritePropertyMultiple, `write_local_encoded` and a
/// Command's or Channel's local writes share it, so each answers an index the
/// same way and ahead of the value's decoding. The typed local paths, which
/// have no octets, run the index check alone.
pub fn gate_and_decode_write(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    check_write_array_index(object, property, array_index)?;
    decode_write_property_value(
        property,
        object_value_type(object, property),
        array_index,
        bytes,
    )
}

/// PROPERTY / INVALID_DATA_ENCODING for an undecodable propertyValue payload.
fn invalid_data_encoding_error() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
}

/// Decode the complete propertyValue payload handed to an object write arm.
///
/// `value_type` is the property's datatype and shape on the target object:
/// its metadata row's, or the standard one for the object type where no
/// object is at hand. It decides how the octets reach the object.
///
/// Most values go through the generic loop: one application-tagged value
/// after another. A property that is a BACnetLIST written whole reaches the
/// object as a `PropertyValue::List` whatever its length, so an empty value
/// clears it and one element arrives as a list of one rather than as that
/// element alone. For a property that isn't a list, an empty value is
/// INVALID_DATA_ENCODING.
///
/// Values of several constructed datatypes reach their object as raw octets
/// in `PropertyValue::ApplicationData`, which it decodes itself, or one chunk
/// per array element. Each such object says what an empty value means; a
/// Setpoint_Reference with no octets, for one, holds no reference. Index 0
/// of an array, its size, is an Unsigned and takes the generic loop.
pub fn decode_write_property_value(
    property: PropertyIdentifier,
    value_type: Option<ValueType>,
    array_index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    let list = value_type.is_some_and(ValueType::is_list);
    match Decoding::of(property, value_type, array_index) {
        Decoding::Application => decode_application_values(bytes, list && array_index.is_none()),
        Decoding::Raw => Ok(PropertyValue::ApplicationData(bytes.to_vec())),
        Decoding::RawOrNull if bytes == [0x00] => Ok(PropertyValue::Null),
        Decoding::RawOrNull => Ok(PropertyValue::ApplicationData(bytes.to_vec())),
        Decoding::EventParameter => decode_event_parameter(bytes, list && array_index.is_none()),
        Decoding::Recipient => decode_recipient(bytes),
        Decoding::Elements(end) => decode_structured_array(bytes, array_index, end),
    }
}

/// How a write's octets reach the object, by the property's value type.
#[derive(Clone, Copy)]
enum Decoding {
    /// Application-tagged values, the generic loop.
    Application,
    /// The octets as they came.
    Raw,
    /// The octets as they came, except that an application NULL alone is
    /// `Null`, which the object refuses as the wrong datatype, so the write
    /// is judged as a relinquish (`relinquish`, #1417).
    RawOrNull,
    /// A BACnetEventParameter: the opaque form this stack writes for an
    /// algorithm it has no codec for reaches the object as its octet string;
    /// anything else takes the generic loop.
    EventParameter,
    /// One BACnetRecipient, checked here (#1124).
    Recipient,
    /// One chunk per array element, each split by this decoder (#1225).
    Elements(fn(&[u8], usize) -> Result<usize, Error>),
}

impl Decoding {
    fn of(property: PropertyIdentifier, value_type: Option<ValueType>, index: Option<u32>) -> Self {
        let Some(ValueType {
            datatype, shape, ..
        }) = value_type
        else {
            return Self::Application;
        };
        // Index 0 of an array is its size, an Unsigned.
        let element = index != Some(0);
        match datatype {
            Datatype::EventParameter => Self::EventParameter,
            // Whatever the index: the recipient and subscription lists, an
            // Effective_Period, a Lighting Output's Lighting_Command (#1263),
            // Color_Command (#1386), a Network Port's BDT (#1578), a foreign
            // device's BBMD address (#1825), a BBMD's global address behind
            // NAT (#940), and a Value_Source.
            Datatype::Destination
            | Datatype::EventNotificationSubscription
            | Datatype::DateRange
            | Datatype::LightingCommand
            | Datatype::ColorCommand
            | Datatype::BdtEntry
            | Datatype::HostNPort => Self::Raw,
            Datatype::ValueSource if matches!(shape, ValueShape::Single) => Self::Raw,
            Datatype::Recipient if matches!(shape, ValueShape::Single) => Self::Recipient,
            // A Loop's Setpoint_Reference (#1312).
            Datatype::SetpointReference => Self::RawOrNull,
            // The Schedule's arrays of constructed elements, an Access
            // Rights object's rule arrays, and Tags, which the objects split
            // and decode with the shared codecs (#1057, #1330, #1553).
            Datatype::DailySchedule
            | Datatype::SpecialEvent
            | Datatype::AccessRule
            | Datatype::NameValue
                if element =>
            {
                Self::Raw
            }
            Datatype::PortPermission if element => Self::Elements(|data, offset| {
                bacnet_encoding::constructed::decode_port_permission(data, offset)
                    .map(|(_, end)| end)
            }),
            Datatype::StageLimitValue if element => Self::Elements(|data, offset| {
                bacnet_encoding::constructed::decode_stage_limit_value(data, offset)
                    .map(|(_, end)| end)
            }),
            Datatype::DeviceObjectPropertyReference
            | Datatype::ObjectPropertyReference
            | Datatype::DeviceObjectReference => Self::reference(property, element),
            _ => Self::Application,
        }
    }

    /// A reference's decoding. Each reference datatype also serves
    /// properties whose objects take application values (a Global Group's
    /// members, Event_Algorithm_Inhibit_Ref, an Access Zone's doors), so
    /// these name the properties whose objects decode the reference
    /// themselves, after the handler has put any reference naming this
    /// device in its local form (#1122, #1313).
    fn reference(property: PropertyIdentifier, element: bool) -> Self {
        use PropertyIdentifier as P;
        match property {
            // A Schedule's, Channel's or Timer's references, the empty list
            // included (#1088, #1151); Staging targets (#1136) and a
            // Structured View's Subordinate_List (#2208).
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES | P::TARGET_REFERENCES | P::SUBORDINATE_LIST
                if element =>
            {
                Self::Raw
            }
            // Log_DeviceObjectProperty and an Averaging object's or Event
            // Enrollment's Object_Property_Reference (#1234, #1313).
            P::LOG_DEVICE_OBJECT_PROPERTY | P::OBJECT_PROPERTY_REFERENCE if element => {
                Self::RawOrNull
            }
            // The Loop and Pulse Converter references (#1312).
            P::CONTROLLED_VARIABLE_REFERENCE
            | P::MANIPULATED_VARIABLE_REFERENCE
            | P::INPUT_REFERENCE => Self::RawOrNull,
            _ => Self::Application,
        }
    }
}

/// The generic loop: application-tagged values back to back. A list's
/// encoding is its elements back to back, nothing at all for an empty one
/// (Clause 20.2.17), so `whole_list` gives a list of any length. The other
/// datatypes reaching this loop have no empty encoding, so no octets is
/// INVALID_DATA_ENCODING for them (Clause 15.9.1.3).
fn decode_application_values(bytes: &[u8], whole_list: bool) -> Result<PropertyValue, Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(bytes, offset)
                .map_err(|_| invalid_data_encoding_error())?;
        values.push(value);
        offset = new_offset;
    }
    if whole_list {
        return Ok(PropertyValue::List(values));
    }
    match values.len() {
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

/// An Event_Parameters value: the opaque form as its octet string, anything
/// else through the generic loop.
fn decode_event_parameter(bytes: &[u8], whole_list: bool) -> Result<PropertyValue, Error> {
    use bacnet_types::constructed::BACnetEventParameter;

    if !bytes.starts_with(&[0xfe, 0xff]) {
        return decode_application_values(bytes, whole_list);
    }
    match bacnet_encoding::constructed::decode_event_parameter(bytes, 0) {
        Ok((BACnetEventParameter::Opaque { tag, data }, consumed))
            if tag == u8::MAX && consumed == bytes.len() =>
        {
            Ok(PropertyValue::OctetString(data))
        }
        _ => Err(invalid_data_encoding_error()),
    }
}

/// An Audit_Notification_Recipient value, checked as one BACnetRecipient and
/// passed on as its octets.
fn decode_recipient(bytes: &[u8]) -> Result<PropertyValue, Error> {
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
    Ok(PropertyValue::ApplicationData(bytes.to_vec()))
}

fn decode_structured_array(
    bytes: &[u8],
    array_index: Option<u32>,
    decode: fn(&[u8], usize) -> Result<usize, Error>,
) -> Result<PropertyValue, Error> {
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

#[cfg(test)]
#[path = "decode_equivalence_tests.rs"]
mod equivalence_tests;
