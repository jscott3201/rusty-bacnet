//! The object builders the three endpoint owners share (#1867). They take
//! the server builders' keywords, parsed by the same `object_options` code:
//! `tags`, `profile_location` and `profile_name` on all four, plus
//! `present_value_access`, `value_source_tracking` and the Audit policy on
//! the Value objects, and the event algorithm parameter keywords (#2410). Each call builds its object once, so a bad keyword
//! fails there, and keeps the parsed parameters for the rebuild at start.
//! The same generated block carries `report_device_reset_local` (#1837).

use pyo3::prelude::*;

use crate::endpoint::common::ValueKeywords;
use crate::object_options::present_value_access;

/// The Value keywords, checked the way the server's Value builders check
/// them.
pub(super) fn value_keywords(
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

/// Generate the shared methods on one endpoint owner. Invoked in the
/// owner's own module, so the expansion reaches its private `lifecycle` and
/// `push_pending`.
macro_rules! endpoint_builders {
    ($endpoint:ty) => {
        #[pymethods]
        impl $endpoint {
            /// Add an Analog Input object (before start).
            ///
            /// `tags`, `profile_location` and `profile_name` provision the
            /// optional Tags, Profile_Location and Profile_Name rows, as on
            /// `BACnetServer.add_analog_input`; each is served only when
            /// given. The parameter keywords set those parameters as there,
            /// with the same checks.
            #[pyo3(signature = (
                        instance, name, units=62, present_value=0.0, *,
                        tags=None, profile_location=None, profile_name=None,
                        high_limit=None, low_limit=None, deadband=None, limit_enable=None,
                        time_delay=None, time_delay_normal=None
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
                high_limit: Option<f32>,
                low_limit: Option<f32>,
                deadband: Option<f32>,
                limit_enable: Option<$crate::types::PyBitString>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let pending = $crate::endpoint::common::PendingObject::AnalogInput {
                    instance,
                    name: name.to_string(),
                    units,
                    present_value,
                    profile: $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    parameters: $crate::algorithm_keywords::AlgorithmKeywords::analog(
                        high_limit, low_limit, deadband, limit_enable, time_delay, time_delay_normal,
                    ),
                };
                pending.build()?;
                self.push_pending(pending)
            }

            /// Add an Analog Value object (before start).
            ///
            /// `present_value_access`, `value_source_tracking`, the Audit
            /// keywords, the profile keywords and the parameter keywords are
            /// as on `BACnetServer.add_analog_value`.
            #[pyo3(signature = (
                        instance, name, units=62, *,
                        present_value_access="commandable", value_source_tracking=false,
                        audit_level=None, auditable_operations=None, audit_priority_filter=None,
                        tags=None, profile_location=None, profile_name=None,
                        high_limit=None, low_limit=None, deadband=None, limit_enable=None,
                        time_delay=None, time_delay_normal=None
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
                high_limit: Option<f32>,
                low_limit: Option<f32>,
                deadband: Option<f32>,
                limit_enable: Option<$crate::types::PyBitString>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let pending = $crate::endpoint::common::PendingObject::AnalogValue {
                    instance,
                    name: name.to_string(),
                    units,
                    value: $crate::endpoint::builders::value_keywords(
                        present_value_access,
                        value_source_tracking,
                        audit_level,
                        auditable_operations,
                        audit_priority_filter,
                    )?,
                    profile: $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    parameters: $crate::algorithm_keywords::AlgorithmKeywords::analog(
                        high_limit, low_limit, deadband, limit_enable, time_delay, time_delay_normal,
                    ),
                };
                pending.build()?;
                self.push_pending(pending)
            }

            /// Add a Binary Input object (before start). Profile keywords as
            /// on `add_analog_input`, `alarm_value` and the delays as on
            /// `BACnetServer.add_binary_input`.
            #[pyo3(signature = (
                        instance, name, *, tags=None, profile_location=None, profile_name=None,
                        alarm_value=None, time_delay=None, time_delay_normal=None
                    ))]
            #[allow(clippy::too_many_arguments)]
            fn add_binary_input(
                &self,
                instance: u32,
                name: &str,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                alarm_value: Option<u32>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let pending = $crate::endpoint::common::PendingObject::BinaryInput {
                    instance,
                    name: name.to_string(),
                    profile: $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    parameters: $crate::algorithm_keywords::AlgorithmKeywords::binary(
                        alarm_value, time_delay, time_delay_normal,
                    ),
                };
                pending.build()?;
                self.push_pending(pending)
            }

            /// Add a Binary Value object (before start). Access, Value_Source,
            /// Audit and profile keywords as on `add_analog_value`,
            /// `alarm_value` and the delays as on `BACnetServer.add_binary_value`.
            #[pyo3(signature = (
                        instance, name, *,
                        present_value_access="commandable", value_source_tracking=false,
                        audit_level=None, auditable_operations=None, audit_priority_filter=None,
                        tags=None, profile_location=None, profile_name=None,
                        alarm_value=None, time_delay=None, time_delay_normal=None
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
                alarm_value: Option<u32>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let pending = $crate::endpoint::common::PendingObject::BinaryValue {
                    instance,
                    name: name.to_string(),
                    value: $crate::endpoint::builders::value_keywords(
                        present_value_access,
                        value_source_tracking,
                        audit_level,
                        auditable_operations,
                        audit_priority_filter,
                    )?,
                    profile: $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    parameters: $crate::algorithm_keywords::AlgorithmKeywords::binary(
                        alarm_value, time_delay, time_delay_normal,
                    ),
                };
                pending.build()?;
                self.push_pending(pending)
            }

            /// Add an Audit Reporter object (before start), for
            /// `source_audit_reporter` to select (#2348, #2357).
            ///
            /// `audit_level` ("none", "audit_config" or "audit_all") and
            /// `auditable_operations` (bit n for AuditOperation n) are required, as
            /// on `BACnetServer.configure_audit_reporters`, and filter what the
            /// session reports. Peers can't change them through an endpoint. A
            /// Reporter that `source_audit_reporter` doesn't select records nothing.
            #[pyo3(signature = (
                instance, name, *,
                audit_level, auditable_operations, issue_confirmed_notifications=false
            ))]
            fn add_audit_reporter(
                &self,
                instance: u32,
                name: &str,
                audit_level: &str,
                auditable_operations: &Bound<'_, PyAny>,
                issue_confirmed_notifications: bool,
            ) -> PyResult<()> {
                use $crate::server::{reporter_audit_level, reporter_auditable_operations};
                let pending = $crate::endpoint::common::PendingObject::AuditReporter {
                    instance,
                    name: name.to_string(),
                    level: reporter_audit_level(audit_level)?,
                    operations: reporter_auditable_operations(auditable_operations)?,
                    confirmed: issue_confirmed_notifications,
                };
                pending.build()?;
                self.push_pending(pending)
            }

            /// Record a reset the application makes for its own reasons: a
            /// DEVICE_RESET Audit record from this device, with no invoke ID,
            /// through the session's source Reporter when it has one, as
            /// `BACnetServer.report_device_reset_local` does (#1837). Each
            /// endpoint selects one with `source_audit_reporter` (#2348,
            /// #2357), and its level and operations filter the record;
            /// without one this records nothing. A peer's accepted restart
            /// is recorded already. RuntimeError when the endpoint is not
            /// running.
            fn report_device_reset_local<'py>(
                &self,
                py: Python<'py>,
            ) -> PyResult<Bound<'py, PyAny>> {
                let inner = self.lifecycle.session.clone();
                let future = async move {
                    let guard = inner.lock().await;
                    let session = guard.as_ref().ok_or_else(|| {
                        pyo3::exceptions::PyRuntimeError::new_err(
                            "endpoint not started — use 'async with' or await start()",
                        )
                    })?;
                    session
                        .report_device_reset_local()
                        .await
                        .map_err(crate::errors::to_py_err)
                };
                crate::py_async::future_into_py(py, crate::unit_result(future))
            }
        }
    };
}

pub(super) use endpoint_builders;
