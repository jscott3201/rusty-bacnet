//! Which constructed production a property's value is made of.
//!
//! A value read from a device arrives as bare octets. For a primitive value
//! the application tags say where each element starts and ends, but a list
//! or array of constructed elements (timestamps, schedule days, references)
//! carries nothing that marks the boundary between one element and the
//! next. Splitting it takes the element's datatype, and that depends on the
//! object type and the property, which the octets don't say.
//!
//! [`property_element`] is that knowledge as a table: for each (object type,
//! property) pair whose datatype it knows, the [`ConstructedElement`] the
//! value is made of and whether the property holds one of them or a
//! collection ([`ElementShape`]). Each element names the codec in
//! [`crate::constructed`] (or, for [`ConstructedElement::ReadAccessResult`],
//! the ReadPropertyMultiple ACK decoder in `bacnet-services`) that decodes
//! it, which `split_elements` takes to find where each element ends.
//!
//! The table needs no allocator; `split_elements` needs `alloc`.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
#[cfg(feature = "alloc")]
use bacnet_types::error::Error;

/// A constructed production a property's value is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConstructedElement {
    /// A Tags element, a `BACnetNameValue`
    /// (`decode_name_value`).
    NameValue,
    /// A Recipient_List element, a `BACnetDestination`
    /// (`decode_destination`).
    Destination,
    /// A Port_Filter element, a `BACnetPortPermission`
    /// (`decode_port_permission`).
    PortPermission,
    /// A Group member, a `ReadAccessSpecification`
    /// (`decode_read_access_specification`).
    ReadAccessSpecification,
    /// A Group's Present_Value element: one result of a ReadPropertyMultiple
    /// ACK, which `bacnet-services` decodes.
    ReadAccessResult,
    /// A Command's Action element, a `BACnetActionList`
    /// (`decode_action_list`).
    ActionList,
    /// A `BACnetDeviceObjectReference`
    /// (`decode_device_object_reference`).
    DeviceObjectReference,
    /// A Supported_Formats element, a `BACnetAuthenticationFactorFormat`
    /// (`decode_authentication_factor_format`).
    AuthenticationFactorFormat,
    /// An Authentication_Policy_List element, a `BACnetAuthenticationPolicy`
    /// (`decode_authentication_policy`).
    AuthenticationPolicy,
    /// A Stages element, a `BACnetStageLimitValue`
    /// (`decode_stage_limit_value`).
    StageLimitValue,
    /// An Access Rights rule, a `BACnetAccessRule`
    /// (`decode_access_rule`).
    AccessRule,
    /// A `BACnetDeviceObjectPropertyReference`
    /// (`decode_device_object_property_reference`).
    DeviceObjectPropertyReference,
    /// A `BACnetObjectPropertyReference`, which names no device
    /// (`decode_object_property_reference_at`).
    ObjectPropertyReference,
    /// A Global Group member's result, a `BACnetPropertyAccessResult`
    /// (`decode_property_access_result`).
    PropertyAccessResult,
    /// A `BACnetRecipient`
    /// (`decode_recipient`).
    Recipient,
    /// A Weekly_Schedule day, a `BACnetDailySchedule`
    /// (`decode_daily_schedule`).
    DailySchedule,
    /// An Exception_Schedule element, a `BACnetSpecialEvent`
    /// (`decode_special_event`).
    SpecialEvent,
    /// A Date_List element, a `BACnetCalendarEntry`
    /// (`decode_calendar_entry`).
    CalendarEntry,
    /// A `BACnetDateRange`
    /// (`decode_date_range`).
    DateRange,
    /// A `BACnetTimeStamp`
    /// (`decode_timestamp_choice`).
    TimeStamp,
    /// An Active_COV_Subscriptions element, a `BACnetCOVSubscription`
    /// (`decode_cov_subscription`).
    CovSubscription,
    /// A command source, a `BACnetValueSource`
    /// (`decode_value_source`).
    ValueSource,
    /// An Accumulator's Scale, a `BACnetScale`
    /// (`decode_scale`).
    Scale,
    /// An Accumulator's Prescale, a `BACnetPrescale`
    /// (`decode_prescale`).
    Prescale,
    /// A Device_Address_Binding element: the Device's identifier, then its
    /// network number and MAC, each application tagged.
    AddressBinding,
    /// A Load Control shed level, a `BACnetShedLevel`
    /// (`decode_shed_level`).
    ShedLevel,
    /// A `BACnetAuthenticationFactor`
    /// (`decode_authentication_factor`).
    AuthenticationFactor,
    /// An Event Enrollment's Event_Parameters, a `BACnetEventParameter`
    /// (`decode_event_parameter`).
    EventParameter,
}

impl ConstructedElement {
    /// Every element, each once.
    pub const ALL: [Self; 28] = [
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
        Self::ObjectPropertyReference,
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
        Self::ShedLevel,
        Self::AuthenticationFactor,
        Self::EventParameter,
    ];
}

/// How a property holds its elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementShape {
    /// A BACnetARRAY or BACnetLIST: a whole read is the elements back to
    /// back, and an indexed read of an array one element.
    Collection,
    /// One value: a whole read is the element itself.
    Single,
}

/// The element `property` of `object_type` is made of, and how the property
/// holds it; `None` for a property this table doesn't know, whose value the
/// application tags alone describe or whose datatype it doesn't cover.
pub fn property_element(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> Option<(ConstructedElement, ElementShape)> {
    type O = ObjectType;
    type P = PropertyIdentifier;
    use ConstructedElement as E;
    use ElementShape::{Collection, Single};
    Some(match (object_type, property) {
        (O::NOTIFICATION_CLASS | O::NOTIFICATION_FORWARDER, P::RECIPIENT_LIST) => {
            (E::Destination, Collection)
        }
        (O::NOTIFICATION_FORWARDER, P::PORT_FILTER) => (E::PortPermission, Collection),
        (O::GROUP, P::LIST_OF_GROUP_MEMBERS) => (E::ReadAccessSpecification, Collection),
        (O::GROUP, P::PRESENT_VALUE) => (E::ReadAccessResult, Collection),
        (O::COMMAND, P::ACTION) => (E::ActionList, Collection),
        (O::ACCESS_DOOR, P::DOOR_MEMBERS)
        | (O::ACCESS_POINT, P::ACCESS_DOORS)
        | (O::STAGING, P::TARGET_REFERENCES)
        | (O::ACCESS_ZONE, P::ENTRY_POINTS | P::EXIT_POINTS)
        | (O::ACCESS_USER, P::CREDENTIALS | P::MEMBERS | P::MEMBER_OF)
        | (O::LIFE_SAFETY_POINT | O::LIFE_SAFETY_ZONE, P::MEMBER_OF)
        | (O::LIFE_SAFETY_ZONE, P::ZONE_MEMBERS)
        | (O::STRUCTURED_VIEW, P::SUBORDINATE_LIST) => (E::DeviceObjectReference, Collection),
        (O::ACCESS_RIGHTS, P::ACCOMPANIMENT)
        | (O::ACCESS_POINT, P::ACCESS_EVENT_CREDENTIAL)
        | (O::LIFT | O::ESCALATOR, P::ENERGY_METER_REF) => (E::DeviceObjectReference, Single),
        (O::ACCESS_POINT, P::ACCESS_EVENT_AUTHENTICATION_FACTOR)
        | (O::CREDENTIAL_DATA_INPUT, P::PRESENT_VALUE) => (E::AuthenticationFactor, Single),
        (O::ACCESS_POINT, P::ACCESS_EVENT_TIME) => (E::TimeStamp, Single),
        (
            O::LOAD_CONTROL,
            P::REQUESTED_SHED_LEVEL | P::EXPECTED_SHED_LEVEL | P::ACTUAL_SHED_LEVEL,
        ) => (E::ShedLevel, Single),
        (O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS) => (E::EventParameter, Single),
        (O::CREDENTIAL_DATA_INPUT, P::SUPPORTED_FORMATS) => {
            (E::AuthenticationFactorFormat, Collection)
        }
        (O::ACCESS_POINT, P::AUTHENTICATION_POLICY_LIST) => (E::AuthenticationPolicy, Collection),
        (O::STAGING, P::STAGES) => (E::StageLimitValue, Collection),
        (O::ACCESS_RIGHTS, P::POSITIVE_ACCESS_RULES | P::NEGATIVE_ACCESS_RULES) => {
            (E::AccessRule, Collection)
        }
        (O::GLOBAL_GROUP, P::GROUP_MEMBERS)
        | (O::SCHEDULE | O::CHANNEL, P::LIST_OF_OBJECT_PROPERTY_REFERENCES)
        | (O::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY) => {
            (E::DeviceObjectPropertyReference, Collection)
        }
        (O::TREND_LOG, P::LOG_DEVICE_OBJECT_PROPERTY)
        | (O::AVERAGING | O::EVENT_ENROLLMENT, P::OBJECT_PROPERTY_REFERENCE) => {
            (E::DeviceObjectPropertyReference, Single)
        }
        (O::GLOBAL_GROUP, P::PRESENT_VALUE) => (E::PropertyAccessResult, Collection),
        (O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT) => (E::Recipient, Single),
        (O::SCHEDULE, P::WEEKLY_SCHEDULE) => (E::DailySchedule, Collection),
        (O::SCHEDULE, P::EXCEPTION_SCHEDULE) => (E::SpecialEvent, Collection),
        (O::SCHEDULE, P::EFFECTIVE_PERIOD) => (E::DateRange, Single),
        (O::CALENDAR, P::DATE_LIST) => (E::CalendarEntry, Collection),
        (O::DEVICE, P::ACTIVE_COV_SUBSCRIPTIONS) => (E::CovSubscription, Collection),
        (O::DEVICE, P::DEVICE_ADDRESS_BINDING) => (E::AddressBinding, Collection),
        (O::ACCUMULATOR, P::SCALE) => (E::Scale, Single),
        (O::ACCUMULATOR, P::PRESCALE) => (E::Prescale, Single),
        // Every object type that has these properties gives them one datatype.
        (_, P::TAGS) => (E::NameValue, Collection),
        (_, P::EVENT_TIME_STAMPS | P::COMMAND_TIME_ARRAY) => (E::TimeStamp, Collection),
        (_, P::LAST_COMMAND_TIME | P::LAST_RESTORE_TIME) => (E::TimeStamp, Single),
        (_, P::VALUE_SOURCE_ARRAY) => (E::ValueSource, Collection),
        (_, P::VALUE_SOURCE) => (E::ValueSource, Single),
        (_, P::EVENT_ALGORITHM_INHIBIT_REF) => (E::ObjectPropertyReference, Single),
        _ => return None,
    })
}

/// Where each element of `octets` ends, with `decode` reading the element
/// that starts at an offset and returning the offset just past it. `None`
/// when an element doesn't decode or `decode` doesn't move forward, so a
/// value that isn't wholly made of the elements is never half split. Empty
/// octets split into no elements.
#[cfg(feature = "alloc")]
pub fn split_elements(
    octets: &[u8],
    mut decode: impl FnMut(&[u8], usize) -> Result<usize, Error>,
) -> Option<alloc::vec::Vec<usize>> {
    let mut ends = alloc::vec::Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let end = decode(octets, offset).ok()?;
        if end <= offset || end > octets.len() {
            return None;
        }
        ends.push(end);
        offset = end;
    }
    Some(ends)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_the_element_and_its_shape() {
        type O = ObjectType;
        type P = PropertyIdentifier;
        use ConstructedElement as E;
        for (object_type, property, expected) in [
            (
                O::ANALOG_INPUT,
                P::EVENT_TIME_STAMPS,
                Some((E::TimeStamp, ElementShape::Collection)),
            ),
            (
                O::ANALOG_INPUT,
                P::EVENT_ALGORITHM_INHIBIT_REF,
                Some((E::ObjectPropertyReference, ElementShape::Single)),
            ),
            (
                O::SCHEDULE,
                P::WEEKLY_SCHEDULE,
                Some((E::DailySchedule, ElementShape::Collection)),
            ),
            (
                O::TREND_LOG,
                P::LOG_DEVICE_OBJECT_PROPERTY,
                Some((E::DeviceObjectPropertyReference, ElementShape::Single)),
            ),
            (
                O::TREND_LOG_MULTIPLE,
                P::LOG_DEVICE_OBJECT_PROPERTY,
                Some((E::DeviceObjectPropertyReference, ElementShape::Collection)),
            ),
            (O::ANALOG_INPUT, P::PRESENT_VALUE, None),
            (O::SCHEDULE, P::PRESENT_VALUE, None),
        ] {
            assert_eq!(
                property_element(object_type, property),
                expected,
                "{object_type} {property}"
            );
        }
    }

    #[test]
    fn all_lists_each_element_once() {
        for (i, element) in ConstructedElement::ALL.iter().enumerate() {
            assert!(
                !ConstructedElement::ALL[..i].contains(element),
                "{element:?}"
            );
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn a_split_stops_at_the_first_element_that_does_not_decode() {
        // Elements two octets long, while the octets hold an even count.
        let pairs = |octets: &[u8], offset: usize| {
            if offset + 2 <= octets.len() {
                Ok(offset + 2)
            } else {
                Err(Error::decoding(offset, "cut short"))
            }
        };
        assert_eq!(split_elements(&[1, 2, 3, 4], pairs), Some(vec![2, 4]));
        assert_eq!(split_elements(&[1, 2, 3], pairs), None);
        assert_eq!(split_elements(&[], pairs), Some(vec![]));
        // A decoder that doesn't move forward, or runs past the end.
        assert_eq!(split_elements(&[1], |_, offset| Ok(offset)), None);
        assert_eq!(split_elements(&[1], |_, _| Ok(5)), None);
    }
}
