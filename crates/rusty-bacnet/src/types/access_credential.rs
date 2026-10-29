//! Strict credential inputs and lossless mapping of native rights-stage evidence.

use bacnet_objects::access_control::{
    AccessRightsDecision, AccessRightsEvaluation, AccessRuleKind, AccessRulePosition,
    UnresolvedReason,
};
use bacnet_types::constructed::BACnetAssignedAccessRights;
use bacnet_types::enums::AuthorizationExemption;
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList};

use super::PyObjectIdentifier;
use super::access_rule::device_object_reference;
use super::constructed_py::object_reference_to_py;
use super::mapping::{fixed_integer, mapping, required_item, validate_keys};

pub(crate) fn assigned_access_rights_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<Vec<BACnetAssignedAccessRights>> {
    let entries: Vec<Bound<'_, PyAny>> = value
        .extract()
        .map_err(|_| PyTypeError::new_err("assigned_access_rights must be a list"))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = format!("assigned_access_rights[{index}]");
            let entry = mapping(entry, &name)?;
            validate_keys(entry, &name, &["assigned_access_rights", "enable"], &[])?;
            let enable = required_item(entry, &name, "enable")?;
            if !enable.is_instance_of::<PyBool>() {
                return Err(PyTypeError::new_err(format!(
                    "{name}.enable must be a bool"
                )));
            }
            Ok(BACnetAssignedAccessRights {
                assigned_access_rights: device_object_reference(
                    &required_item(entry, &name, "assigned_access_rights")?,
                    &format!("{name}.assigned_access_rights"),
                )?,
                enable: enable.extract()?,
            })
        })
        .collect()
}

pub(crate) fn authorization_exemptions_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<Vec<AuthorizationExemption>> {
    let values: Vec<Bound<'_, PyAny>> = value
        .extract()
        .map_err(|_| PyTypeError::new_err("authorization_exemptions must be a list"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            fixed_integer::<u32>(value, &format!("authorization_exemptions[{index}]"))
                .map(AuthorizationExemption::from_raw)
        })
        .collect()
}

/// Owned evaluation; the async bridge converts it after native guards are gone.
pub(crate) struct AccessRightsValue(pub(crate) AccessRightsEvaluation);

impl<'py> IntoPyObject<'py> for AccessRightsValue {
    type Target = PyDict;
    type Output = Bound<'py, PyDict>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
        let result = PyDict::new(py);
        let (decision, event, rule) = match self.0.decision {
            AccessRightsDecision::Granted { rule, .. } => ("granted", None, Some(rule)),
            AccessRightsDecision::Denied {
                access_event, rule, ..
            } => ("denied", Some(access_event.to_raw()), rule),
            AccessRightsDecision::Exempt => ("exempt", None, None),
            _ => return Err(PyRuntimeError::new_err("unmapped access rights decision")),
        };
        result.set_item("decision", decision)?;
        result.set_item("access_event", event)?;
        result.set_item("rule", rule.map(|rule| rule_to_py(py, rule)).transpose()?)?;
        let unresolved = PyList::empty(py);
        for entry in self.0.unresolved {
            let item = PyDict::new(py);
            item.set_item("index", entry.index)?;
            item.set_item("reference", object_reference_to_py(py, &entry.reference)?)?;
            item.set_item("reason", unresolved_reason(entry.reason)?)?;
            unresolved.append(item)?;
        }
        result.set_item("unresolved", unresolved)?;
        Ok(result)
    }
}

fn rule_to_py(py: Python<'_>, rule: AccessRulePosition) -> PyResult<Bound<'_, PyDict>> {
    let result = PyDict::new(py);
    result.set_item(
        "access_rights",
        PyObjectIdentifier::from_rust(rule.access_rights),
    )?;
    result.set_item("kind", rule_kind(rule.kind)?)?;
    result.set_item("index", rule.index)?;
    Ok(result)
}

fn rule_kind(kind: AccessRuleKind) -> PyResult<&'static str> {
    match kind {
        AccessRuleKind::Positive => Ok("positive"),
        AccessRuleKind::Negative => Ok("negative"),
        _ => Err(PyRuntimeError::new_err("unmapped access rule kind")),
    }
}

fn unresolved_reason(reason: UnresolvedReason) -> PyResult<&'static str> {
    match reason {
        UnresolvedReason::Remote => Ok("remote"),
        UnresolvedReason::WildcardDevice => Ok("wildcard_device"),
        UnresolvedReason::NotAccessRights => Ok("not_access_rights"),
        UnresolvedReason::Missing => Ok("missing"),
        UnresolvedReason::Unreadable => Ok("unreadable"),
        _ => Err(PyRuntimeError::new_err(
            "unmapped unresolved access rights reason",
        )),
    }
}

#[cfg(test)]
#[path = "access_credential_tests.rs"]
mod tests;
