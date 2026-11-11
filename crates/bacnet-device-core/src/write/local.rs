//! The application's own writes, made in the device it runs in (#2446).
//!
//! [`apply`] is the staged local write: under the host's database guard it
//! checks one [`LocalChange`] and commits it, and says whether the object
//! changed. It holds no lock, reads no clock and does none of the work a
//! change owes afterwards: the host runs its event pass and COV reporting
//! once the object holds the value. `bacnet-server`'s `write_local`,
//! `set_present_value_local` and its other `*_local` inputs make their change
//! through it.
//!
//! A local write is not [`commit_attempt`](super::commit_attempt), the
//! network commit, and the two differ (#2446): here a NULL is told by its
//! value, not its octets; a write of an Audit Reporter's Description,
//! Maximum_Send_Delay or Send_Now takes the object's ordinary writer, not
//! the reporter's own route; the Device's Audit recipient gets no route of
//! its own; and the observer's [`applied`](WriteCommitObserver::applied) is
//! never called.
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::{BACnetObject, LocalInputValue, not_supported};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{Applied, WriteCommitObserver, WriteTarget, relinquish};

/// Initiator asserted by a trusted local caller. The local Device (the
/// identity's, in a running server) owns the command in either case; an
/// object changes only its published source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandSource {
    /// The concrete local Device initiated the command.
    ServerDevice,
    /// An existing concrete local object initiated the command.
    Object(ObjectIdentifier),
}

/// The command origin `source` names in `db`: the local Device as owner,
/// with the initiating object for [`LocalCommandSource::Object`].
///
/// PROPERTY / WRITE_ACCESS_DENIED when the database's local Device
/// ([`ObjectDatabase::local_device`]) is unknown or its object isn't here,
/// or when the initiating object isn't here.
pub fn resolve_source(
    db: &ObjectDatabase,
    source: LocalCommandSource,
) -> Result<CommandOrigin, Error> {
    let denied = || Error::protocol(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    // The Device this server speaks for, not the lowest one (#2227), and
    // only while its object is here.
    let owner_device = db
        .local_device()
        .identifier()
        .filter(|device| db.get(device).is_some())
        .ok_or_else(denied)?;
    let initiating_object = match source {
        LocalCommandSource::ServerDevice => None,
        LocalCommandSource::Object(oid) => {
            if db.get(&oid).is_none() {
                return Err(denied());
            }
            Some(oid)
        }
    };
    let origin = CommandOrigin::Local {
        owner_device,
        initiating_object,
    };
    origin.validate()?;
    Ok(origin)
}

/// Write `value` to `property` of `object` as `origin`, or with the
/// object's context-free writer when no origin resolved.
///
/// On absent or invalid local identity, context-free dispatch preserves
/// unrelated property writes while first-party tracked commands fail closed
/// in the object.
pub fn write_target(
    object: &mut dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
    priority: Option<u8>,
    origin: Option<&CommandOrigin>,
) -> Result<(), Error> {
    match origin {
        Some(origin) => object.write_property_from(property, index, value, priority, origin),
        None => object.write_property(property, index, value, priority),
    }
}

/// One change the application makes to an object of its own device.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LocalChange {
    /// An input the application supplies: Present_Value and the server's
    /// other `*_local` inputs, through the object's
    /// [`LocalInput`](bacnet_objects::traits::LocalInput). A Present_Value
    /// without an origin is given the local Device's, when one resolves, so
    /// a noncommandable Value that tracks its source publishes that Device
    /// (#1552); otherwise it is applied without one.
    Input(LocalInputValue),
    /// A network-equivalent property write made as `source`. Present_Value
    /// at a priority commands that slot, and NULL relinquishes it.
    Property {
        /// The property written.
        property: PropertyIdentifier,
        /// The array index written, if any.
        array_index: Option<u32>,
        /// The value written.
        value: PropertyValue,
        /// The write's priority, if any.
        priority: Option<u8>,
        /// Who asserts the write.
        source: LocalCommandSource,
    },
}

/// The host's observer for a [`LocalChange::Property`], with the octets the
/// [`WriteTarget`] it is shown carries.
pub struct Observed<'a> {
    /// Told of the write as [`commit_attempt`](super::commit_attempt) tells
    /// it, except [`applied`](WriteCommitObserver::applied), which a local
    /// write never calls.
    pub observer: &'a mut dyn WriteCommitObserver,
    /// The value's octets, as the host encodes it for the observer.
    pub octets: &'a [u8],
}

/// Check `change` to `oid`, then commit it.
///
/// The checks, in WriteProperty's order: the object exists (OBJECT /
/// UNKNOWN_OBJECT), and for a property write the array index fits the
/// property and a new Object_Name is free. Nothing has changed when one
/// fails, and the observer heard nothing.
///
/// A property write is then localized ([`localize`](super::localize)) and
/// made as its source resolves ([`resolve_source`]); an unresolved source
/// writes without an origin. An executor-owned Device property is refused
/// ([`check_executor_owned_write`](super::check_executor_owned_write)). The
/// observer, given only for a property write, gets
/// [`before`](WriteCommitObserver::before) and
/// [`commit_policy`](WriteCommitObserver::commit_policy) ahead of the
/// object's write, then [`committed`](WriteCommitObserver::committed) or
/// [`failed`](WriteCommitObserver::failed). An input reports to no
/// observer.
///
/// [`Applied::Unchanged`] is a NULL the property left as it was
/// ([`relinquish`]): a success the observer is told of as committed, after
/// which nothing follows. On [`Applied::Written`] after a write of
/// Object_Name the database's name index follows, and after a write of
/// Input_Reference the object's reference is judged again.
pub fn apply(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    change: LocalChange,
    observed: Option<Observed<'_>>,
) -> Result<Applied, Error> {
    precheck(db, oid, &change)?;
    match change {
        LocalChange::Input(input) => {
            // The application's own Present_Value comes from this Device,
            // which a tracked noncommandable Value publishes as its source
            // (#1552).
            let input = match input {
                LocalInputValue::PresentValue {
                    value,
                    origin: None,
                    ..
                } => match resolve_source(db, LocalCommandSource::ServerDevice) {
                    Ok(origin) => LocalInputValue::present_value_from(value, origin),
                    Err(_) => LocalInputValue::present_value(value),
                },
                input => input,
            };
            let object = db.get_mut(&oid).expect("existence checked above");
            apply_input(object, input)?;
            Ok(Applied::Written)
        }
        LocalChange::Property {
            property,
            array_index,
            value,
            priority,
            source,
        } => {
            let (mut observer, octets) = match observed {
                Some(Observed { observer, octets }) => (Some(observer), octets),
                None => (None, &[][..]),
            };
            let target = WriteTarget {
                oid,
                property,
                array_index,
                priority,
                value: octets,
            };
            if let Some(observer) = observer.as_deref_mut() {
                observer.before(db, target);
            }
            let value = super::localize(db, oid, property, value);
            let null = relinquish::is_null_value(&value);
            let prepared = observer
                .as_deref_mut()
                .and_then(|observer| observer.commit_policy(db, target, &value));
            let origin = resolve_source(db, source).ok();
            let result = prepared.unwrap_or_else(|| {
                super::check_executor_owned_write(oid, property)?;
                write_target(
                    db.get_mut(&oid).expect("existence checked above"),
                    property,
                    array_index,
                    value,
                    priority,
                    origin.as_ref(),
                )
            });
            if let Err(error) = result {
                // A NULL the property left as it was succeeds unchanged:
                // the observer records the write it is, and nothing changed.
                let object = db.get(&oid).expect("existence checked above");
                if null && relinquish::leaves_unchanged(object, property, array_index, &error) {
                    if let Some(observer) = observer {
                        observer.committed(db);
                    }
                    return Ok(Applied::Unchanged);
                }
                if let Some(observer) = observer {
                    observer.failed(db, &error);
                }
                return Err(error);
            }
            if let Some(observer) = observer {
                observer.committed(db);
            }
            if property == PropertyIdentifier::OBJECT_NAME {
                db.update_name_index(&oid);
            }
            // A Pulse Converter judges its new Input_Reference as it commits
            // (#1341).
            if property == PropertyIdentifier::INPUT_REFERENCE {
                db.check_input_reference(&oid);
            }
            Ok(Applied::Written)
        }
    }
}

/// Hand `input` to the object's [`LocalInput`](bacnet_objects::traits::LocalInput),
/// refusing it as the object would an input it doesn't take when the object
/// lends none.
fn apply_input(object: &mut dyn BACnetObject, input: LocalInputValue) -> Result<(), Error> {
    match object.local_input() {
        Some(local) => local.apply_local_input(input),
        None => Err(not_supported()),
    }
}

/// The checks made before the observer or the object sees the value, in
/// WriteProperty's order: the object exists, and for a property write, the
/// array index fits the property (WriteProperty's own
/// [`check_write_array_index`](bacnet_objects::property_metadata::check_write_array_index))
/// and a new Object_Name is free. The name check calls
/// [`ObjectDatabase::check_name_available`] directly, which is what
/// WriteProperty's [`check_and_prepare_name_write`](super::check_and_prepare_name_write)
/// runs for a CharacterString.
fn precheck(db: &ObjectDatabase, oid: ObjectIdentifier, change: &LocalChange) -> Result<(), Error> {
    let object = db.get(&oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;
    let LocalChange::Property {
        property,
        array_index,
        value,
        ..
    } = change
    else {
        return Ok(());
    };
    bacnet_objects::property_metadata::check_write_array_index(object, *property, *array_index)?;
    match value {
        PropertyValue::CharacterString(new_name)
            if *property == PropertyIdentifier::OBJECT_NAME =>
        {
            db.check_name_available(&oid, new_name)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
