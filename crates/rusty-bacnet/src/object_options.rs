//! Keywords the object builders share (#1552, #1553, #1554): how a Value
//! object's Present_Value is written and whether its source is tracked, and
//! the optional Tags, Profile_Location and Profile_Name rows.

use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::present_value_access::PresentValueAccess;
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
    match crate::types::object_profile_from_py(tags, profile_location, profile_name)? {
        Some(profile) => set_profile(profile).map_err(to_py_err),
        None => Ok(()),
    }
}
