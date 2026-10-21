//! Typed read results for constructed properties: the collections the
//! binding also takes as typed values (#1310), and the other constructed
//! lists, arrays and single values the stack has a codec for (#1344, #1345).
//!
//! Nothing in a list of constructed elements marks where one element ends,
//! so the generic decoder in `read_value` can't split it. For the properties
//! `bacnet_encoding::constructed::property_element` names, the element's
//! datatype is known, so the value is split
//! with that datatype's codec from `bacnet_encoding::constructed` (or, for a
//! Group's Present_Value, the ReadPropertyMultiple ACK decoder). Each element
//! keeps its own octets, so the value encodes back to exactly what was read,
//! and its Python form (see `constructed_py`) is built from those octets
//! when `.value` is asked for. A value that doesn't decode as the expected
//! elements, to the last octet, is left to the generic decoder.

pub(super) use bacnet_encoding::constructed::ConstructedElement as Element;
use bacnet_encoding::constructed::{
    decode_access_rule, decode_action_list, decode_authentication_factor,
    decode_authentication_factor_format, decode_authentication_policy, decode_calendar_entry,
    decode_cov_subscription, decode_daily_schedule, decode_date_range, decode_destination,
    decode_device_object_property_reference, decode_device_object_reference,
    decode_event_parameter, decode_name_value, decode_object_property_reference_at,
    decode_port_permission, decode_prescale, decode_property_access_result,
    decode_read_access_specification, decode_recipient, decode_scale, decode_shed_level,
    decode_special_event, decode_stage_limit_value, decode_value_source, property_element,
    split_elements, ElementShape,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_encoding::tags;
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK};
use bacnet_types::constructed::{BACnetAccessRule, BACnetEventParameter};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use super::constructed_py::Decoded;
use super::PyPropertyValue;

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
    let (element, shape) = property_element(object_type, property)?;
    let ends = element.split(octets)?;
    let one = || {
        Some(PyPropertyValue::constructed(
            PropertyValue::ApplicationData(octets.to_vec()),
            element,
        ))
    };
    match (shape, array_index) {
        (ElementShape::Collection, None) if !ends.is_empty() => {
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
        (ElementShape::Collection, Some(index)) if index != 0 && ends.len() == 1 => one(),
        (ElementShape::Single, None) if ends.len() == 1 => one(),
        _ => None,
    }
}

/// The tag of a value the generic decoder read.
const GENERIC_TAG: &str = "application_data";

/// The binding's view of each element: its `PropertyValue.tag`, its Python
/// form, and the codec that splits a read into elements.
pub(super) trait TypedElement: Sized {
    /// The element whose [`Self::tag`] is `tag`.
    fn from_tag(tag: &str) -> Option<Self>;
    /// Whether `octets` are exactly one element of this production.
    fn is_one(&self, octets: &[u8]) -> bool;
    /// The `PropertyValue.tag` of one element.
    fn tag(self) -> &'static str;
    /// The Python form of one element's octets.
    fn to_py(self, py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>>;
    /// Where each element of `octets` ends, or `None` when one doesn't
    /// decode.
    fn split(self, octets: &[u8]) -> Option<Vec<usize>>;
    /// Decode the element at `offset`, returning it and the offset past it.
    fn decode_at(self, octets: &[u8], offset: usize) -> Result<(Decoded, usize), Error>;
}

impl TypedElement for Element {
    /// The element whose [`Self::tag`] is `tag`.
    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|element| tag != GENERIC_TAG && element.tag() == tag)
    }

    /// Whether `octets` are exactly one element of this production.
    fn is_one(&self, octets: &[u8]) -> bool {
        self.split(octets).is_some_and(|ends| ends.len() == 1)
    }

    /// The `PropertyValue.tag` of one element.
    fn tag(self) -> &'static str {
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
            Self::ObjectPropertyReference => "object_property_reference",
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
            Self::ShedLevel => "shed_level",
            Self::AuthenticationFactor => "authentication_factor",
            Self::EventParameter => "event_parameter",
            // An element newer than the binding has no typed form here
            // (`decode_at` refuses it), so no read is ever tagged with it.
            _ => GENERIC_TAG,
        }
    }

    /// The Python form of one element's octets. They decoded when the read
    /// was split, so they decode again here; if they somehow didn't, the
    /// octets come back as `bytes` rather than raising.
    fn to_py(self, py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
        match self.decode_at(octets, 0) {
            Ok((decoded, _)) => decoded.into_python(py),
            Err(_) => Ok(PyBytes::new(py, octets).into_any().unbind()),
        }
    }

    /// Where each element of `octets` ends, or `None` when one doesn't
    /// decode.
    fn split(self, octets: &[u8]) -> Option<Vec<usize>> {
        split_elements(octets, |octets, offset| {
            self.decode_at(octets, offset).map(|(_, end)| end)
        })
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
            Self::ObjectPropertyReference => with(
                decode_object_property_reference_at(octets, offset),
                Decoded::ObjectPropertyReference,
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
            Self::ShedLevel => with(decode_shed_level(octets, offset), Decoded::ShedLevel),
            Self::AuthenticationFactor => with(
                decode_authentication_factor(octets, offset),
                Decoded::AuthenticationFactor,
            ),
            Self::EventParameter => with(event_parameter(octets, offset), Decoded::EventParameter),
            _ => Err(Error::decoding(
                offset,
                "the binding has no typed form for this element",
            )),
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

/// Decode one Event_Parameters value at `offset`. The decoder also takes two
/// forms that aren't the wire CHOICE (an application octet string, and the
/// legacy wrapper the enrollment once stored); both come back as an opaque
/// value with tag 255, which leaves the value to the generic decoder.
fn event_parameter(octets: &[u8], offset: usize) -> Result<(BACnetEventParameter, usize), Error> {
    let (parameter, end) = decode_event_parameter(octets, offset)?;
    if let BACnetEventParameter::Opaque { tag: u8::MAX, .. } = parameter {
        return Err(Error::decoding(offset, "not a BACnetEventParameter CHOICE"));
    }
    Ok((parameter, end))
}

/// One BACnetAddressBinding: the Device, its network number and its MAC.
pub(crate) type AddressBinding = (ObjectIdentifier, u16, Vec<u8>);

/// Decode one BACnetAddressBinding at `offset`: the Device's identifier,
/// then its address's network number and MAC, each application tagged.
fn address_binding(octets: &[u8], offset: usize) -> Result<(AddressBinding, usize), Error> {
    use bacnet_encoding::primitives::decode_application_value as next;
    let unexpected = |at, label: &'static str| Error::decoding(at, label);
    let (PropertyValue::ObjectIdentifier(device), at) = next(octets, offset)? else {
        return Err(unexpected(
            offset,
            "expected the address binding's Device identifier",
        ));
    };
    let (PropertyValue::Unsigned(network), mac_at) = next(octets, at)? else {
        return Err(unexpected(
            at,
            "expected the address binding's network number",
        ));
    };
    let network = u16::try_from(network)
        .map_err(|_| unexpected(at, "expected the address binding's Unsigned16 network"))?;
    let (PropertyValue::OctetString(mac), end) = next(octets, mac_at)? else {
        return Err(unexpected(mac_at, "expected the address binding's MAC"));
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

#[cfg(test)]
#[path = "constructed_read_single_tests.rs"]
mod single_tests;
