//! The `members` argument of `add_group`, shared by `BACnetServer` and the
//! endpoint owners: the `read_property_multiple` spec shape, checked member by
//! member through `GroupObject::add_member`.

use bacnet_objects::group::GroupObject;
use bacnet_types::constructed::ReadAccessSpecification;
use pyo3::PyResult;
use pyo3::exceptions::PyValueError;

use crate::errors::to_py_err;
use crate::types::{PyReadAccessSpec, py_to_rpm_specs};

/// Convert `members`, reusing the `read_property_multiple` spec conversion;
/// `None` is a Group with no members.
pub(crate) fn members(members: Option<Vec<PyReadAccessSpec>>) -> Vec<ReadAccessSpecification> {
    py_to_rpm_specs(members.unwrap_or_default())
}

/// Build a Group with its List_Of_Group_Members in order.
///
/// Each member goes through `GroupObject::add_member`. A refused member is a
/// ValueError naming its position and the rule it breaks: no properties, or
/// another group's Present_Value.
pub(crate) fn group(
    instance: u32,
    name: &str,
    members: &[ReadAccessSpecification],
) -> PyResult<GroupObject> {
    let mut object = GroupObject::new(instance, name).map_err(to_py_err)?;
    for (position, member) in members.iter().enumerate() {
        object.add_member(member.clone()).map_err(|refusal| {
            PyValueError::new_err(format!("group member {position}: {refusal}"))
        })?;
    }
    Ok(object)
}
