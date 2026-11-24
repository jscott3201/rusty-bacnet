//! The one registration path of the nine point builders (#2711):
//! `BACnetServer` and the three endpoints read their `add_analog_input` ...
//! `add_multistate_value` arguments into a [`Registration`] and pass it to
//! [`admit`], which makes every check in one order on both surfaces before
//! [`Registration::build`] builds it.
//!
//! The two surfaces differ only in when they build. `BACnetServer` builds
//! once, with its `storage_path` keywords ([`Storage`]), and keeps the
//! object until start. An endpoint builds once to check the registration,
//! keeps the [`Registration`], and builds it again from it on each start
//! attempt, so a failed or cancelled start leaves every registration for
//! the next. A `Registration` is plain data, so that rebuild needs no
//! closure or trait.
//!
//! `bacnet_objects::points` (#2445) describes points too, for a device
//! built from a point list, but it has no room for what these builders
//! take: Polarity, the event algorithm parameters, the profile rows, the
//! Audit policy, Value_Source tracking and storage. So the binding keeps
//! its own type.

use bacnet_objects::audit::ObjectAuditPolicy;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_types::enums::{ObjectType, Polarity};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::algorithm_keywords::AlgorithmKeywords;
use crate::errors::to_py_err;
use crate::object_options::present_value_access;

mod build;
#[cfg(test)]
mod tests;

/// One point registration: what its builder call gave, parsed.
#[derive(Clone, Debug)]
pub(crate) struct Registration {
    pub(crate) instance: u32,
    pub(crate) name: String,
    pub(crate) kind: RegistrationKind,
    /// The `tags`, `profile_location` and `profile_name` keywords (#1553,
    /// #1867).
    pub(crate) profile: Option<ObjectProfile>,
    /// The event algorithm parameter keywords (#2410).
    pub(crate) parameters: AlgorithmKeywords,
}

/// What each of the nine object types takes beyond the instance, the name,
/// the profile and the parameter keywords.
///
/// `present_value` is the initial value. On a commandable object it sets
/// Relinquish_Default and the priority array starts empty; on any other it
/// sets Present_Value, checked as a local input is, so an Analog Input's
/// NaN or infinity is refused. `None` keeps the type's default.
#[derive(Clone, Debug)]
pub(crate) enum RegistrationKind {
    AnalogInput {
        units: u32,
        present_value: f32,
    },
    AnalogOutput {
        units: u32,
        present_value: Option<f32>,
    },
    AnalogValue {
        units: u32,
        value: ValueKeywords,
        present_value: Option<f32>,
    },
    BinaryInput {
        polarity: Polarity,
        present_value: Option<u32>,
    },
    BinaryOutput {
        polarity: Polarity,
        present_value: Option<u32>,
    },
    BinaryValue {
        value: ValueKeywords,
        present_value: Option<u32>,
    },
    MultiStateInput {
        number_of_states: u32,
        present_value: Option<u32>,
    },
    MultiStateOutput {
        number_of_states: u32,
        present_value: Option<u32>,
    },
    /// A Multi-state Value takes no Audit keywords.
    MultiStateValue {
        number_of_states: u32,
        access: PresentValueAccess,
        value_source_tracking: bool,
        present_value: Option<u32>,
    },
}

/// The Value-object keywords (#1552, #1867): how Present_Value is written,
/// whether Value_Source is served, and the optional Audit policy.
#[derive(Clone, Debug)]
pub(crate) struct ValueKeywords {
    pub(crate) access: PresentValueAccess,
    pub(crate) value_source_tracking: bool,
    pub(crate) audit_policy: ObjectAuditPolicy,
}

/// The Value keywords an Analog or Binary Value builder was given, checked.
pub(crate) fn value_keywords(
    access: &str,
    value_source_tracking: bool,
    audit_level: Option<&str>,
    auditable_operations: Option<&Bound<'_, PyAny>>,
    audit_priority_filter: Option<&Bound<'_, PyAny>>,
) -> PyResult<ValueKeywords> {
    Ok(ValueKeywords {
        access: present_value_access(access, value_source_tracking)?,
        value_source_tracking,
        audit_policy: crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?,
    })
}

/// Where a server's point keeps what a peer writes to it (#2331, #2364):
/// `storage_path`, and `clear_written_reporting` to drop what was kept
/// first. An endpoint's points keep nothing ([`Storage::NONE`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Storage<'a> {
    pub(crate) path: Option<&'a str>,
    pub(crate) clear: bool,
}

impl Storage<'_> {
    /// No storage: what an endpoint's points have.
    pub(crate) const NONE: Self = Self {
        path: None,
        clear: false,
    };
}

impl Registration {
    /// The registration one builder call describes.
    pub(crate) fn new(
        instance: u32,
        name: &str,
        kind: RegistrationKind,
        profile: Option<ObjectProfile>,
        parameters: AlgorithmKeywords,
    ) -> Self {
        Self {
            instance,
            name: name.to_string(),
            kind,
            profile,
            parameters,
        }
    }

    /// The object type this point builds.
    pub(crate) fn object_type(&self) -> ObjectType {
        match self.kind {
            RegistrationKind::AnalogInput { .. } => ObjectType::ANALOG_INPUT,
            RegistrationKind::AnalogOutput { .. } => ObjectType::ANALOG_OUTPUT,
            RegistrationKind::AnalogValue { .. } => ObjectType::ANALOG_VALUE,
            RegistrationKind::BinaryInput { .. } => ObjectType::BINARY_INPUT,
            RegistrationKind::BinaryOutput { .. } => ObjectType::BINARY_OUTPUT,
            RegistrationKind::BinaryValue { .. } => ObjectType::BINARY_VALUE,
            RegistrationKind::MultiStateInput { .. } => ObjectType::MULTI_STATE_INPUT,
            RegistrationKind::MultiStateOutput { .. } => ObjectType::MULTI_STATE_OUTPUT,
            RegistrationKind::MultiStateValue { .. } => ObjectType::MULTI_STATE_VALUE,
        }
    }

    /// The identity the uniqueness checks compare: type, instance, name.
    pub(crate) fn identity(&self) -> (ObjectType, u32, &str) {
        (self.object_type(), self.instance, &self.name)
    }

    /// The checks a build would make after storage attaches, made first:
    /// the profile rows, which the object provisions last and refuses only
    /// as [`ObjectProfile::check`] does. Every other refusal happens in the
    /// build before storage attaches (see `build`), so once this passes, a
    /// refused registration has touched no `storage_path` file.
    pub(crate) fn check(&self) -> PyResult<()> {
        match &self.profile {
            Some(profile) => profile.check().map_err(to_py_err),
            None => Ok(()),
        }
    }
}

/// Admit one registration, in the order both surfaces check it (#2711):
/// an instance an object may have ([`check_identifier`]), an identifier
/// and a name no object in `taken` has ([`check_unique`]), the checks
/// `check` makes without building, then `build`. Each surface calls it
/// under the lock that its start seals, after refusing a call that comes
/// too late, and keeps what `build` gives back or the registration itself.
/// So one call with two faults raises the same error on either surface.
pub(crate) fn admit<'a, T>(
    (object_type, instance, name): (ObjectType, u32, &str),
    taken: impl IntoIterator<Item = (ObjectType, u32, &'a str)>,
    check: impl FnOnce() -> PyResult<()>,
    build: impl FnOnce() -> PyResult<T>,
) -> PyResult<T> {
    check_identifier(object_type, instance)?;
    check_unique(object_type, instance, name, taken)?;
    check()?;
    build()
}

/// Refuse an instance no object may have: past 4194303, or 4194303
/// itself, which names no particular object (#2171). Raises BacnetError,
/// as the object constructors do for an instance out of range.
pub(crate) fn check_identifier(object_type: ObjectType, instance: u32) -> PyResult<()> {
    ObjectIdentifier::new_addressable(object_type, instance)
        .map(|_| ())
        .map_err(to_py_err)
}

/// Refuse `name` or the identifier (`object_type`, `instance`) when an
/// object in `taken` already has it: the Device, an endpoint's declared
/// Network Ports and every registration kept so far. ValueError, named for
/// what clashed: each object in turn, its identifier before its name.
pub(crate) fn check_unique<'a>(
    object_type: ObjectType,
    instance: u32,
    name: &str,
    taken: impl IntoIterator<Item = (ObjectType, u32, &'a str)>,
) -> PyResult<()> {
    for (other_type, other_instance, other_name) in taken {
        if (object_type, instance) == (other_type, other_instance) {
            return Err(PyValueError::new_err(format!(
                "duplicate object identifier: {object_type:?} {instance}"
            )));
        }
        if name == other_name {
            return Err(PyValueError::new_err(format!(
                "duplicate object name: {name}"
            )));
        }
    }
    Ok(())
}
