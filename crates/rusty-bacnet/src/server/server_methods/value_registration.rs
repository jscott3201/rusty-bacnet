//! Analog, binary and multi-state object registration. Each takes the
//! `tags`, `profile_location` and `profile_name` keywords that provision the
//! optional profile rows (#1553), and `storage_path` and
//! `clear_written_reporting`, which keep the reporting settings a peer
//! writes (#2331); the Value objects also take `present_value_access`,
//! `value_source_tracking` (#1552) and the independently optional Audit
//! policy.
use super::super::*;
use crate::object_options::{keep_reporting, present_value_access, provision_profile};
use bacnet_types::enums::EngineeringUnits;

#[pymethods]
impl BACnetServer {
    /// Add an Analog Input object to the server (before starting).
    ///
    /// `storage_path` (a `str`) keeps the reporting settings a peer writes
    /// (Notification_Class, Event_Enable, Notify_Type,
    /// Event_Detection_Enable, Event_Message_Texts_Config and the
    /// Event_Algorithm_Inhibit pair) in that file (#2331): such a write is
    /// saved before it is answered, refused if it can't be, and served again
    /// after a restart. `clear_written_reporting` drops what was saved first.
    /// Every analog, binary and multi-state builder takes both, as do
    /// `add_access_door`, `add_access_zone`, `add_event_enrollment` and
    /// `add_alert_enrollment`.
    #[pyo3(signature = (
        instance, name, units=62, present_value=0.0, *,
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
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
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut ai = keep_reporting(
            || AnalogInputObject::new(instance, name, EngineeringUnits::from_raw(units)),
            storage_path,
            clear_written_reporting,
            AnalogInputObject::with_reporting_persistence,
            AnalogInputObject::clear_written_reporting,
        )?;
        ai.set_present_value(present_value);
        provision_profile(tags, profile_location, profile_name, |p| ai.set_profile(p))?;
        self.push_pending(Box::new(ai))
    }

    /// Add an Analog Output object to the server (before starting).
    #[pyo3(signature = (
        instance, name, units=62, *, tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
    ))]
    fn add_analog_output(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut ao = keep_reporting(
            || AnalogOutputObject::new(instance, name, EngineeringUnits::from_raw(units)),
            storage_path,
            clear_written_reporting,
            AnalogOutputObject::with_reporting_persistence,
            AnalogOutputObject::clear_written_reporting,
        )?;
        provision_profile(tags, profile_location, profile_name, |p| ao.set_profile(p))?;
        self.push_pending(Box::new(ao))
    }

    /// Add a Binary Input object to the server (before starting).
    #[pyo3(signature = (
        instance, name, *, tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
    ))]
    fn add_binary_input(
        &self,
        instance: u32,
        name: &str,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut bi = keep_reporting(
            || BinaryInputObject::new(instance, name),
            storage_path,
            clear_written_reporting,
            BinaryInputObject::with_reporting_persistence,
            BinaryInputObject::clear_written_reporting,
        )?;
        provision_profile(tags, profile_location, profile_name, |p| bi.set_profile(p))?;
        self.push_pending(Box::new(bi))
    }

    /// Add a Binary Output object to the server (before starting).
    #[pyo3(signature = (
        instance, name, *, tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
    ))]
    fn add_binary_output(
        &self,
        instance: u32,
        name: &str,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut bo = keep_reporting(
            || BinaryOutputObject::new(instance, name),
            storage_path,
            clear_written_reporting,
            BinaryOutputObject::with_reporting_persistence,
            BinaryOutputObject::clear_written_reporting,
        )?;
        provision_profile(tags, profile_location, profile_name, |p| bo.set_profile(p))?;
        self.push_pending(Box::new(bo))
    }

    /// Add a Multi-State Input object to the server (before starting).
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
    ))]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut msi = keep_reporting(
            || MultiStateInputObject::new(instance, name, number_of_states),
            storage_path,
            clear_written_reporting,
            MultiStateInputObject::with_reporting_persistence,
            MultiStateInputObject::clear_written_reporting,
        )?;
        provision_profile(tags, profile_location, profile_name, |p| msi.set_profile(p))?;
        self.push_pending(Box::new(msi))
    }

    /// Add a Multi-State Output object to the server (before starting).
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
    ))]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let mut mso = keep_reporting(
            || MultiStateOutputObject::new(instance, name, number_of_states),
            storage_path,
            clear_written_reporting,
            MultiStateOutputObject::with_reporting_persistence,
            MultiStateOutputObject::clear_written_reporting,
        )?;
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
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
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
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let mut msv = keep_reporting(
            || MultiStateValueObject::with_access(instance, name, number_of_states, access),
            storage_path,
            clear_written_reporting,
            MultiStateValueObject::with_reporting_persistence,
            MultiStateValueObject::clear_written_reporting,
        )?;
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
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
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
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut obj = keep_reporting(
            || {
                AnalogValueObject::with_access(
                    instance,
                    name,
                    EngineeringUnits::from_raw(units),
                    access,
                )
            },
            storage_path,
            clear_written_reporting,
            AnalogValueObject::with_reporting_persistence,
            AnalogValueObject::clear_written_reporting,
        )?;
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
        tags=None, profile_location=None, profile_name=None,
        storage_path=None, clear_written_reporting=false
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
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let audit_policy = crate::object_audit_policy::parse(
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let mut bv = keep_reporting(
            || BinaryValueObject::with_access(instance, name, access),
            storage_path,
            clear_written_reporting,
            BinaryValueObject::with_reporting_persistence,
            BinaryValueObject::clear_written_reporting,
        )?;
        bv.set_audit_policy(audit_policy);
        bv.set_value_source_tracking(value_source_tracking);
        provision_profile(tags, profile_location, profile_name, |p| bv.set_profile(p))?;
        self.push_pending(Box::new(bv))
    }
}
