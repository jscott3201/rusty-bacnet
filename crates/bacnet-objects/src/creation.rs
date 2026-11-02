//! The object types a CreateObject request can build (Clause 15.3), one
//! constructor per type (#2172), and the rest of each built-in type's
//! lifecycle: what it takes only at creation, and whether DeleteObject
//! removes it (#2179).
//!
//! The table here is the one source for both sides of createability: the
//! bundled server's CreateObject handler builds each new object through
//! [`builtin_constructor`], and its PICS reports a type as createable when
//! the table has an entry for it (or the application registered a
//! constructor for it). Createability, the creation-only properties
//! ([`builtin_creation_only`], from each type's [`Creatable`] impl) and
//! deleteability ([`builtin_deleteable`]) belong to the type, not to an
//! instance, so no object is asked; only code that names this table links
//! its constructors and initializers.
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
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::database::ObjectDatabase;

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

/// A type whose objects take some CreateObject initial values that
/// WriteProperty can't change afterwards (#1429).
///
/// Clause 15.3 doesn't limit initial values to the properties WriteProperty
/// can change, so a device may take some of its read-only properties at
/// creation. The bundled server hands such an initial value, written
/// without an array index, to [`initialize_property`](Self::initialize_property)
/// instead of the write route; every other initial value goes the way a
/// WriteProperty does. The PICS lists the set for each createable type.
///
/// This belongs to the type, not to an instance, so it isn't a
/// [`BACnetObject`] method. The built-in types the table builds implement
/// it, and an application implements it for a type it registers a
/// CreateObject constructor for (the bundled server's
/// `ObjectConstructors::register` takes the constructor's concrete type).
/// The defaults set no property only at creation, so a type with none
/// implements it with an empty `impl`.
pub trait Creatable: BACnetObject + Sized + 'static {
    /// The properties an initial value may set although WriteProperty
    /// refuses them.
    const CREATION_ONLY_PROPERTIES: &'static [PropertyIdentifier] = &[];

    /// Apply a CreateObject initial value to one of
    /// [`CREATION_ONLY_PROPERTIES`](Self::CREATION_ONLY_PROPERTIES),
    /// written whole.
    ///
    /// The server calls this only on the object it is creating, before any
    /// other request can see it. Check the value as a write would, and on
    /// `Err` leave the object unchanged: the server then refuses the
    /// request, naming the initial value, and drops the object. As on the
    /// write route, an initial value that is one application NULL and that
    /// this refuses with PROPERTY / INVALID_DATA_TYPE leaves the property as
    /// it is and counts as applied, so refuse a NULL that way. The default
    /// refuses every property with PROPERTY / WRITE_ACCESS_DENIED, the
    /// answer for a property that can't be initialized at creation (Clause
    /// 15.3.1.3.1).
    fn initialize_property(
        &mut self,
        _property: PropertyIdentifier,
        _value: PropertyValue,
    ) -> Result<(), Error> {
        Err(crate::common::write_access_denied_error())
    }
}

/// One type's [`Creatable`] answers with the type erased: what the table
/// holds for a built-in type, and a server's registration for an
/// application's type.
#[derive(Clone, Copy)]
pub struct CreationOnly {
    properties: &'static [PropertyIdentifier],
    initialize: Initializer,
}

type Initializer =
    fn(&mut (dyn BACnetObject + 'static), PropertyIdentifier, PropertyValue) -> Result<(), Error>;

impl CreationOnly {
    /// No property is set only at creation.
    pub const NONE: Self = Self {
        properties: &[],
        initialize: |_, _, _| Err(crate::common::write_access_denied_error()),
    };

    /// `O`'s creation-only properties and initializer.
    pub const fn of<O: Creatable>() -> Self {
        Self {
            properties: O::CREATION_ONLY_PROPERTIES,
            initialize: initialize_as::<O>,
        }
    }

    /// The properties set only at creation.
    pub fn properties(&self) -> &'static [PropertyIdentifier] {
        self.properties
    }

    /// Whether `property` is set only at creation.
    pub fn contains(&self, property: PropertyIdentifier) -> bool {
        self.properties.contains(&property)
    }

    /// Apply an initial value to the object `oid` names in `db`, through
    /// [`Creatable::initialize_property`]. An object that isn't of the type
    /// these answers came from, or isn't there, is refused with PROPERTY /
    /// WRITE_ACCESS_DENIED, as the default refuses.
    pub fn initialize(
        &self,
        db: &mut ObjectDatabase,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let object = db
            .stored_object_mut(oid)
            .ok_or_else(crate::common::write_access_denied_error)?;
        self.initialize_object(object, property, value)
    }

    /// [`initialize`](Self::initialize) on the stored object itself.
    pub(crate) fn initialize_object(
        &self,
        object: &mut (dyn BACnetObject + 'static),
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        (self.initialize)(object, property, value)
    }
}

impl core::fmt::Debug for CreationOnly {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("CreationOnly")
            .field(&self.properties)
            .finish()
    }
}

fn initialize_as<O: Creatable>(
    object: &mut (dyn BACnetObject + 'static),
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let object = object
        .as_stored_any_mut(crate::traits::ObjectStorageAccess(()))
        .downcast_mut::<O>()
        .ok_or_else(crate::common::write_access_denied_error)?;
    Creatable::initialize_property(object, property, value)
}

/// The old per-object forms of the table's answers, for the tests that
/// build objects through the table.
#[cfg(test)]
pub(crate) trait CreationTestExt {
    /// The type's creation-only properties.
    fn creation_only_properties(&self) -> &'static [PropertyIdentifier];
    /// The type's initializer, applied to this object.
    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error>;
}

#[cfg(test)]
impl CreationTestExt for dyn BACnetObject {
    fn creation_only_properties(&self) -> &'static [PropertyIdentifier] {
        builtin_creation_only(self.object_identifier().object_type()).properties()
    }

    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        builtin_creation_only(self.object_identifier().object_type())
            .initialize_object(self, property, value)
    }
}

/// The number of states a new multi-state object starts with, until a
/// Number_Of_States or State_Text initial value sets it.
const MULTI_STATE_DEFAULT_STATES: u32 = 2;

const BUILTIN: &[Entry] = &[
    creatable::<AnalogInputObject>(ObjectType::ANALOG_INPUT, |instance, name, _| {
        boxed(AnalogInputObject::new(
            instance,
            name,
            EngineeringUnits::NO_UNITS,
        ))
    }),
    creatable::<AnalogOutputObject>(ObjectType::ANALOG_OUTPUT, |instance, name, _| {
        boxed(AnalogOutputObject::new(
            instance,
            name,
            EngineeringUnits::NO_UNITS,
        ))
    }),
    creatable::<AnalogValueObject>(ObjectType::ANALOG_VALUE, |instance, name, initial| {
        let mut object = AnalogValueObject::new(instance, name, EngineeringUnits::NO_UNITS)?;
        object.set_audit_policy(audit_policy(initial));
        boxed(Ok(object))
    }),
    creatable::<BinaryInputObject>(ObjectType::BINARY_INPUT, |instance, name, _| {
        boxed(BinaryInputObject::new(instance, name))
    }),
    creatable::<BinaryOutputObject>(ObjectType::BINARY_OUTPUT, |instance, name, _| {
        boxed(BinaryOutputObject::new(instance, name))
    }),
    creatable::<BinaryValueObject>(ObjectType::BINARY_VALUE, |instance, name, initial| {
        let mut object = BinaryValueObject::new(instance, name)?;
        object.set_audit_policy(audit_policy(initial));
        boxed(Ok(object))
    }),
    creatable::<MultiStateInputObject>(ObjectType::MULTI_STATE_INPUT, |instance, name, _| {
        boxed(MultiStateInputObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    creatable::<MultiStateOutputObject>(ObjectType::MULTI_STATE_OUTPUT, |instance, name, _| {
        boxed(MultiStateOutputObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    creatable::<MultiStateValueObject>(ObjectType::MULTI_STATE_VALUE, |instance, name, _| {
        boxed(MultiStateValueObject::new(
            instance,
            name,
            MULTI_STATE_DEFAULT_STATES,
        ))
    }),
    creatable::<IntegerValueObject>(ObjectType::INTEGER_VALUE, |instance, name, _| {
        boxed(IntegerValueObject::new(instance, name))
    }),
    creatable::<PositiveIntegerValueObject>(
        ObjectType::POSITIVE_INTEGER_VALUE,
        |instance, name, _| boxed(PositiveIntegerValueObject::new(instance, name)),
    ),
    creatable::<LargeAnalogValueObject>(ObjectType::LARGE_ANALOG_VALUE, |instance, name, _| {
        boxed(LargeAnalogValueObject::new(instance, name))
    }),
    creatable::<CharacterStringValueObject>(
        ObjectType::CHARACTERSTRING_VALUE,
        |instance, name, _| boxed(CharacterStringValueObject::new(instance, name)),
    ),
    creatable::<OctetStringValueObject>(ObjectType::OCTETSTRING_VALUE, |instance, name, _| {
        boxed(OctetStringValueObject::new(instance, name))
    }),
    creatable::<BitStringValueObject>(ObjectType::BITSTRING_VALUE, |instance, name, _| {
        boxed(BitStringValueObject::new(instance, name))
    }),
    creatable::<DateValueObject>(ObjectType::DATE_VALUE, |instance, name, _| {
        boxed(DateValueObject::new(instance, name))
    }),
    creatable::<TimeValueObject>(ObjectType::TIME_VALUE, |instance, name, _| {
        boxed(TimeValueObject::new(instance, name))
    }),
    creatable::<DateTimeValueObject>(ObjectType::DATETIME_VALUE, |instance, name, _| {
        boxed(DateTimeValueObject::new(instance, name))
    }),
    creatable::<DatePatternValueObject>(ObjectType::DATEPATTERN_VALUE, |instance, name, _| {
        boxed(DatePatternValueObject::new(instance, name))
    }),
    creatable::<TimePatternValueObject>(ObjectType::TIMEPATTERN_VALUE, |instance, name, _| {
        boxed(TimePatternValueObject::new(instance, name))
    }),
    creatable::<DateTimePatternValueObject>(
        ObjectType::DATETIMEPATTERN_VALUE,
        |instance, name, _| boxed(DateTimePatternValueObject::new(instance, name)),
    ),
    creatable::<CalendarObject>(ObjectType::CALENDAR, |instance, name, _| {
        boxed(CalendarObject::new(instance, name))
    }),
    creatable::<ScheduleObject>(ObjectType::SCHEDULE, |instance, name, _| {
        boxed(ScheduleObject::new(instance, name, PropertyValue::Null))
    }),
    creatable::<NotificationClass>(ObjectType::NOTIFICATION_CLASS, |instance, name, _| {
        boxed(NotificationClass::new(instance, name))
    }),
    creatable::<TrendLogObject>(ObjectType::TREND_LOG, |instance, name, _| {
        boxed(TrendLogObject::new(instance, name, CREATED_BUFFER_SIZE))
    }),
    // The legacy Event_Parameters layout names no algorithm, so on a created
    // enrollment it runs none: Event_Type reads NONE for it.
    creatable::<EventEnrollmentObject>(ObjectType::EVENT_ENROLLMENT, |instance, name, _| {
        boxed(EventEnrollmentObject::new(instance, name, EventType::NONE))
    }),
    creatable::<StructuredViewObject>(ObjectType::STRUCTURED_VIEW, |instance, name, _| {
        boxed(StructuredViewObject::new(instance, name))
    }),
    // The node itself and its ports: never created, and never deleted.
    fixed(ObjectType::DEVICE),
    fixed(ObjectType::NETWORK_PORT),
];

/// One built-in type's lifecycle: how CreateObject builds it, if it does,
/// what it takes only at creation, and whether DeleteObject removes it. A
/// type with no entry isn't created and is deleteable.
struct Entry {
    object_type: ObjectType,
    constructor: Option<BuiltinConstructor>,
    creation_only: CreationOnly,
    deleteable: bool,
}

/// A type CreateObject builds through `constructor`, with `O`'s
/// creation-only properties.
const fn creatable<O: Creatable>(
    object_type: ObjectType,
    constructor: BuiltinConstructor,
) -> Entry {
    Entry {
        object_type,
        constructor: Some(constructor),
        creation_only: CreationOnly::of::<O>(),
        deleteable: true,
    }
}

/// A type that is neither created nor deleted.
const fn fixed(object_type: ObjectType) -> Entry {
    Entry {
        object_type,
        constructor: None,
        creation_only: CreationOnly::NONE,
        deleteable: false,
    }
}

fn entry(object_type: ObjectType) -> Option<&'static Entry> {
    BUILTIN
        .iter()
        .find(|entry| entry.object_type == object_type)
}

/// The constructor for `object_type`, or `None` when CreateObject doesn't
/// build that type.
pub fn builtin_constructor(object_type: ObjectType) -> Option<BuiltinConstructor> {
    entry(object_type)?.constructor
}

/// Every type the table builds, in table order.
pub fn builtin_createable_types() -> impl Iterator<Item = ObjectType> {
    BUILTIN
        .iter()
        .filter(|entry| entry.constructor.is_some())
        .map(|entry| entry.object_type)
}

/// What a new object of a built-in `object_type` takes only at creation:
/// [`CreationOnly::NONE`] for a type the table doesn't build.
pub fn builtin_creation_only(object_type: ObjectType) -> CreationOnly {
    entry(object_type).map_or(CreationOnly::NONE, |entry| entry.creation_only)
}

/// Whether DeleteObject may remove an object of a built-in `object_type`.
///
/// Deleteability belongs to the type: every type is deleteable but Device
/// and Network Port, which model the node itself and its ports. The
/// database still refuses to remove an object a running runtime holds,
/// such as the Audit Reporter an endpoint reports through
/// ([`ObjectDatabase::check_removable`]).
pub fn builtin_deleteable(object_type: ObjectType) -> bool {
    entry(object_type).is_none_or(|entry| entry.deleteable)
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

#[cfg(test)]
#[path = "creation_table_tests.rs"]
mod table_tests;
