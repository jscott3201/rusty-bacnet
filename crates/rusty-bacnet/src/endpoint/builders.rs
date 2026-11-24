//! The object builders the three endpoint owners share (#1867). They take
//! the server builders' keywords, parsed by the same `object_options` code:
//! `tags`, `profile_location` and `profile_name` on the point objects, plus
//! `present_value_access`, `value_source_tracking` and the Audit policy on
//! the Value objects, and the event algorithm parameter keywords (#2410).
//! Each call builds its object once, so a bad keyword fails there, and
//! keeps the parsed parameters for the rebuild at start. The application's
//! own changes to its objects, and its own resets' Audit records, go
//! through `await endpoint.server()` (#2617).
//!
//! The nine point builders read their arguments into the
//! [`Registration`](crate::point_registration::Registration)
//! `BACnetServer`'s builders read theirs into, and pass the same checks in
//! the same order (#2711).

/// Generate the shared methods on one endpoint owner. Invoked in the
/// owner's own module, so the expansion reaches its private `lifecycle` and
/// `push_pending`.
macro_rules! endpoint_builders {
    ($endpoint:ty) => {
        #[pymethods]
        impl $endpoint {
            /// Add an Analog Input object (before start).
            ///
            /// `present_value` (a finite number, default 0.0) sets
            /// Present_Value directly; NaN or an infinity raises
            /// BacnetProtocolError VALUE_OUT_OF_RANGE. Every point builder
            /// checks its registration as `BACnetServer.add_analog_input`
            /// says, with the same code (#2711).
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
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::AnalogInput { units, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::analog(
                        high_limit, low_limit, deadband, limit_enable, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add an Analog Value object (before start).
            ///
            /// `present_value` is the initial value. With
            /// `present_value_access="commandable"` (the default) it sets
            /// Relinquish_Default and the priority array starts empty, so
            /// Present_Value reads as that value until something is
            /// commanded; with "writable" or "read_only" it sets
            /// Present_Value directly. `None` keeps the object type's
            /// default.
            ///
            /// `present_value_access`, `value_source_tracking`, the Audit
            /// keywords, the profile keywords and the parameter keywords are
            /// as on `BACnetServer.add_analog_value`.
            #[pyo3(signature = (
                        instance, name, units=62, *, present_value=None,
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
                present_value: Option<f32>,
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
                let value = $crate::point_registration::value_keywords(
                    present_value_access,
                    value_source_tracking,
                    audit_level,
                    auditable_operations,
                    audit_priority_filter,
                )?;
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::AnalogValue { units, value, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::analog(
                        high_limit, low_limit, deadband, limit_enable, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add a Binary Input object (before start).
            ///
            /// `present_value` is the logical value (already after Polarity):
            /// it sets Present_Value, and the object derives its physical
            /// input state from it. Left as `None`, the object keeps its
            /// unset physical-input state.
            ///
            /// Profile keywords as on `add_analog_input`, `polarity`,
            /// `alarm_value` and the delays as on `BACnetServer.add_binary_input`.
            #[pyo3(signature = (
                        instance, name, *, present_value=None, polarity=0, tags=None, profile_location=None, profile_name=None,
                        alarm_value=None, time_delay=None, time_delay_normal=None
                    ))]
            #[allow(clippy::too_many_arguments)]
            fn add_binary_input(
                &self,
                instance: u32,
                name: &str,
                present_value: Option<u32>,
                #[pyo3(from_py_with = $crate::types::int_enums::polarity_arg)] polarity: u32,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                alarm_value: Option<u32>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let polarity = bacnet_types::enums::Polarity::from_raw(polarity);
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::BinaryInput { polarity, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::binary(
                        alarm_value, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add a Binary Value object (before start).
            ///
            /// `present_value` is the initial value. With
            /// `present_value_access="commandable"` (the default) it sets
            /// Relinquish_Default and the priority array starts empty, so
            /// Present_Value reads as that value until something is
            /// commanded; with "writable" or "read_only" it sets
            /// Present_Value directly. `None` keeps the object type's
            /// default.
            ///
            /// Access, Value_Source, Audit and profile keywords as on
            /// `add_analog_value`, `alarm_value` and the delays as on
            /// `BACnetServer.add_binary_value`.
            #[pyo3(signature = (
                        instance, name, *, present_value=None,
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
                present_value: Option<u32>,
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
                let value = $crate::point_registration::value_keywords(
                    present_value_access,
                    value_source_tracking,
                    audit_level,
                    auditable_operations,
                    audit_priority_filter,
                )?;
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::BinaryValue { value, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::binary(
                        alarm_value, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add an Analog Output object before startup.
            ///
            /// `present_value` sets Relinquish_Default. The priority array
            /// starts empty, so Present_Value reads as that value until
            /// something is commanded. `None` keeps the object type's
            /// default.
            #[pyo3(signature = (
                instance, name, units=62, *, present_value=None, tags=None, profile_location=None, profile_name=None,
                high_limit=None, low_limit=None, deadband=None, limit_enable=None,
                time_delay=None, time_delay_normal=None
            ))]
            #[allow(clippy::too_many_arguments)]
            fn add_analog_output(
                &self,
                instance: u32,
                name: &str,
                units: u32,
                present_value: Option<f32>,
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
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::AnalogOutput { units, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::analog(
                        high_limit, low_limit, deadband, limit_enable, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add a Binary Output object before startup.
            ///
            /// `present_value` sets Relinquish_Default. The priority array
            /// starts empty, so Present_Value reads as that value until
            /// something is commanded. `None` keeps the object type's
            /// default.
            #[pyo3(signature = (
                instance, name, *, present_value=None, polarity=0, tags=None, profile_location=None, profile_name=None,
                time_delay=None, time_delay_normal=None
            ))]
            #[allow(clippy::too_many_arguments)]
            fn add_binary_output(
                &self,
                instance: u32,
                name: &str,
                present_value: Option<u32>,
                #[pyo3(from_py_with = $crate::types::int_enums::polarity_arg)] polarity: u32,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let polarity = bacnet_types::enums::Polarity::from_raw(polarity);
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::BinaryOutput { polarity, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::delays(time_delay, time_delay_normal),
                )))
            }

            /// Add a Multi-state Input object before startup.
            ///
            /// `present_value` sets Present_Value directly. `None` keeps the
            /// object type's default. The three multi-state builders take 1
            /// to 1024 states; another count raises BacnetError.
            #[pyo3(signature = (
                instance, name, number_of_states, *, present_value=None,
                tags=None, profile_location=None, profile_name=None,
                alarm_values=None, time_delay=None, time_delay_normal=None
            ))]
            #[allow(clippy::too_many_arguments)]
            fn add_multistate_input(
                &self,
                instance: u32,
                name: &str,
                number_of_states: u32,
                present_value: Option<u32>,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                alarm_values: Option<Vec<u32>>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::MultiStateInput { number_of_states, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::multistate(
                        alarm_values, time_delay, time_delay_normal,
                    ),
                )))
            }

            /// Add a Multi-state Output object before startup.
            ///
            /// `present_value` sets Relinquish_Default. The priority array
            /// starts empty, so Present_Value reads as that value until
            /// something is commanded. `None` keeps the object type's
            /// default.
            #[pyo3(signature = (
                instance, name, number_of_states, *, present_value=None,
                tags=None, profile_location=None, profile_name=None,
                time_delay=None, time_delay_normal=None
            ))]
            #[allow(clippy::too_many_arguments)]
            fn add_multistate_output(
                &self,
                instance: u32,
                name: &str,
                number_of_states: u32,
                present_value: Option<u32>,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::MultiStateOutput { number_of_states, present_value },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::delays(time_delay, time_delay_normal),
                )))
            }

            /// Add a Multi-state Value before startup.
            ///
            /// `present_value` is the initial value. With
            /// `present_value_access="commandable"` (the default) it sets
            /// Relinquish_Default and the priority array starts empty, so
            /// Present_Value reads as that value until something is
            /// commanded; with "writable" or "read_only" it sets
            /// Present_Value directly. `None` keeps the object type's
            /// default.
            #[pyo3(signature = (
                instance, name, number_of_states, *, present_value=None,
                present_value_access="commandable", value_source_tracking=false,
                tags=None, profile_location=None, profile_name=None,
                alarm_values=None, time_delay=None, time_delay_normal=None
            ))]
            #[allow(clippy::too_many_arguments)]
            fn add_multistate_value(
                &self,
                instance: u32,
                name: &str,
                number_of_states: u32,
                present_value: Option<u32>,
                present_value_access: &str,
                value_source_tracking: bool,
                tags: Option<&Bound<'_, PyAny>>,
                profile_location: Option<String>,
                profile_name: Option<String>,
                alarm_values: Option<Vec<u32>>,
                time_delay: Option<u32>,
                time_delay_normal: Option<u32>,
            ) -> PyResult<()> {
                let access = $crate::object_options::present_value_access(
                    present_value_access,
                    value_source_tracking,
                )?;
                self.push_pending($crate::endpoint::common::PendingObject::Point($crate::point_registration::Registration::new(
                    instance,
                    name,
                    $crate::point_registration::RegistrationKind::MultiStateValue {
                        number_of_states,
                        access,
                        value_source_tracking,
                        present_value,
                    },
                    $crate::object_options::object_profile(tags, profile_location, profile_name)?,
                    $crate::algorithm_keywords::AlgorithmKeywords::multistate(
                        alarm_values, time_delay, time_delay_normal,
                    ),
                )))
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
                self.push_pending(pending)
            }
        }
    };
}

pub(super) use endpoint_builders;
