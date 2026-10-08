//! Typed read results for constructed properties: the collections the
//! binding also takes as typed values (#1310), and the other constructed
//! lists, arrays and single values the stack has a codec for (#1344, #1345).
//!
//! Nothing in a list of constructed elements marks where one element ends,
//! so the generic decoder in `read_value` can't split it. For the properties
//! [`element`] names, the element's datatype is known, so the value is split
//! with that datatype's codec from `bacnet_encoding::constructed` (or, for a
//! Group's Present_Value, the ReadPropertyMultiple ACK decoder). Each element
//! keeps its own octets, so the value encodes back to exactly what was read,
//! and its Python form (see `constructed_py`) is built from those octets
//! when `.value` is asked for. A value that doesn't decode as the expected
//! elements, to the last octet, is left to the generic decoder.

use bacnet_encoding::constructed::{
    decode_access_rule, decode_action_list, decode_authentication_factor_format,
    decode_authentication_policy, decode_calendar_entry, decode_cov_subscription,
    decode_daily_schedule, decode_date_range, decode_destination,
    decode_device_object_property_reference, decode_device_object_reference, decode_name_value,
    decode_port_permission, decode_prescale, decode_property_access_result,
    decode_read_access_specification, decode_recipient, decode_scale, decode_special_event,
    decode_stage_limit_value, decode_value_source,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_encoding::tags;
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK};
use bacnet_types::constructed::BACnetAccessRule;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use super::constructed_py::Decoded;
use super::PyPropertyValue;

/// The constructed production a typed read splits a value into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Element {
    /// A Tags element as a name and an optional primitive PropertyValue.
    NameValue,
    /// A Recipient_List destination, as a `Destination` mapping.
    Destination,
    /// A Port_Filter element, as a `(port_id, enabled)` pair.
    PortPermission,
    /// A Group member, in the `read_property_multiple` spec shape.
    ReadAccessSpecification,
    /// A Group's Present_Value result, in the `read_property_multiple`
    /// result shape.
    ReadAccessResult,
    /// A Command's action list, as a list of `ActionCommand` mappings.
    ActionList,
    /// An object reference: an `ObjectIdentifier`, or a `(device, object)`
    /// pair when it names a device.
    DeviceObjectReference,
    /// A Supported_Formats element: the format type, or a `(format_type,
    /// vendor_id, vendor_format)` triple when it carries vendor members.
    AuthenticationFactorFormat,
    /// An Authentication_Policy_List element, as the `(entries,
    /// order_enforced, timeout)` triple `add_access_point` takes (#1325).
    AuthenticationPolicy,
    /// A Stages element, as a `(limit, values, deadband)` triple.
    StageLimitValue,
    /// An Access Rights rule, as an `AccessRule` mapping.
    AccessRule,
    /// A property reference, as a `DeviceObjectPropertyReference` mapping.
    DeviceObjectPropertyReference,
    /// A Global Group member's result: its reference, value and error.
    PropertyAccessResult,
    /// An audit recipient, as an `AuditRecipientInput` mapping.
    Recipient,
    /// A Weekly_Schedule day, as `[(time, value), ...]`.
    DailySchedule,
    /// An Exception_Schedule event, as a mapping.
    SpecialEvent,
    /// A Date_List entry, as a mapping whose `kind` names the alternative.
    CalendarEntry,
    /// An Effective_Period, as `(start_date, end_date)`.
    DateRange,
    /// A timestamp, as a `BACnetTimeStamp`.
    TimeStamp,
    /// An Active_COV_Subscriptions element, as a mapping.
    CovSubscription,
    /// A command source: `None`, an object reference, or an address mapping
    /// with `kind` `"address"`.
    ValueSource,
    /// An Accumulator's Scale: a `float` for a float scale, an `int` for a
    /// power-of-ten scale.
    Scale,
    /// An Accumulator's Prescale, as `(multiplier, modulo_divide)`.
    Prescale,
    /// A Device_Address_Binding element (#1369), as a mapping of
    /// `device_identifier`, `network_number` and `mac_address`.
    AddressBinding,
}

/// How a property holds its elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A BACnetARRAY or BACnetLIST: a whole read is a list of elements, and
    /// an indexed read one element.
    Collection,
    /// One value: a whole read is the element itself.
    Single,
}

/// The element production of `property` on `object_type`, and how the
/// property holds it, for the properties the binding reads as typed values.
pub(crate) fn element(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> Option<(Element, Shape)> {
    type O = ObjectType;
    type P = PropertyIdentifier;
    use Shape::{Collection, Single};
    Some(match (object_type, property) {
        (O::NOTIFICATION_CLASS | O::NOTIFICATION_FORWARDER, P::RECIPIENT_LIST) => {
            (Element::Destination, Collection)
        }
        (O::NOTIFICATION_FORWARDER, P::PORT_FILTER) => (Element::PortPermission, Collection),
        (O::GROUP, P::LIST_OF_GROUP_MEMBERS) => (Element::ReadAccessSpecification, Collection),
        (O::GROUP, P::PRESENT_VALUE) => (Element::ReadAccessResult, Collection),
        (O::COMMAND, P::ACTION) => (Element::ActionList, Collection),
        (O::ACCESS_DOOR, P::DOOR_MEMBERS)
        | (O::ACCESS_POINT, P::ACCESS_DOORS)
        | (O::STAGING, P::TARGET_REFERENCES)
        | (O::ACCESS_ZONE, P::ENTRY_POINTS | P::EXIT_POINTS)
        | (O::ACCESS_USER, P::CREDENTIALS | P::MEMBERS | P::MEMBER_OF)
        | (O::LIFE_SAFETY_POINT | O::LIFE_SAFETY_ZONE, P::MEMBER_OF)
        | (O::LIFE_SAFETY_ZONE, P::ZONE_MEMBERS) => (Element::DeviceObjectReference, Collection),
        (O::ACCESS_RIGHTS, P::ACCOMPANIMENT)
        | (O::ACCESS_POINT, P::ACCESS_EVENT_CREDENTIAL)
        | (O::LIFT | O::ESCALATOR, P::ENERGY_METER_REF) => (Element::DeviceObjectReference, Single),
        (O::CREDENTIAL_DATA_INPUT, P::SUPPORTED_FORMATS) => {
            (Element::AuthenticationFactorFormat, Collection)
        }
        (O::ACCESS_POINT, P::AUTHENTICATION_POLICY_LIST) => {
            (Element::AuthenticationPolicy, Collection)
        }
        (O::STAGING, P::STAGES) => (Element::StageLimitValue, Collection),
        (O::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES | P::NEGATIVE_ACCESS_RULES) => {
            (Element::AccessRule, Collection)
        }
        (O::GLOBAL_GROUP, P::GROUP_MEMBERS)
        | (O::SCHEDULE | O::CHANNEL, P::LIST_OF_OBJECT_PROPERTY_REFERENCES)
        | (O::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY) => {
            (Element::DeviceObjectPropertyReference, Collection)
        }
        (O::TREND_LOG, P::LOG_DEVICE_OBJECT_PROPERTY)
        | (O::AVERAGING | O::EVENT_ENROLLMENT, P::OBJECT_PROPERTY_REFERENCE) => {
            (Element::DeviceObjectPropertyReference, Single)
        }
        (O::GLOBAL_GROUP, P::PRESENT_VALUE) => (Element::PropertyAccessResult, Collection),
        (O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT) => (Element::Recipient, Single),
        (O::SCHEDULE, P::WEEKLY_SCHEDULE) => (Element::DailySchedule, Collection),
        (O::SCHEDULE, P::EXCEPTION_SCHEDULE) => (Element::SpecialEvent, Collection),
        (O::SCHEDULE, P::EFFECTIVE_PERIOD) => (Element::DateRange, Single),
        (O::CALENDAR, P::DATE_LIST) => (Element::CalendarEntry, Collection),
        (O::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS) => (Element::CovSubscription, Collection),
        (O::DEVICE, P::DEVICE_ADDRESS_BINDING) => (Element::AddressBinding, Collection),
        (O::ACCUMULATOR, P::SCALE) => (Element::Scale, Single),
        (O::ACCUMULATOR, P::PRESCALE) => (Element::Prescale, Single),
        // Every object type that has these properties gives them one datatype.
        (_, P::TAGS) => (Element::NameValue, Collection),
        (_, P::EVENT_TIME_STAMPS | P::COMMAND_TIME_ARRAY) => (Element::TimeStamp, Collection),
        (_, P::LAST_COMMAND_TIME) => (Element::TimeStamp, Single),
        (_, P::VALUE_SOURCE_ARRAY) => (Element::ValueSource, Collection),
        (_, P::VALUE_SOURCE) => (Element::ValueSource, Single),
        _ => return None,
    })
}

/// The typed form of a read of `property` on `object_type`, or `None` when
/// the property has no element production here or `octets` don't decode as
/// its elements; the generic decoder then takes the value.
///
/// A whole read of a collection is a list of the elements, an indexed read
/// one element, and a read of a single value that element. An empty value
/// and index 0 (an array's size) are left to the generic decoder, which
/// already gives them their shape.
pub(crate) fn decode(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> Option<PyPropertyValue> {
    let (element, shape) = element(object_type, property)?;
    let ends = element.split(octets)?;
    let one = || {
        Some(PyPropertyValue::constructed(
            PropertyValue::ApplicationData(octets.to_vec()),
            element,
        ))
    };
    match (shape, array_index) {
        (Shape::Collection, None) if !ends.is_empty() => {
            let mut start = 0;
            let elements = ends
                .into_iter()
                .map(|end| {
                    let element = PropertyValue::ApplicationData(octets[start..end].to_vec());
                    start = end;
                    element
                })
                .collect();
            Some(PyPropertyValue::constructed(
                PropertyValue::List(elements),
                element,
            ))
        }
        (Shape::Collection, Some(index)) if index != 0 && ends.len() == 1 => one(),
        (Shape::Single, None) if ends.len() == 1 => one(),
        _ => None,
    }
}

impl Element {
    /// Every element production, each once; a new one goes here too, so
    /// that [`Self::from_tag`] knows it.
    const ALL: [Self; 24] = [
        Self::NameValue,
        Self::Destination,
        Self::PortPermission,
        Self::ReadAccessSpecification,
        Self::ReadAccessResult,
        Self::ActionList,
        Self::DeviceObjectReference,
        Self::AuthenticationFactorFormat,
        Self::AuthenticationPolicy,
        Self::StageLimitValue,
        Self::AccessRule,
        Self::DeviceObjectPropertyReference,
        Self::PropertyAccessResult,
        Self::Recipient,
        Self::DailySchedule,
        Self::SpecialEvent,
        Self::CalendarEntry,
        Self::DateRange,
        Self::TimeStamp,
        Self::CovSubscription,
        Self::ValueSource,
        Self::Scale,
        Self::Prescale,
        Self::AddressBinding,
    ];

    /// The element whose [`Self::tag`] is `tag`.
    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|element| element.tag() == tag)
    }

    /// Whether `octets` are exactly one element of this production.
    pub(crate) fn is_one(self, octets: &[u8]) -> bool {
        self.split(octets).is_some_and(|ends| ends.len() == 1)
    }

    /// The `PropertyValue.tag` of one element.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            Self::NameValue => "name_value",
            Self::Destination => "destination",
            Self::PortPermission => "port_permission",
            Self::ReadAccessSpecification => "read_access_specification",
            Self::ReadAccessResult => "read_access_result",
            Self::ActionList => "action_list",
            Self::DeviceObjectReference => "device_object_reference",
            Self::AuthenticationFactorFormat => "authentication_factor_format",
            Self::AuthenticationPolicy => "authentication_policy",
            Self::StageLimitValue => "stage_limit_value",
            Self::AccessRule => "access_rule",
            Self::DeviceObjectPropertyReference => "device_object_property_reference",
            Self::PropertyAccessResult => "property_access_result",
            Self::Recipient => "recipient",
            Self::DailySchedule => "daily_schedule",
            Self::SpecialEvent => "special_event",
            Self::CalendarEntry => "calendar_entry",
            Self::DateRange => "date_range",
            Self::TimeStamp => "timestamp",
            Self::CovSubscription => "cov_subscription",
            Self::ValueSource => "value_source",
            Self::Scale => "scale",
            Self::Prescale => "prescale",
            Self::AddressBinding => "address_binding",
        }
    }

    /// The Python form of one element's octets. They decoded when the read
    /// was split, so they decode again here; if they somehow didn't, the
    /// octets come back as `bytes` rather than raising.
    pub(crate) fn to_py(self, py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
        match self.decode_at(octets, 0) {
            Ok((decoded, _)) => decoded.into_python(py),
            Err(_) => Ok(PyBytes::new(py, octets).into_any().unbind()),
        }
    }

    /// Where each element of `octets` ends, or `None` when one doesn't
    /// decode.
    fn split(self, octets: &[u8]) -> Option<Vec<usize>> {
        let mut ends = Vec::new();
        let mut offset = 0;
        while offset < octets.len() {
            let (_, end) = self.decode_at(octets, offset).ok()?;
            if end <= offset {
                return None;
            }
            ends.push(end);
            offset = end;
        }
        Some(ends)
    }

    /// Decode the element at `offset`, returning it and the offset past it.
    fn decode_at(self, octets: &[u8], offset: usize) -> Result<(Decoded, usize), Error> {
        /// Wrap a decoded value in `variant`.
        fn with<T>(
            decoded: Result<(T, usize), Error>,
            variant: fn(T) -> Decoded,
        ) -> Result<(Decoded, usize), Error> {
            decoded.map(|(value, end)| (variant(value), end))
        }
        match self {
            Self::NameValue => with(decode_name_value(octets, offset), Decoded::NameValue),
            Self::Destination => with(decode_destination(octets, offset), Decoded::Destination),
            Self::PortPermission => with(
                decode_port_permission(octets, offset),
                Decoded::PortPermission,
            ),
            Self::ReadAccessSpecification => with(
                decode_read_access_specification(octets, offset),
                Decoded::ReadAccessSpecification,
            ),
            Self::ReadAccessResult => with(
                read_access_result(octets, offset),
                Decoded::ReadAccessResult,
            ),
            Self::ActionList => with(decode_action_list(octets, offset), Decoded::ActionList),
            Self::DeviceObjectReference => with(
                decode_device_object_reference(octets, offset),
                Decoded::DeviceObjectReference,
            ),
            Self::AuthenticationFactorFormat => with(
                decode_authentication_factor_format(octets, offset),
                Decoded::AuthenticationFactorFormat,
            ),
            Self::AuthenticationPolicy => with(
                decode_authentication_policy(octets, offset),
                Decoded::AuthenticationPolicy,
            ),
            Self::StageLimitValue => with(
                decode_stage_limit_value(octets, offset),
                Decoded::StageLimitValue,
            ),
            Self::AccessRule => with(access_rule(octets, offset), Decoded::AccessRule),
            Self::DeviceObjectPropertyReference => with(
                decode_device_object_property_reference(octets, offset),
                Decoded::DeviceObjectPropertyReference,
            ),
            Self::PropertyAccessResult => with(
                decode_property_access_result(octets, offset),
                Decoded::PropertyAccessResult,
            ),
            Self::Recipient => with(decode_recipient(octets, offset), Decoded::Recipient),
            Self::DailySchedule => with(
                decode_daily_schedule(octets, offset),
                Decoded::DailySchedule,
            ),
            Self::SpecialEvent => with(decode_special_event(octets, offset), Decoded::SpecialEvent),
            Self::CalendarEntry => with(
                decode_calendar_entry(octets, offset),
                Decoded::CalendarEntry,
            ),
            Self::DateRange => with(decode_date_range(octets, offset), Decoded::DateRange),
            Self::TimeStamp => with(decode_timestamp_choice(octets, offset), Decoded::TimeStamp),
            Self::CovSubscription => with(
                decode_cov_subscription(octets, offset),
                Decoded::CovSubscription,
            ),
            Self::ValueSource => with(decode_value_source(octets, offset), Decoded::ValueSource),
            Self::Scale => with(decode_scale(octets, offset), Decoded::Scale),
            Self::Prescale => with(decode_prescale(octets, offset), Decoded::Prescale),
            Self::AddressBinding => with(address_binding(octets, offset), Decoded::AddressBinding),
        }
    }
}

/// Decode one access rule at `offset`. Its Python form says ALWAYS and ALL
/// by leaving the time range and location out, so only a rule whose
/// specifiers agree with the references it carries has one; any other rule
/// leaves the value to the generic decoder.
fn access_rule(octets: &[u8], offset: usize) -> Result<(BACnetAccessRule, usize), Error> {
    let (rule, end) = decode_access_rule(octets, offset)?;
    let canonical = BACnetAccessRule::new(rule.time_range.clone(), rule.location.clone(), true);
    if rule.time_range_specifier != canonical.time_range_specifier
        || rule.location_specifier != canonical.location_specifier
    {
        return Err(Error::decoding(
            offset,
            "access rule specifiers disagree with its references",
        ));
    }
    Ok((rule, end))
}

/// One BACnetAddressBinding: the Device, its network number and its MAC.
pub(crate) type AddressBinding = (ObjectIdentifier, u16, Vec<u8>);

/// Decode one BACnetAddressBinding at `offset`: the Device's identifier,
/// then its address's network number and MAC, each application tagged.
fn address_binding(octets: &[u8], offset: usize) -> Result<(AddressBinding, usize), Error> {
    use bacnet_encoding::primitives::decode_application_value as next;
    let unexpected =
        |at, what| Error::decoding(at, format!("expected the address binding's {what}"));
    let (PropertyValue::ObjectIdentifier(device), at) = next(octets, offset)? else {
        return Err(unexpected(offset, "Device identifier"));
    };
    let (PropertyValue::Unsigned(network), mac_at) = next(octets, at)? else {
        return Err(unexpected(at, "network number"));
    };
    let network = u16::try_from(network).map_err(|_| unexpected(at, "Unsigned16 network"))?;
    let (PropertyValue::OctetString(mac), end) = next(octets, mac_at)? else {
        return Err(unexpected(mac_at, "MAC"));
    };
    Ok(((device, network, mac), end))
}

/// Decode one Group Present_Value element at `offset`: an object identifier
/// in context tag 0, then its results inside an opening and closing tag 1.
/// The element is one result of a ReadPropertyMultiple ACK, so that ACK's
/// decoder checks it.
fn read_access_result(octets: &[u8], offset: usize) -> Result<(ReadAccessResult, usize), Error> {
    let (tag, contents) = tags::decode_tag(octets, offset)?;
    if !tag.is_context(0) || tag.is_opening || tag.is_closing {
        return Err(Error::decoding(
            offset,
            "expected the object identifier [0]",
        ));
    }
    let after = contents
        .checked_add(tag.length as usize)
        .filter(|&after| after <= octets.len())
        .ok_or_else(|| Error::decoding(contents, "object identifier cut short"))?;
    let (tag, contents) = tags::decode_tag(octets, after)?;
    if !tag.is_opening_tag(1) {
        return Err(Error::decoding(after, "expected the results [1]"));
    }
    let (_, end) = tags::extract_context_value(octets, contents, 1)?;
    let ack = ReadPropertyMultipleACK::decode(&octets[offset..end])?;
    let [result] = <[ReadAccessResult; 1]>::try_from(ack.list_of_read_access_results)
        .map_err(|_| Error::decoding(offset, "expected one result"))?;
    Ok((result, end))
}

#[cfg(test)]
#[path = "constructed_read_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "constructed_read_more_tests.rs"]
mod more_tests;

#[cfg(test)]
#[path = "constructed_read_scale_tests.rs"]
mod scale_tests;

#[cfg(test)]
#[path = "constructed_read_tags_tests.rs"]
mod tags_tests;

#[cfg(test)]
#[path = "constructed_read_reference_tests.rs"]
mod reference_tests;
