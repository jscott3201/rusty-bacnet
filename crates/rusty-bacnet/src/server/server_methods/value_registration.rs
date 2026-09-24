//! AV/BV registration with independently optional Audit policy.
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Add an Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62, *, audit_level=None, auditable_operations=None, audit_priority_filter=None))]
    fn add_analog_value(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut obj = AnalogValueObject::new(instance, name, units).map_err(to_py_err)?;
        obj.set_audit_policy(audit_policy);
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Value object to the server (before starting).
    #[pyo3(signature = (instance, name, *, audit_level=None, auditable_operations=None, audit_priority_filter=None))]
    fn add_binary_value(
        &self,
        instance: u32,
        name: &str,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut bv = BinaryValueObject::new(instance, name).map_err(to_py_err)?;
        bv.set_audit_policy(audit_policy);
        self.push_pending(Box::new(bv))
    }
}
