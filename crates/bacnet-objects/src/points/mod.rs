//! Build an object database from a point list (#2445).
//!
//! [`build_database`] takes a [`PointList`] by value and returns the
//! database a server or a device core serves: the Device, its declared
//! Network Ports, then the points in list order, with the Device's
//! Object_List in that order. It is a pure function of the list and the
//! identity: building the same list again gives the same identifiers, names
//! and order. [`build_object`] builds one point, for a host that assembles
//! its own database. The list's types live in [`bacnet_types::points`] and
//! are re-exported here.
//!
//! The database comes back before anything serves it, so the host can set
//! what a list doesn't describe first. Once something serves it, the
//! application's changes go where that host reports them (#2446):
//! - firmware with `bacnet-device-core`'s `Reporter`: `Reporter::update`
//!   for a read-only point's Present_Value, `Reporter::command` and
//!   `Reporter::relinquish` for a commandable point's priority slot as the
//!   local Device, `Reporter::set_reliability`, and `Reporter::apply_local`
//!   for any other change, such as Out_Of_Service. Each queues the COV
//!   notifications the change calls for;
//! - a `bacnet-server`: `BACnetServer::set_present_value_local`,
//!   `write_local` and `set_reliability_local`, so COV and event processing
//!   see each change.
//!
//! A host with neither, one that answers requests without reporting COV,
//! or any host before it serves the database, reaches the built objects as
//! `dyn BACnetObject` through [`ObjectDatabase::get_mut`]. The routes there
//! are:
//! - a read-only point's Present_Value: [`BACnetObject::local_input`] and
//!   its `apply_local_input`;
//! - a commandable point's priority slot: [`BACnetObject::write_property_from`]
//!   with the priority and a [`CommandOrigin::Local`] origin, a value to
//!   command the slot and NULL to relinquish it;
//! - Reliability: [`BACnetObject::reliability_eval`] and its
//!   `set_reliability`; Out_Of_Service: `write_property`.
//!
//! `bacnet-device-core`'s `write::local::apply` makes any of these changes
//! with the checks and the source the hosts' own routes give them, and
//! reports to no one.
//!
//! A refusal is a [`BuildError`] naming the object and, where one property
//! is at fault, that property. Only one is reported: the first the checks
//! meet in their order. The Device comes first, then a pass over every
//! point for repeated identifiers and names, then each point in list order
//! for the rest, so a repeated name late in the list is reported before a
//! wrong kind early in it. Making one formats nothing; its text is written
//! only when something displays it, so firmware that never shows one links
//! no formatting for it.
//!
//! [`CommandOrigin::Local`]: crate::command_source::CommandOrigin::Local
//!
//! The builder refuses:
//! - a list whose Device instance isn't the identity's, or whose Device
//!   name is empty;
//! - an identifier another point, the Device or a declared Network Port
//!   already has, or the reserved instance 4194303 (Clause 12.1.1);
//! - an empty name, or one another point, the Device or a port
//!   (`port-<instance>`) already has. `ObjectDatabase::add` would replace
//!   an object under an identifier it holds, and refuses a name in use
//!   without saying whose, so the builder checks both first. Names are
//!   compared exactly, and nothing limits their length or characters: a
//!   long dotted path such as `site.building-2.floor-3.ahu-1.supply-temp`
//!   is taken as it is, as an Object_Name write would take it;
//! - an object type other than the ten it covers (the analog, binary and
//!   multi-state Inputs, Outputs and Values, and the Integer Value), and a
//!   kind of another family than the type's;
//! - an Input whose access isn't [`Access::ReadOnly`], and an Output whose
//!   access isn't [`Access::Commandable`];
//! - a Real that isn't finite, Min_Pres_Value above Max_Pres_Value (an
//!   analog object's or an Integer Value's), Units past 65535, a state
//!   count outside 1 to [`MAX_NUMBER_OF_STATES`], and State_Text given with
//!   other than one label per state;
//! - whatever the objects' own setters refuse, as a write would be: a
//!   negative COV_Increment, a binary relinquish default other than
//!   INACTIVE or ACTIVE, a multi-state one that names no state.
//!
//! A relinquish default outside Min_Pres_Value to Max_Pres_Value builds,
//! an analog one or an Integer Value's: the builder, like the objects,
//! doesn't compare it with the range. Every INTEGER is a valid Integer
//! Value relinquish default and every Unsigned a valid COV_Increment, so
//! an Integer Value's setters refuse nothing the builder lets through.
//!
//! Identifiers are checked, never assigned, and a list has no size cap: the
//! 10,000-object cap is a rule for CreateObject from the network, and a
//! firmware image is bounded by its heap.
//!
//! ```
//! use bacnet_objects::device_identity::DeviceIdentity;
//! use bacnet_objects::points::{
//!     Access, Analog, DeviceInfo, MultiState, Point, PointList, Refusal, build_database,
//! };
//! use bacnet_objects::command_source::CommandOrigin;
//! use bacnet_objects::traits::LocalInputValue;
//! use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
//! use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
//!
//! let identity = DeviceIdentity::new(1001, 42)?;
//! let supply = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
//! let setpoint = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1)?;
//! let mode = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1)?;
//! let celsius = EngineeringUnits::DEGREES_CELSIUS;
//! let supply_point = Point::new(
//!     supply,
//!     "ahu-1.supply-air-temp",
//!     Analog::new(celsius, Access::ReadOnly).with_cov_increment(0.5),
//! );
//! let list = PointList::new(
//!     DeviceInfo::new(1001).with_name("AHU-1"),
//!     vec![
//!         supply_point.clone(),
//!         Point::new(
//!             setpoint,
//!             "ahu-1.zone-setpoint",
//!             Analog::new(celsius, Access::Commandable { relinquish_default: Some(21.0) }),
//!         ),
//!         Point::new(
//!             mode,
//!             "ahu-1.mode",
//!             MultiState::new(3, Access::Writable)
//!                 .with_state_texts(["Off", "Heat", "Cool"]),
//!         ),
//!     ],
//! );
//! let mut database = build_database(list.clone(), &identity)?;
//!
//! // Nothing serves the database yet: give the input its first reading.
//! let sensor = database.get_mut(&supply).expect("the builder added it");
//! sensor
//!     .local_input()
//!     .expect("an Analog Input takes its Present_Value from the application")
//!     .apply_local_input(LocalInputValue::present_value(PropertyValue::Real(18.5)))?;
//! // Command the setpoint at priority 8 as the application itself; a NULL
//! // there would relinquish it, back to 21.
//! let local = CommandOrigin::Local {
//!     owner_device: identity.device_oid(),
//!     initiating_object: None,
//! };
//! database.get_mut(&setpoint).expect("built").write_property_from(
//!     PropertyIdentifier::PRESENT_VALUE,
//!     None,
//!     PropertyValue::Real(23.0),
//!     Some(8),
//!     &local,
//! )?;
//! let read = |oid| database.get(&oid).expect("built").read_property(PropertyIdentifier::PRESENT_VALUE, None);
//! assert_eq!(read(supply)?, PropertyValue::Real(18.5));
//! assert_eq!(read(setpoint)?, PropertyValue::Real(23.0));
//!
//! // The same identifier twice is refused, naming the second point.
//! let mut twice = list;
//! twice.points.push(Point { name: "another".into(), ..supply_point });
//! let Err(refused) = build_database(twice, &identity) else {
//!     panic!("a repeated identifier is refused");
//! };
//! assert_eq!(refused.object, supply);
//! assert!(matches!(refused.reason, Refusal::DuplicateIdentifier));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use core::fmt;

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::collections::HashSet;
use crate::database::ObjectDatabase;
use crate::device_identity::{DeviceIdentity, NetworkPortEntry, build_database_with_extra};
use crate::multistate::{
    MAX_NUMBER_OF_STATES, MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use crate::prelude::*;
use crate::present_value_access::PresentValueAccess;
use crate::traits::BACnetObject;
use crate::value_types::IntegerValueObject;

pub use bacnet_types::points::{
    Access, Analog, Binary, DeviceInfo, Integer, MultiState, Point, PointKind, PointList,
};

/// Why a point list, or one point, couldn't be built.
#[derive(Debug)]
#[non_exhaustive]
pub struct BuildError {
    /// The refused point, or the identity's Device when the list's Device
    /// part is at fault.
    pub object: ObjectIdentifier,
    /// The property at fault; `None` when the point is refused as a whole
    /// (its type or kind) or its object's constructor refused it.
    pub property: Option<P>,
    /// What was wrong.
    pub reason: Refusal,
}

impl BuildError {
    fn new(object: ObjectIdentifier, property: Option<P>, reason: Refusal) -> Self {
        Self {
            object,
            property,
            reason,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.object)?;
        if let Some(property) = self.property {
            write!(f, " {property}")?;
        }
        write!(f, ": {}", self.reason)
    }
}

impl core::error::Error for BuildError {}

/// What was wrong with a point, as a [`BuildError`] reports it.
///
/// The builder's own checks give the named variants: [`NotFinite`] for a
/// Real that is infinite or NaN, [`MinAboveMax`], and [`OutOfRange`] for
/// Units and the state count, each before any object is made. What it
/// leaves to the object comes back as [`Object`] with the error the
/// object's constructor or setter gave, the one a write of the value gets:
/// a negative COV_Increment, or a relinquish default that isn't one of the
/// object's states. The reserved instance comes back as [`Object`] too,
/// with the error `ObjectIdentifier::new_addressable` gives it.
///
/// [`NotFinite`]: Self::NotFinite
/// [`MinAboveMax`]: Self::MinAboveMax
/// [`OutOfRange`]: Self::OutOfRange
/// [`Object`]: Self::Object
#[derive(Debug)]
#[non_exhaustive]
pub enum Refusal {
    /// The object's constructor or setter refused the value, with the error
    /// a write of it would get.
    Object(Error),
    /// Another point, the Device or a declared Network Port has this
    /// identifier.
    DuplicateIdentifier,
    /// Another point, the Device or a declared Network Port has this name.
    DuplicateName,
    /// The name is empty.
    EmptyName,
    /// The builder doesn't build objects of this type.
    TypeNotCovered,
    /// The kind is another family's than the object type's.
    KindMismatch,
    /// The type doesn't take this access: an Input is read-only and an
    /// Output commandable.
    AccessNotAllowed,
    /// A Real is infinite or not a number.
    NotFinite,
    /// Min_Pres_Value is above Max_Pres_Value.
    MinAboveMax,
    /// Units past 65535, or a state count of none or past
    /// [`MAX_NUMBER_OF_STATES`].
    OutOfRange,
    /// State_Text was given with other than one label per state.
    StateTextCount,
    /// The list's Device instance isn't the identity's.
    DeviceMismatch,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Object(error) => return write!(f, "{error}"),
            Self::DuplicateIdentifier => "identifier already in use",
            Self::DuplicateName => "name already in use",
            Self::EmptyName => "name is empty",
            Self::TypeNotCovered => "object type not covered by point lists",
            Self::KindMismatch => "point kind does not fit the object type",
            Self::AccessNotAllowed => "access not allowed for the object type",
            Self::NotFinite => "value is not finite",
            Self::MinAboveMax => "Min_Pres_Value is above Max_Pres_Value",
            Self::OutOfRange => "value out of range",
            Self::StateTextCount => "State_Text count differs from Number_Of_States",
            Self::DeviceMismatch => "Device instance differs from the identity's",
        };
        f.write_str(text)
    }
}

/// Build the database `identity` serves with the points of `list`.
///
/// The Device takes the list's name, if it gives one, and its Description;
/// the rest of it, and the Network Ports, come from `identity`. The points
/// follow in list order. See the [module](self) for what is refused.
///
/// The list is taken by value: each point's name, description, state
/// texts and active and inactive texts move into its object, and the
/// builder copies none of them. The database's name index keeps its own
/// copy of every Object_Name, as [`ObjectDatabase::add`] always does, and
/// the Device's name reaches the Device through a copy of `identity`.
///
/// Nothing serves the database it returns, so the host can set values,
/// Reliability and Out_Of_Service on the built objects before starting a
/// server, by the routes the [module](self) names.
pub fn build_database(
    list: PointList,
    identity: &DeviceIdentity,
) -> Result<ObjectDatabase, BuildError> {
    let PointList { device, points } = list;
    let device_oid = identity.device_oid();
    let refuse = |property: P, reason: Refusal| BuildError::new(device_oid, Some(property), reason);
    if device.instance != identity.instance() {
        return Err(refuse(P::OBJECT_IDENTIFIER, Refusal::DeviceMismatch));
    }
    let device_name = device.name.as_deref().unwrap_or(identity.name());
    if device_name.is_empty() {
        return Err(refuse(P::OBJECT_NAME, Refusal::EmptyName));
    }
    check_unique(identity, device_name, &points)?;

    let mut objects = Vec::with_capacity(points.len());
    for point in points {
        objects.push(build_object(point)?);
    }
    let renamed;
    let identity = match device.name {
        Some(name) => {
            renamed = identity.clone().with_name(name);
            &renamed
        }
        None => identity,
    };
    // The identifiers are unique now, so only the identity itself can be
    // refused here.
    let mut database =
        build_database_with_extra(identity, objects).map_err(refused_by(device_oid, None))?;
    if let Some(description) = device.description
        && let Some(object) = database.get_mut(&device_oid)
    {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString(description),
                None,
            )
            .map_err(refused_by(device_oid, Some(P::DESCRIPTION)))?;
    }
    Ok(database)
}

/// Check that no point reuses an identifier or a name the Device, a
/// declared Network Port or an earlier point has, and that no name is
/// empty; a refusal names the later point.
fn check_unique(
    identity: &DeviceIdentity,
    device_name: &str,
    points: &[Point],
) -> Result<(), BuildError> {
    let ports = identity.network_ports();
    let port_names: Vec<String> = ports.iter().map(NetworkPortEntry::object_name).collect();
    let capacity = 1 + ports.len() + points.len();
    let mut identifiers = HashSet::with_capacity(capacity);
    let mut names = HashSet::with_capacity(capacity);
    identifiers.insert(identity.device_oid());
    names.insert(device_name);
    for (port, name) in ports.iter().zip(&port_names) {
        // The identity checked every port's instance when it took the port.
        if let Ok(oid) = ObjectIdentifier::new(ObjectType::NETWORK_PORT, port.instance) {
            identifiers.insert(oid);
        }
        // Ports have distinct instances, so only the Device's name can
        // already be here.
        if !names.insert(name.as_str()) {
            return Err(BuildError::new(
                identity.device_oid(),
                Some(P::OBJECT_NAME),
                Refusal::DuplicateName,
            ));
        }
    }
    for point in points {
        let refuse = |property: P, reason| BuildError::new(point.object, Some(property), reason);
        if !identifiers.insert(point.object) {
            return Err(refuse(P::OBJECT_IDENTIFIER, Refusal::DuplicateIdentifier));
        }
        if point.name.is_empty() {
            return Err(refuse(P::OBJECT_NAME, Refusal::EmptyName));
        }
        if !names.insert(point.name.as_str()) {
            return Err(refuse(P::OBJECT_NAME, Refusal::DuplicateName));
        }
    }
    Ok(())
}

/// Build the object `point` describes.
///
/// It checks the point alone: whether its identifier and name are free is
/// up to the database it joins, which replaces an object under an
/// identifier it holds. [`build_database`] checks both.
pub fn build_object(point: Point) -> Result<Box<dyn BACnetObject>, BuildError> {
    let Point {
        object,
        name,
        description,
        kind,
    } = point;
    let refuse = |property: Option<P>, reason| BuildError::new(object, property, reason);
    if name.is_empty() {
        return Err(refuse(Some(P::OBJECT_NAME), Refusal::EmptyName));
    }
    if let Err(error) =
        ObjectIdentifier::new_addressable(object.object_type(), object.instance_number())
    {
        return Err(refuse(Some(P::OBJECT_IDENTIFIER), Refusal::Object(error)));
    }
    let Some((family, role)) = family_of(object.object_type()) else {
        return Err(refuse(None, Refusal::TypeNotCovered));
    };
    let object = Named {
        object,
        name,
        description,
        role,
    };
    match (family, kind) {
        (Family::Analog, PointKind::Analog(analog)) => analog_object(object, analog),
        (Family::Binary, PointKind::Binary(binary)) => binary_object(object, binary),
        (Family::MultiState, PointKind::MultiState(multi_state)) => {
            multi_state_object(object, multi_state)
        }
        (Family::Integer, PointKind::Integer(integer)) => integer_object(object, integer),
        _ => Err(refuse(None, Refusal::KindMismatch)),
    }
}

/// The families the builder covers.
#[derive(Clone, Copy)]
enum Family {
    Analog,
    Binary,
    MultiState,
    /// The Integer Value alone, a Value.
    Integer,
}

/// Which of a family's types an object is.
#[derive(Clone, Copy)]
enum Role {
    Input,
    Output,
    Value,
}

/// The family and role of `object_type`, if the builder covers it.
fn family_of(object_type: ObjectType) -> Option<(Family, Role)> {
    Some(match object_type {
        ObjectType::ANALOG_INPUT => (Family::Analog, Role::Input),
        ObjectType::ANALOG_OUTPUT => (Family::Analog, Role::Output),
        ObjectType::ANALOG_VALUE => (Family::Analog, Role::Value),
        ObjectType::BINARY_INPUT => (Family::Binary, Role::Input),
        ObjectType::BINARY_OUTPUT => (Family::Binary, Role::Output),
        ObjectType::BINARY_VALUE => (Family::Binary, Role::Value),
        ObjectType::MULTI_STATE_INPUT => (Family::MultiState, Role::Input),
        ObjectType::MULTI_STATE_OUTPUT => (Family::MultiState, Role::Output),
        ObjectType::MULTI_STATE_VALUE => (Family::MultiState, Role::Value),
        ObjectType::INTEGER_VALUE => (Family::Integer, Role::Value),
        _ => return None,
    })
}

/// What every point has, once its type is known to be covered.
struct Named {
    object: ObjectIdentifier,
    name: String,
    description: Option<String>,
    role: Role,
}

/// The Present_Value access `role` takes `access` as, with the relinquish
/// default it gives: an Input only read-only, an Output only commandable.
fn admit<T>(role: Role, access: Access<T>) -> Result<(PresentValueAccess, Option<T>), Refusal> {
    match (role, access) {
        (Role::Input | Role::Value, Access::ReadOnly) => Ok((PresentValueAccess::ReadOnly, None)),
        (Role::Value, Access::Writable) => Ok((PresentValueAccess::Writable, None)),
        (Role::Output | Role::Value, Access::Commandable { relinquish_default }) => {
            Ok((PresentValueAccess::Commandable, relinquish_default))
        }
        _ => Err(Refusal::AccessNotAllowed),
    }
}

/// A refusal by `object`'s constructor (`property` `None`) or by the setter
/// of `property`.
fn refused_by(object: ObjectIdentifier, property: Option<P>) -> impl FnOnce(Error) -> BuildError {
    move |error| BuildError::new(object, property, Refusal::Object(error))
}

/// Refuse the first of `values` that is set and isn't finite.
fn check_finite(object: ObjectIdentifier, values: &[(Option<f32>, P)]) -> Result<(), BuildError> {
    match values
        .iter()
        .find(|(value, _)| value.is_some_and(|value| !value.is_finite()))
    {
        Some(&(_, property)) => Err(BuildError::new(object, Some(property), Refusal::NotFinite)),
        None => Ok(()),
    }
}

fn analog_object(named: Named, analog: Analog) -> Result<Box<dyn BACnetObject>, BuildError> {
    let Named {
        object,
        name,
        description,
        role,
    } = named;
    let refuse = |property: P, reason| BuildError::new(object, Some(property), reason);
    let Analog {
        units,
        cov_increment,
        min_pres_value,
        max_pres_value,
        access,
        ..
    } = analog;
    let (access, relinquish_default) =
        admit(role, access).map_err(|reason| refuse(P::PRESENT_VALUE, reason))?;
    if units.to_raw() > 65_535 {
        return Err(refuse(P::UNITS, Refusal::OutOfRange));
    }
    check_finite(
        object,
        &[
            (cov_increment, P::COV_INCREMENT),
            (min_pres_value, P::MIN_PRES_VALUE),
            (max_pres_value, P::MAX_PRES_VALUE),
            (relinquish_default, P::RELINQUISH_DEFAULT),
        ],
    )?;
    if let (Some(min), Some(max)) = (min_pres_value, max_pres_value)
        && min > max
    {
        return Err(refuse(P::MIN_PRES_VALUE, Refusal::MinAboveMax));
    }
    let instance = object.instance_number();
    // The settings all three types share.
    macro_rules! configure {
        ($built:ident) => {
            if let Some(description) = description {
                $built.set_description(description);
            }
            if let Some(increment) = cov_increment {
                $built
                    .set_cov_increment(increment)
                    .map_err(refused_by(object, Some(P::COV_INCREMENT)))?;
            }
            if let Some(min) = min_pres_value {
                $built.set_min_pres_value(min);
            }
            if let Some(max) = max_pres_value {
                $built.set_max_pres_value(max);
            }
        };
    }
    let constructed = refused_by(object, None);
    let relinquish = refused_by(object, Some(P::RELINQUISH_DEFAULT));
    Ok(match role {
        Role::Input => {
            let mut built = AnalogInputObject::new(instance, name, units).map_err(constructed)?;
            configure!(built);
            Box::new(built)
        }
        Role::Output => {
            let mut built = AnalogOutputObject::new(instance, name, units).map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built.set_relinquish_default(default).map_err(relinquish)?;
            }
            Box::new(built)
        }
        Role::Value => {
            let mut built = AnalogValueObject::with_access(instance, name, units, access)
                .map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built.set_relinquish_default(default).map_err(relinquish)?;
            }
            Box::new(built)
        }
    })
}

fn binary_object(named: Named, binary: Binary) -> Result<Box<dyn BACnetObject>, BuildError> {
    let Named {
        object,
        name,
        description,
        role,
    } = named;
    let Binary {
        active_text,
        inactive_text,
        access,
        ..
    } = binary;
    let (access, relinquish_default) = admit(role, access)
        .map_err(|reason| BuildError::new(object, Some(P::PRESENT_VALUE), reason))?;
    let instance = object.instance_number();
    macro_rules! configure {
        ($built:ident) => {
            if let Some(description) = description {
                $built.set_description(description);
            }
            if let Some(text) = active_text {
                $built.set_active_text(text);
            }
            if let Some(text) = inactive_text {
                $built.set_inactive_text(text);
            }
        };
    }
    let constructed = refused_by(object, None);
    let relinquish = refused_by(object, Some(P::RELINQUISH_DEFAULT));
    Ok(match role {
        Role::Input => {
            let mut built = BinaryInputObject::new(instance, name).map_err(constructed)?;
            configure!(built);
            Box::new(built)
        }
        Role::Output => {
            let mut built = BinaryOutputObject::new(instance, name).map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built
                    .set_relinquish_default(default.to_raw())
                    .map_err(relinquish)?;
            }
            Box::new(built)
        }
        Role::Value => {
            let mut built =
                BinaryValueObject::with_access(instance, name, access).map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built
                    .set_relinquish_default(default.to_raw())
                    .map_err(relinquish)?;
            }
            Box::new(built)
        }
    })
}

fn multi_state_object(
    named: Named,
    multi_state: MultiState,
) -> Result<Box<dyn BACnetObject>, BuildError> {
    let Named {
        object,
        name,
        description,
        role,
    } = named;
    let refuse = |property: P, reason| BuildError::new(object, Some(property), reason);
    let MultiState {
        number_of_states,
        state_texts,
        access,
        ..
    } = multi_state;
    let (access, relinquish_default) =
        admit(role, access).map_err(|reason| refuse(P::PRESENT_VALUE, reason))?;
    if !(1..=MAX_NUMBER_OF_STATES).contains(&number_of_states) {
        return Err(refuse(P::NUMBER_OF_STATES, Refusal::OutOfRange));
    }
    if !state_texts.is_empty() && state_texts.len() != number_of_states as usize {
        return Err(refuse(P::STATE_TEXT, Refusal::StateTextCount));
    }
    let instance = object.instance_number();
    // The labels go in before the relinquish default, which must name one
    // of the states.
    macro_rules! configure {
        ($built:ident) => {
            if let Some(description) = description {
                $built.set_description(description);
            }
            if !state_texts.is_empty() {
                $built
                    .set_state_texts(state_texts)
                    .map_err(refused_by(object, Some(P::STATE_TEXT)))?;
            }
        };
    }
    let constructed = refused_by(object, None);
    let relinquish = refused_by(object, Some(P::RELINQUISH_DEFAULT));
    Ok(match role {
        Role::Input => {
            let mut built = MultiStateInputObject::new(instance, name, number_of_states)
                .map_err(constructed)?;
            configure!(built);
            Box::new(built)
        }
        Role::Output => {
            let mut built = MultiStateOutputObject::new(instance, name, number_of_states)
                .map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built.set_relinquish_default(default).map_err(relinquish)?;
            }
            Box::new(built)
        }
        Role::Value => {
            let mut built =
                MultiStateValueObject::with_access(instance, name, number_of_states, access)
                    .map_err(constructed)?;
            configure!(built);
            if let Some(default) = relinquish_default {
                built.set_relinquish_default(default).map_err(relinquish)?;
            }
            Box::new(built)
        }
    })
}

fn integer_object(named: Named, integer: Integer) -> Result<Box<dyn BACnetObject>, BuildError> {
    let Named {
        object,
        name,
        description,
        role,
    } = named;
    let refuse = |property: P, reason| BuildError::new(object, Some(property), reason);
    let Integer {
        units,
        cov_increment,
        min_pres_value,
        max_pres_value,
        access,
        ..
    } = integer;
    let (access, relinquish_default) =
        admit(role, access).map_err(|reason| refuse(P::PRESENT_VALUE, reason))?;
    if units.to_raw() > 65_535 {
        return Err(refuse(P::UNITS, Refusal::OutOfRange));
    }
    if let (Some(min), Some(max)) = (min_pres_value, max_pres_value)
        && min > max
    {
        return Err(refuse(P::MIN_PRES_VALUE, Refusal::MinAboveMax));
    }
    let mut built = IntegerValueObject::with_access(object.instance_number(), name, access)
        .map_err(refused_by(object, None))?;
    built
        .set_units(units)
        .map_err(refused_by(object, Some(P::UNITS)))?;
    if let Some(description) = description {
        built.set_description(description);
    }
    if let Some(increment) = cov_increment {
        built
            .set_cov_increment(increment)
            .map_err(refused_by(object, Some(P::COV_INCREMENT)))?;
    }
    if let Some(min) = min_pres_value {
        built.set_min_pres_value(min);
    }
    if let Some(max) = max_pres_value {
        built.set_max_pres_value(max);
    }
    if let Some(default) = relinquish_default {
        built
            .set_relinquish_default(default)
            .map_err(refused_by(object, Some(P::RELINQUISH_DEFAULT)))?;
    }
    Ok(Box::new(built))
}

#[cfg(test)]
mod tests;
