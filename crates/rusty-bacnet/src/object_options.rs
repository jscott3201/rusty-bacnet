//! Keywords the object builders share (#1552, #1553, #1554): how a Value
//! object's Present_Value is written and whether its source is tracked, the
//! optional Tags, Profile_Location and Profile_Name rows, and the
//! `storage_path` and `clear_written_reporting` that keep the reporting
//! settings and Description a peer writes (#2331, #2364).

use std::sync::Arc;

use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::write_config::{FileWriteConfigPersistence, WriteConfigPersistence};
use bacnet_types::error::Error;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::errors::to_py_err;

/// The access `present_value_access` names, checked against
/// `value_source_tracking`: a commandable object already tracks its writers
/// through Priority_Array, so asking it to track a single source is a
/// ValueError rather than a silent no-op.
pub(crate) fn present_value_access(
    access: &str,
    value_source_tracking: bool,
) -> PyResult<PresentValueAccess> {
    let access = match access {
        "commandable" => PresentValueAccess::Commandable,
        "writable" => PresentValueAccess::Writable,
        "read_only" => PresentValueAccess::ReadOnly,
        _ => {
            return Err(PyValueError::new_err(
                "present_value_access must be 'commandable', 'writable' or 'read_only'",
            ));
        }
    };
    if value_source_tracking && access == PresentValueAccess::Commandable {
        return Err(PyValueError::new_err(
            "value_source_tracking needs present_value_access 'writable' or 'read_only': \
             a commandable object tracks its sources through Priority_Array",
        ));
    }
    Ok(access)
}

/// Provision the rows the `tags`, `profile_location` and `profile_name`
/// keywords ask for through the object's own `set_profile`, which refuses a
/// profile it doesn't take with a protocol error. Nothing is provisioned when
/// all three are absent.
pub(crate) fn provision_profile(
    tags: Option<&Bound<'_, PyAny>>,
    profile_location: Option<String>,
    profile_name: Option<String>,
    set_profile: impl FnOnce(ObjectProfile) -> Result<(), Error>,
) -> PyResult<()> {
    apply_profile(
        object_profile(tags, profile_location, profile_name)?,
        set_profile,
    )
}

/// The profile the `tags`, `profile_location` and `profile_name` keywords
/// describe, or `None` when all three are absent. A builder that rebuilds
/// its objects later, as the endpoints do at each start, keeps this and
/// hands it to [`apply_profile`] then (#1867).
pub(crate) fn object_profile(
    tags: Option<&Bound<'_, PyAny>>,
    profile_location: Option<String>,
    profile_name: Option<String>,
) -> PyResult<Option<ObjectProfile>> {
    crate::types::object_profile_from_py(tags, profile_location, profile_name)
}

/// Provision a parsed [`object_profile`] through the object's own
/// `set_profile`; nothing when it is `None`.
pub(crate) fn apply_profile(
    profile: Option<ObjectProfile>,
    set_profile: impl FnOnce(ObjectProfile) -> Result<(), Error>,
) -> PyResult<()> {
    match profile {
        Some(profile) => set_profile(profile).map_err(to_py_err),
        None => Ok(()),
    }
}

/// Build an object with `build`, keeping its reporting settings and a
/// written Description in the file at `storage_path` when there is one
/// (#2331, #2364). With `clear`, the settings a
/// write saved are dropped first and the object is opened again on the
/// cleared file, so it starts from its defaults, as `add_timer` does. Every
/// server builder of an object with intrinsic reporting takes it.
pub(crate) fn keep_reporting<T>(
    build: impl Fn() -> Result<T, Error>,
    storage_path: Option<&str>,
    clear: bool,
    attach: fn(T, Arc<dyn WriteConfigPersistence>) -> Result<T, Error>,
    clear_written: fn(&mut T) -> Result<(), Error>,
) -> PyResult<T> {
    let Some(path) = storage_path else {
        return build().map_err(to_py_err);
    };
    let storage: Arc<dyn WriteConfigPersistence> =
        Arc::new(FileWriteConfigPersistence::new(path).map_err(to_py_err)?);
    let open = || build().and_then(|object| attach(object, storage.clone()));
    let mut object = open().map_err(to_py_err)?;
    if clear {
        clear_written(&mut object).map_err(to_py_err)?;
        // Dropping it waits for its saves.
        drop(object);
        object = open().map_err(to_py_err)?;
    }
    Ok(object)
}
