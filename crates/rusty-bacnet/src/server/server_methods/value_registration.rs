//! Analog, binary and multi-state object registration. Each takes the
//! `tags`, `profile_location` and `profile_name` keywords that provision the
//! optional profile rows (#1553); the Value objects also take
//! `present_value_access`, `value_source_tracking` (#1552) and the
//! independently optional Audit policy.
use super::super::*;
use crate::object_options::{present_value_access, provision_profile};
use bacnet_types::enums::EngineeringUnits;

#[pymethods]
impl BACnetServer {
    /// Add an Analog Input object to the server (before starting).
    #[pyo3(signature = (
        instance, name, units=62, present_value=0.0, *,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_analog_input(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut ai = AnalogInputObject::new(instance, name, EngineeringUnits::from_raw(units))
            .map_err(to_py_err)?;
        ai.set_present_value(present_value);
        provision_profile(tags, profile_location, profile_name, |p| ai.set_profile(p))?;
        self.push_pending(Box::new(ai))
    }

    /// Add an Analog Output object to the server (before starting).
    #[pyo3(signature = (
        instance, name, units=62, *, tags=None, profile_location=None, profile_name=None
    ))]
    fn add_analog_output(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut ao = AnalogOutputObject::new(instance, name, EngineeringUnits::from_raw(units))
            .map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| ao.set_profile(p))?;
        self.push_pending(Box::new(ao))
    }

    /// Add a Binary Input object to the server (before starting).
    #[pyo3(signature = (instance, name, *, tags=None, profile_location=None, profile_name=None))]
    fn add_binary_input(
        &self,
        instance: u32,
        name: &str,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut bi = BinaryInputObject::new(instance, name).map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| bi.set_profile(p))?;
        self.push_pending(Box::new(bi))
    }

    /// Add a Binary Output object to the server (before starting).
    #[pyo3(signature = (instance, name, *, tags=None, profile_location=None, profile_name=None))]
    fn add_binary_output(
        &self,
        instance: u32,
        name: &str,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut bo = BinaryOutputObject::new(instance, name).map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| bo.set_profile(p))?;
        self.push_pending(Box::new(bo))
    }

    /// Add a Multi-State Input object to the server (before starting).
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None
    ))]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut msi =
            MultiStateInputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| msi.set_profile(p))?;
        self.push_pending(Box::new(msi))
    }

    /// Add a Multi-State Output object to the server (before starting).
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None
    ))]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let mut mso =
            MultiStateOutputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        provision_profile(tags, profile_location, profile_name, |p| mso.set_profile(p))?;
        self.push_pending(Box::new(mso))
    }

    /// Add a Multi-State Value object to the server (before starting).
    ///
    /// `present_value_access` and `value_source_tracking` are as on
    /// `add_analog_value`.
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        present_value_access="commandable", value_source_tracking=false,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_multistate_value(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        present_value_access: &str,
        value_source_tracking: bool,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let mut msv = MultiStateValueObject::with_access(instance, name, number_of_states, access)
            .map_err(to_py_err)?;
        msv.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| msv.set_profile(p))?;
        self.push_pending(Box::new(msv))
    }

    /// Add an Analog Value object to the server (before starting).
    ///
    /// `present_value_access` is `"commandable"` (writes go through
    /// Priority_Array), `"writable"` (a write replaces Present_Value) or
    /// `"read_only"` (peers write it only while out of service).
    /// `value_source_tracking` serves Value_Source on a noncommandable object:
    /// the last writer, who alone may then correct it (#1552). A commandable
    /// object tracks its sources through Priority_Array already, so asking it
    /// to track raises ValueError.
    #[pyo3(signature = (
        instance, name, units=62, *,
        present_value_access="commandable", value_source_tracking=false,
        audit_level=None, auditable_operations=None, audit_priority_filter=None,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_analog_value(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value_access: &str,
        value_source_tracking: bool,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut obj = AnalogValueObject::with_access(
            instance,
            name,
            EngineeringUnits::from_raw(units),
            access,
        )
        .map_err(to_py_err)?;
        obj.set_audit_policy(audit_policy);
        obj.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| obj.set_profile(p))?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Value object to the server (before starting).
    ///
    /// `present_value_access` and `value_source_tracking` are as on
    /// `add_analog_value`.
    #[pyo3(signature = (
        instance, name, *,
        present_value_access="commandable", value_source_tracking=false,
        audit_level=None, auditable_operations=None, audit_priority_filter=None,
        tags=None, profile_location=None, profile_name=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_binary_value(
        &self,
        instance: u32,
        name: &str,
        present_value_access: &str,
        value_source_tracking: bool,
        audit_level: Option<&str>,
        auditable_operations: Option<&Bound<'_, PyAny>>,
        audit_priority_filter: Option<&Bound<'_, PyAny>>,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut bv = BinaryValueObject::with_access(instance, name, access).map_err(to_py_err)?;
        bv.set_audit_policy(audit_policy);
        bv.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| bv.set_profile(p))?;
        self.push_pending(Box::new(bv))
    }
}
