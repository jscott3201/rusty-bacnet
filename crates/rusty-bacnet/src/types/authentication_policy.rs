//! Python boundary for an Access Point's authentication policies (#1325).
//!
//! A policy is an `(entries, order_enforced, timeout)` triple, the form a
//! typed read of an Authentication_Policy_List element gives: `entries` is a
//! list of `(reference, index)` pairs, each reference a Credential Data
//! Input in the forms `door_members` takes, `order_enforced` a bool and
//! `timeout` the seconds allowed, 0 for no limit. `add_access_point` takes
//! the policies as `(name, policy)` pairs, the name being the matching
//! Authentication_Policy_Names element.
//!
//! This layer checks shapes, Python types, integer ranges and the device
//! member of each reference (a non-Device raises ValueError, as for
//! `door_members`, #1285). Whether a policy is usable is the Access Point's
//! call: a malformed one is stored and leaves the point reporting a
//! configuration error while it is the one in effect.

use bacnet_types::constructed::{BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool};

use super::access_rule::device_object_reference;

/// Read `policies`, a list of `(name, policy)` pairs; `name` is the keyword
/// the errors name.
pub(crate) fn authentication_policies_from_py(
    policies: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<Vec<(String, BACnetAuthenticationPolicy)>> {
    let policies: Vec<Bound<'_, PyAny>> = policies
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("{name} must be a list")))?;
    policies
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            let name = format!("{name}[{index}]");
            let (label, policy) = pair.extract::<(String, Bound<'_, PyAny>)>().map_err(|_| {
                PyTypeError::new_err(format!("{name} must be a (name, policy) pair"))
            })?;
            Ok((label, authentication_policy_from_py(&policy, &name)?))
        })
        .collect()
}

/// Read one `(entries, order_enforced, timeout)` policy; `name` is what the
/// errors name. An index or timeout outside Unsigned32 raises
/// OverflowError.
pub(crate) fn authentication_policy_from_py(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetAuthenticationPolicy> {
    let (entries, order_enforced, timeout) = value
        .extract::<(Bound<'_, PyAny>, Bound<'_, PyAny>, Bound<'_, PyAny>)>()
        .map_err(|_| {
            PyTypeError::new_err(format!(
                "{name} must be an (entries, order_enforced, timeout) tuple"
            ))
        })?;
    if !order_enforced.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} order_enforced must be a bool"
        )));
    }
    let entries: Vec<Bound<'_, PyAny>> = entries
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("{name} entries must be a list")))?;
    let policy = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = format!("{name} entries[{index}]");
            let (reference, step) = entry
                .extract::<(Bound<'_, PyAny>, Bound<'_, PyAny>)>()
                .map_err(|_| {
                    PyTypeError::new_err(format!("{name} must be a (reference, index) pair"))
                })?;
            Ok(BACnetAuthenticationPolicyEntry {
                credential_data_input: device_object_reference(&reference, &name)?,
                index: step.extract()?,
            })
        })
        .collect::<PyResult<_>>()?;
    Ok(BACnetAuthenticationPolicy {
        policy,
        order_enforced: order_enforced.extract()?,
        timeout: timeout.extract()?,
    })
}

#[cfg(test)]
#[path = "authentication_policy_tests.rs"]
mod tests;
