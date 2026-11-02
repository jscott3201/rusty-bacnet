//! The object types a CreateObject request can build (Clause 15.3), one
//! constructor per type (#2172).
//!
//! The table here is the one source for both sides of createability: the
//! bundled server's CreateObject handler builds each new object through
//! [`builtin_constructor`], and its PICS reports a type as createable when
//! the table has an entry for it (or the application registered a
//! constructor for it). Createability belongs to the type, not to an
//! instance, so no object is asked; only code that names this table links
//! its constructors.
//!
//! Each constructor takes only the instance and a name, and starts the
//! object with the defaults of its own `new`: no units on the analog and
//! numeric value types, two states on the multi-state types, an empty
//! string or bit string, and unspecified dates and times. The request's
//! initial values are applied afterwards, as WriteProperty applies them.
//!
//! The configuration objects clients commonly create (#2208) start inert:
//! a Calendar with no dates, a Schedule with no references and a NULL
//! Schedule_Default, a Notification Class with priorities 255 and no
//! recipients, a Trend Log of [`CREATED_BUFFER_SIZE`] records with nothing
//! to log, an Event Enrollment with no reference and the `none` event
//! parameters, and a Structured View with no subordinates. Clause 15.3
//! leaves the values of properties the request doesn't give to the device,
//! and its error list has no entry for a missing property, so none of them
//! is refused for lacking initial values: a client creates one with a few
//! initial values and configures the rest by WriteProperty, which is how
//! clients use the service. An object missing its reference or parameters
//! does nothing until it has them (a Schedule writes nowhere, a Trend Log
//! logs nothing, an Event Enrollment runs no algorithm), and starts once a
//! write gives them, without a restart. The properties these types serve
//! read-only (Object_Name on all but the Event Enrollment, a Notification
//! Class's Priority and Ack_Required, a Trend Log's Buffer_Size (at most
//! [`MAX_CREATED_BUFFER_SIZE`](crate::trend::MAX_CREATED_BUFFER_SIZE)
//! records, applied before the other values), an Event
//! Enrollment's Object_Property_Reference, a Structured View's node and
//! subordinate properties) take an initial value as creation-only
//! properties (#1429).
//!
//! The types that model the device itself or its hardware are never
//! created.

use crate::prelude::*;

use bacnet_types::enums::{EngineeringUnits, EventType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::audit::{AuditPriorityPolicy, ObjectAuditPolicy};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event_enrollment::EventEnrollmentObject;
use crate::group::StructuredViewObject;
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::notification_class::NotificationClass;
use crate::schedule::{CalendarObject, ScheduleObject};
use crate::traits::BACnetObject;
use crate::trend::{CREATED_BUFFER_SIZE, TrendLogObject};
use crate::value_types::{
    BitStringValueObject, CharacterStringValueObject, DatePatternValueObject,
    DateTimePatternValueObject, DateTimeValueObject, DateValueObject, IntegerValueObject,
    LargeAnalogValueObject, OctetStringValueObject, PositiveIntegerValueObject,
    TimePatternValueObject, TimeValueObject,
};

/// Builds a new object of one type: its instance, its name, and the
/// properties the request's initial values name.
///
/// The constructor only reads the property list to provision optional rows
/// that exist on request, such as the audit policy rows of an Analog or
/// Binary Value; it applies no value.
pub type BuiltinConstructor = fn(
    instance: u32,
    name: String,
    initial_properties: &[PropertyIdentifier],
) -> Result<Box<dyn BACnetObject>, Error>;

/// The number of states a new multi-state object starts with, until a
/// Number_Of_States or State_Text initial value sets it.
const MULTI_STATE_DEFAULT_STATES: u32 = 2;

const BUILTIN: &[(ObjectType, BuiltinConstructor)] = &[
    (ObjectType::ANALOG_INPUT, |instance, name, _| {
        boxed(AnalogInputObject::new(
            instance,
            name,
            EngineeringUnits::NO_UNITS,
        ))
    }),
    (ObjectType::ANALOG_OUTPUT, |instance, name, _| {
        boxed(AnalogOutputObject::new(
            instance,
            name,
            EngineeringUnits::NO_UNITS,
        ))
    }),
    (ObjectType::ANALOG_VALUE, |instance, name, initial| {
        let mut object = AnalogValueObject::new(instance, name, EngineeringUnits::NO_UNITS)?;
        object.set_audit_policy(audit_policy(initial));
        boxed(Ok(object))
    }),
    (ObjectType::BINARY_INPUT, |instance, name, _| {
        boxed(BinaryInputObject::new(instance, name))
    }),
    (ObjectType::BINARY_OUTPUT, |instance, name, _| {
        boxed(BinaryOutputObject::new(instance, name))
    }),
    (ObjectType::BINARY_VALUE, |instance, name, initial| {
        let mut object = BinaryValueObject::new(instance, name)?;
        object.set_audit_policy(audit_policy(initial));
        boxed(Ok(object))
    }),
    (ObjectType::MULTI_STATE_INPUT, |instance, name, _| {
        boxed(MultiStateInputObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    (ObjectType::MULTI_STATE_OUTPUT, |instance, name, _| {
        boxed(MultiStateOutputObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    (ObjectType::MULTI_STATE_VALUE, |instance, name, _| {
        boxed(MultiStateValueObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    (ObjectType::INTEGER_VALUE, |instance, name, _| {
        boxed(IntegerValueObject::new(instance, name))
    }),
    (ObjectType::POSITIVE_INTEGER_VALUE, |instance, name, _| {
        boxed(PositiveIntegerValueObject::new(instance, name))
    }),
    (ObjectType::LARGE_ANALOG_VALUE, |instance, name, _| {
        boxed(LargeAnalogValueObject::new(instance, name))
    }),
    (ObjectType::CHARACTERSTRING_VALUE, |instance, name, _| {
        boxed(CharacterStringValueObject::new(instance, name))
    }),
    (ObjectType::OCTETSTRING_VALUE, |instance, name, _| {
        boxed(OctetStringValueObject::new(instance, name))
    }),
    (ObjectType::BITSTRING_VALUE, |instance, name, _| {
        boxed(BitStringValueObject::new(instance, name))
    }),
    (ObjectType::DATE_VALUE, |instance, name, _| {
        boxed(DateValueObject::new(instance, name))
    }),
    (ObjectType::TIME_VALUE, |instance, name, _| {
        boxed(TimeValueObject::new(instance, name))
    }),
    (ObjectType::DATETIME_VALUE, |instance, name, _| {
        boxed(DateTimeValueObject::new(instance, name))
    }),
    (ObjectType::DATEPATTERN_VALUE, |instance, name, _| {
        boxed(DatePatternValueObject::new(instance, name))
    }),
    (ObjectType::TIMEPATTERN_VALUE, |instance, name, _| {
        boxed(TimePatternValueObject::new(instance, name))
    }),
    (ObjectType::DATETIMEPATTERN_VALUE, |instance, name, _| {
        boxed(DateTimePatternValueObject::new(instance, name))
    }),
    (ObjectType::CALENDAR, |instance, name, _| {
        boxed(CalendarObject::new(instance, name))
    }),
    (ObjectType::SCHEDULE, |instance, name, _| {
        boxed(ScheduleObject::new(instance, name, PropertyValue::Null))
    }),
    (ObjectType::NOTIFICATION_CLASS, |instance, name, _| {
        boxed(NotificationClass::new(instance, name))
    }),
    (ObjectType::TREND_LOG, |instance, name, _| {
        boxed(TrendLogObject::new(instance, name, CREATED_BUFFER_SIZE))
    }),
    // The legacy Event_Parameters layout names no algorithm, so on a created
    // enrollment it runs none: Event_Type reads NONE for it.
    (ObjectType::EVENT_ENROLLMENT, |instance, name, _| {
        boxed(EventEnrollmentObject::new(instance, name, EventType::NONE))
    }),
    (ObjectType::STRUCTURED_VIEW, |instance, name, _| {
        boxed(StructuredViewObject::new(instance, name))
    }),
];

/// The constructor for `object_type`, or `None` when CreateObject doesn't
/// build that type.
pub fn builtin_constructor(object_type: ObjectType) -> Option<BuiltinConstructor> {
    BUILTIN
        .iter()
        .find(|(kind, _)| *kind == object_type)
        .map(|(_, constructor)| *constructor)
}

/// Every type the table builds, in table order.
pub fn builtin_createable_types() -> impl Iterator<Item = ObjectType> {
    BUILTIN.iter().map(|(kind, _)| *kind)
}

fn boxed<O: BACnetObject + 'static>(
    object: Result<O, Error>,
) -> Result<Box<dyn BACnetObject>, Error> {
    Ok(Box::new(object?))
}

/// The audit policy rows a new Analog or Binary Value serves: only those
/// its initial values name, each at its default until that value is
/// applied. WriteProperty can't add an absent row, so this is the only way
/// a created object gets one.
fn audit_policy(initial_properties: &[PropertyIdentifier]) -> ObjectAuditPolicy {
    let mut policy = ObjectAuditPolicy::default();
    for property in initial_properties {
        match *property {
            PropertyIdentifier::AUDIT_LEVEL => {
                policy.level = Some(bacnet_types::enums::AuditLevel::DEFAULT)
            }
            PropertyIdentifier::AUDITABLE_OPERATIONS => {
                policy.operations = Some(bacnet_types::bitstring::AuditOperationFlags::empty())
            }
            PropertyIdentifier::AUDIT_PRIORITY_FILTER => {
                policy.priority_filter = Some(AuditPriorityPolicy::Inherit)
            }
            _ => {}
        }
    }
    policy
}

#[cfg(test)]
#[path = "creation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "creation_config_tests.rs"]
mod config_tests;
