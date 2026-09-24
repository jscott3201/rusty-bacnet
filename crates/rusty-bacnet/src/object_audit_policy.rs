//! Shared pre-registration AV/BV policy parsing for server and endpoint builders.
use bacnet_objects::audit::{AuditPriorityPolicy, ObjectAuditPolicy};
use bacnet_types::{
    bitstring::{AuditOperationFlags, BACnetPriorityFilter},
    enums::AuditLevel,
};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyBool, PyInt},
};

pub(crate) fn parse(
    level: Option<&str>,
    operations: Option<&Bound<'_, PyAny>>,
    priorities: Option<&Bound<'_, PyAny>>,
) -> PyResult<ObjectAuditPolicy> {
    let level = level
        .map(|v| match v {
            "default" => Ok(AuditLevel::DEFAULT),
            "none" => Ok(AuditLevel::NONE),
            "audit_config" => Ok(AuditLevel::AUDIT_CONFIG),
            "audit_all" => Ok(AuditLevel::AUDIT_ALL),
            _ => Err(PyValueError::new_err(
                "audit_level must be default, none, audit_config, audit_all, or None",
            )),
        })
        .transpose()?;
    let operations = operations
        .map(|v| {
            let bits = integer(v, "auditable_operations")?;
            AuditOperationFlags::from_bits(bits).map_err(|e| PyValueError::new_err(e.to_string()))
        })
        .transpose()?;
    let priority_filter = priorities
        .map(|v| -> PyResult<AuditPriorityPolicy> {
            if v.extract::<&str>().is_ok_and(|v| v == "inherit") {
                return Ok(AuditPriorityPolicy::Inherit);
            }
            let bits = integer(v, "audit_priority_filter")?;
            let bits = u16::try_from(bits)
                .map_err(|_| PyValueError::new_err("audit_priority_filter must be in 0..=65535"))?;
            Ok(AuditPriorityPolicy::Filter(
                BACnetPriorityFilter::from_bits(bits),
            ))
        })
        .transpose()?;
    Ok(ObjectAuditPolicy {
        level,
        operations,
        priority_filter,
    })
}
fn integer(value: &Bound<'_, PyAny>, name: &str) -> PyResult<u64> {
    if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err(format!(
            "{name} must be an integer (not bool)"
        )));
    }
    value
        .extract::<u64>()
        .map_err(|_| PyValueError::new_err(format!("{name} must be in 0..=18446744073709551615")))
}
