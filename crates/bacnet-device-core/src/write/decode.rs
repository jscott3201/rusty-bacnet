//! What a write does with a value's octets before the object takes it: the
//! array-index gate, then the decoding into the value the object receives.
use super::protocol_error;
use alloc::vec::Vec;
use bacnet_objects::property_metadata::{
    Datatype, ValueShape, ValueType, check_write_array_index, object_value_type,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

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
    let value_type = object_value_type(object, property);
    // A property the object doesn't serve is UNKNOWN_PROPERTY, as its own
    // write answers, rather than a refusal of a datatype it never holds.
    #[cfg(not(feature = "structured-writes"))]
    if Decoding::of(property, value_type, array_index).is_structured()
        && !bacnet_objects::property_metadata::has_property(&object.property_metadata(), property)
    {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ));
    }
    decode_write_property_value(property, value_type, array_index, bytes)
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
/// element alone. A BACnetARRAY written whole with no octets is an array of
/// no elements, an empty `PropertyValue::List`, which its object takes or
/// refuses by its own rules (#2305). Any other empty value, an array
/// element's included, is INVALID_DATA_ENCODING.
///
/// Values of several constructed datatypes reach their object as raw octets
/// in `PropertyValue::ApplicationData`, which it decodes itself, or one chunk
/// per array element. Each such object says what an empty value means; a
/// Setpoint_Reference with no octets, for one, holds no reference. Index 0
/// of an array, its size, is an Unsigned and takes the generic loop.
///
/// Three datatypes take arms that call bacnet-encoding's structured
/// decoders only this path needs: a BACnetEventParameter (for its opaque
/// form), and the elements of a BACnetPortPermission or
/// BACnetStageLimitValue array. Without the `structured-writes` feature any
/// write of one of them is refused with PROPERTY / WRITE_ACCESS_DENIED, so
/// an image that never writes them doesn't link those decoders (#2210).
/// The metadata rows still call such a property writable, so a build
/// without the feature lists it as read-only in its PICS. A single
/// BACnetRecipient is checked either way: the Device's own write of its
/// Audit_Notification_Recipient links that decoder already.
pub fn decode_write_property_value(
    property: PropertyIdentifier,
    value_type: Option<ValueType>,
    array_index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    let whole = Whole::of(value_type, array_index);
    match Decoding::of(property, value_type, array_index) {
        Decoding::Application => decode_application_values(bytes, whole),
        Decoding::Raw => Ok(PropertyValue::ApplicationData(bytes.to_vec())),
        Decoding::RawOrNull if bytes == [0x00] => Ok(PropertyValue::Null),
        Decoding::RawOrNull => Ok(PropertyValue::ApplicationData(bytes.to_vec())),
        #[cfg(feature = "structured-writes")]
        Decoding::EventParameter => decode_event_parameter(bytes, whole),
        Decoding::Recipient => decode_recipient(bytes),
        #[cfg(feature = "structured-writes")]
        Decoding::Elements(element) => decode_structured_array(bytes, array_index, element),
        // The decode runs on a runtime datatype, so the linker keeps every
        // decoder an arm can call, whichever datatypes the image's objects
        // serve. Without the feature these arms call none.
        #[cfg(not(feature = "structured-writes"))]
        Decoding::EventParameter | Decoding::Elements(_) => Err(structured_write_refused()),
    }
}

/// PROPERTY / WRITE_ACCESS_DENIED for a write of a datatype this build
/// doesn't decode. The value may be of the right datatype, so neither
/// INVALID_DATA_TYPE nor DATATYPE_NOT_SUPPORTED fits; the device has chosen
/// not to take writes of the property. The opening text of Clauses 15.9 and
/// 15.10 lets an implementation restrict writes that way, answered with
/// WRITE_ACCESS_DENIED, the code their error tables give a property that
/// can't be written. Nor is it a refusal of the NULL's
/// datatype, so a lone NULL isn't judged a relinquish (`relinquish`).
#[cfg(not(feature = "structured-writes"))]
fn structured_write_refused() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
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
    /// One chunk per array element, each split by its datatype's decoder
    /// (#1225).
    Elements(
        #[cfg_attr(
            not(feature = "structured-writes"),
            expect(dead_code, reason = "refused without its decoders")
        )]
        Element,
    ),
}

/// An array element datatype whose elements [`Decoding::Elements`] splits.
#[derive(Clone, Copy)]
enum Element {
    /// A Notification Forwarder's BACnetPortPermission.
    PortPermission,
    /// A Staging object's BACnetStageLimitValue.
    StageLimitValue,
}

impl Decoding {
    /// Whether this decoding needs one of the structured decoders that the
    /// `structured-writes` feature links.
    #[cfg(not(feature = "structured-writes"))]
    fn is_structured(self) -> bool {
        matches!(self, Self::EventParameter | Self::Elements(_))
    }

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
            Datatype::PortPermission if element => Self::Elements(Element::PortPermission),
            Datatype::StageLimitValue if element => Self::Elements(Element::StageLimitValue),
            Datatype::DeviceObjectPropertyReference
            | Datatype::ObjectPropertyReference
            | Datatype::DeviceObjectReference => Self::reference(property, element),
            // `Datatype` is non-exhaustive in bacnet-types, so this arm is
            // required: every other datatype, and any added later, takes the
            // application loop.
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

/// What a write without an array index names, by the property's shape: the
/// generic loop's answer for a value of no elements or of one depends on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Whole {
    /// A BACnetLIST written whole: a list of any length.
    List,
    /// A BACnetARRAY written whole: no octets is an array of no elements.
    Array,
    /// One value, or one element of an array.
    Value,
}

impl Whole {
    fn of(value_type: Option<ValueType>, array_index: Option<u32>) -> Self {
        match value_type {
            Some(value_type) if array_index.is_none() && value_type.is_list() => Self::List,
            Some(value_type) if array_index.is_none() && value_type.is_array() => Self::Array,
            _ => Self::Value,
        }
    }
}

/// The generic loop: application-tagged values back to back. A list's
/// encoding, and an array's written whole, is its elements back to back,
/// nothing at all for an empty one (Clause 20.2.17), so a whole list is a
/// list of any length and a whole array of no octets an empty one. A
/// whole array of one element or more keeps the answer a value gets: its
/// element alone, or a list of them. The other datatypes reaching this loop
/// have no empty encoding, nor has an array element, so no octets is
/// INVALID_DATA_ENCODING for them (Clause 15.9.1.3).
fn decode_application_values(bytes: &[u8], whole: Whole) -> Result<PropertyValue, Error> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(bytes, offset)
                .map_err(|_| invalid_data_encoding_error())?;
        values.push(value);
        offset = new_offset;
    }
    if whole == Whole::List {
        return Ok(PropertyValue::List(values));
    }
    match values.len() {
        0 if whole == Whole::Array => Ok(PropertyValue::List(values)),
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

/// An Event_Parameters value: the opaque form as its octet string, anything
/// else through the generic loop.
#[cfg(feature = "structured-writes")]
fn decode_event_parameter(bytes: &[u8], whole: Whole) -> Result<PropertyValue, Error> {
    use bacnet_types::constructed::BACnetEventParameter;

    if !bytes.starts_with(&[0xfe, 0xff]) {
        return decode_application_values(bytes, whole);
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
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

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

#[cfg(feature = "structured-writes")]
fn decode_structured_array(
    bytes: &[u8],
    array_index: Option<u32>,
    element: Element,
) -> Result<PropertyValue, Error> {
    use bacnet_encoding::constructed::{decode_port_permission, decode_stage_limit_value};

    let decode = |data: &[u8], offset| match element {
        Element::PortPermission => decode_port_permission(data, offset).map(|(_, end)| end),
        Element::StageLimitValue => decode_stage_limit_value(data, offset).map(|(_, end)| end),
    };
    // A lone NULL is no element of these arrays. It reaches the object as
    // Null, which refuses it as the wrong datatype, so the write is judged
    // as a relinquish (`relinquish`) rather than as undecodable octets.
    if super::relinquish::is_null_octets(bytes) {
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

// The frozen decoding decodes all four structured datatypes, so it is
// compared with the build that has their decoders.
#[cfg(all(test, feature = "structured-writes"))]
#[path = "decode_equivalence_tests.rs"]
mod equivalence_tests;

#[cfg(test)]
#[path = "structured_writes_tests.rs"]
mod structured_writes_tests;

#[cfg(test)]
#[path = "empty_array_tests.rs"]
mod empty_array_tests;
