//! Analog, binary and multi-state object registration. Each takes the
//! `tags`, `profile_location` and `profile_name` keywords that provision the
//! optional profile rows (#1553), and `storage_path` and
//! `clear_written_reporting`, which keep the reporting settings a peer
//! writes, and each object's event algorithm parameters (#2331), which
//! the parameter keywords configure (#2410); the Value objects also take `present_value_access`,
//! `value_source_tracking` (#1552) and the independently optional Audit
//! policy.
//!
//! The nine builders register through the endpoints' path (#2711): each
//! reads its arguments into a [`Registration`], which
//! [`BACnetServer::register_point`] admits as an endpoint admits it and
//! builds with the same code.
use super::super::*;
use crate::algorithm_keywords::AlgorithmKeywords;
use crate::object_options::{object_profile, present_value_access};
use crate::point_registration::{Registration, RegistrationKind, Storage, value_keywords};
use crate::types::PyBitString;
use bacnet_types::enums::Polarity;

impl BACnetServer {
    /// Keep `point`, built once with `storage`, for start (#2711), once
    /// it has been admitted as an endpoint admits it (#2748: through the
    /// path every builder takes). Every check comes before the object is
    /// built, or inside the build before storage attaches, so a refused
    /// registration has touched no storage file and leaves the others as
    /// they were.
    fn register_point(&self, point: Registration, storage: Storage<'_>) -> PyResult<()> {
        self.admit_pending(point.identity(), || point.check(), || point.build(storage))
    }
}

#[pymethods]
impl BACnetServer {
    /// Add an Analog Input object to the server (before starting).
    ///
    /// `present_value` must be finite: NaN or an infinity raises
    /// BacnetProtocolError VALUE_OUT_OF_RANGE, as on the endpoints.
    ///
    /// The nine analog, binary and multi-state builders check each
    /// registration as the endpoints' builders do (#2711), before anything
    /// is kept: instance 4194303 or past raises BacnetError, an object
    /// identifier or name the Device or an earlier registration already has
    /// raises ValueError, and a refused registration leaves the others for
    /// start. A multi-state builder takes 1 to 1024 states.
    ///
    /// `storage_path` (a `str`) keeps the reporting settings a peer writes
    /// (Notification_Class, Event_Enable, Notify_Type,
    /// Event_Detection_Enable, Event_Message_Texts_Config and the
    /// Event_Algorithm_Inhibit pair) in that file (#2331), and a written
    /// Description (#2364): such a write is saved before it is answered,
    /// refused if it can't be, and served again after a restart. The
    /// analog, binary and multi-state builders keep the event algorithm
    /// parameters a peer writes there too (#2331): the two time delays on
    /// all nine, High_Limit, Low_Limit, Deadband and Limit_Enable on the
    /// analog ones, Alarm_Value on a Binary Input or Value, and
    /// Alarm_Values on a Multi-state Input or Value. The three
    /// multi-state builders keep a written State_Text there too, and with
    /// it the state count, which the restarted object takes over
    /// `number_of_states` (#2414). `add_access_door`,
    /// `add_access_zone` and `add_event_enrollment` keep theirs too, as
    /// each says.
    /// `clear_written_reporting` drops what was saved first, Description
    /// and parameters included.
    /// Every analog, binary and multi-state builder takes both, as do
    /// `add_access_door`, `add_access_zone`, `add_event_enrollment` and
    /// `add_alert_enrollment`.
    ///
    /// The parameter keywords configure those parameters (#2410):
    /// `high_limit`, `low_limit` and `deadband` (floats), `limit_enable` (a
    /// `BitString` of two bits, low limit first, the form a read of
    /// Limit_Enable gives: `BitString(6, b"\x40")` enables the high limit
    /// alone), and `time_delay` and `time_delay_normal` (seconds) on the
    /// analog builders; `alarm_value` (0 or 1, for INACTIVE or ACTIVE) and
    /// the delays on a Binary Input or Value; `alarm_values` (a list of states)
    /// and the delays on a Multi-state Input or Value; the delays alone on a
    /// Binary or Multi-state Output. None leaves the object's default. Each
    /// value is checked as a write of the property is, and one the object
    /// refuses raises BacnetProtocolError: VALUE_OUT_OF_RANGE for a
    /// non-finite limit, a negative deadband, an `alarm_value` past 1 or a
    /// state of 0 or past `number_of_states`, and INVALID_DATA_ENCODING for
    /// a `limit_enable` that isn't two bits wide. A negative delay raises
    /// OverflowError. The keywords apply before storage attaches, so they
    /// are never saved: a value a peer wrote and the file holds is served
    /// over its keyword, and the keyword applies again on a start that
    /// passes `clear_written_reporting=True`. A `write_property_local` of a
    /// parameter after the builder call is kept like a peer's write. An
    /// `alarm_values` keyword is checked against `number_of_states`, so a
    /// restored State_Text with fewer states (#2414) leaves it naming a
    /// state past the count: a Multi-state Value then reports
    /// CONFIGURATION_ERROR.
    #[pyo3(signature = (
        instance, name, units=62, present_value=0.0, *,
        tags=None, profile_location=None, profile_name=None,
        high_limit=None, low_limit=None, deadband=None, limit_enable=None,
        time_delay=None, time_delay_normal=None,
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
        high_limit: Option<f32>,
        low_limit: Option<f32>,
        deadband: Option<f32>,
        limit_enable: Option<PyBitString>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::AnalogInput {
                units,
                present_value,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::analog(
                high_limit,
                low_limit,
                deadband,
                limit_enable,
                time_delay,
                time_delay_normal,
            ),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add an Analog Output object to the server (before starting).
    ///
    /// The parameter keywords are as on `add_analog_input`.
    #[pyo3(signature = (
        instance, name, units=62, *, tags=None, profile_location=None, profile_name=None,
        high_limit=None, low_limit=None, deadband=None, limit_enable=None,
        time_delay=None, time_delay_normal=None,
        storage_path=None, clear_written_reporting=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_analog_output(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        high_limit: Option<f32>,
        low_limit: Option<f32>,
        deadband: Option<f32>,
        limit_enable: Option<PyBitString>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::AnalogOutput {
                units,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::analog(
                high_limit,
                low_limit,
                deadband,
                limit_enable,
                time_delay,
                time_delay_normal,
            ),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Binary Input object to the server (before starting).
    ///
    /// The parameter keywords are as on `add_analog_input`. `polarity` is
    /// a `Polarity` or its number: NORMAL (0, the default) or REVERSE (1),
    /// which inverts the state `set_physical_input_local` reports on its
    /// way to Present_Value; any other int, negative included, raises
    /// BacnetProtocolError VALUE_OUT_OF_RANGE. Clients
    /// can read Polarity but not write it; `set_polarity_local` changes it
    /// later.
    #[pyo3(signature = (
        instance, name, *, polarity=0, tags=None, profile_location=None, profile_name=None,
        alarm_value=None, time_delay=None, time_delay_normal=None,
        storage_path=None, clear_written_reporting=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_binary_input(
        &self,
        instance: u32,
        name: &str,
        #[pyo3(from_py_with = crate::types::int_enums::polarity_arg)] polarity: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        alarm_value: Option<u32>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::BinaryInput {
                polarity: Polarity::from_raw(polarity),
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::binary(alarm_value, time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Binary Output object to the server (before starting).
    ///
    /// The parameter keywords are as on `add_analog_input`. `polarity` is
    /// a `Polarity` or its number: NORMAL (0, the default) or REVERSE (1),
    /// which inverts the state `physical_output_local` gives for the
    /// commanded Present_Value; any other int, negative included, raises
    /// BacnetProtocolError VALUE_OUT_OF_RANGE. Clients
    /// can read Polarity but not write it; `set_polarity_local` changes it
    /// later.
    #[pyo3(signature = (
        instance, name, *, polarity=0, tags=None, profile_location=None, profile_name=None,
        time_delay=None, time_delay_normal=None,
        storage_path=None, clear_written_reporting=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_binary_output(
        &self,
        instance: u32,
        name: &str,
        #[pyo3(from_py_with = crate::types::int_enums::polarity_arg)] polarity: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::BinaryOutput {
                polarity: Polarity::from_raw(polarity),
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::delays(time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Multi-State Input object to the server (before starting).
    ///
    /// The parameter keywords are as on `add_analog_input`.
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None,
        alarm_values=None, time_delay=None, time_delay_normal=None,
        storage_path=None, clear_written_reporting=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        alarm_values: Option<Vec<u32>>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::MultiStateInput {
                number_of_states,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::multistate(alarm_values, time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Multi-State Output object to the server (before starting).
    ///
    /// The parameter keywords are as on `add_analog_input`.
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        tags=None, profile_location=None, profile_name=None,
        time_delay=None, time_delay_normal=None,
        storage_path=None, clear_written_reporting=false
    ))]
    #[allow(clippy::too_many_arguments)]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
        tags: Option<&Bound<'_, PyAny>>,
        profile_location: Option<String>,
        profile_name: Option<String>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::MultiStateOutput {
                number_of_states,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::delays(time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Multi-State Value object to the server (before starting).
    ///
    /// `present_value_access` and `value_source_tracking` are as on
    /// `add_analog_value`, the parameter keywords as on `add_analog_input`.
    #[pyo3(signature = (
        instance, name, number_of_states, *,
        present_value_access="commandable", value_source_tracking=false,
        tags=None, profile_location=None, profile_name=None,
        alarm_values=None, time_delay=None, time_delay_normal=None,
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
        alarm_values: Option<Vec<u32>>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let access = self::present_value_access(present_value_access, value_source_tracking)?;
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::MultiStateValue {
                number_of_states,
                access,
                value_source_tracking,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::multistate(alarm_values, time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add an Analog Value object to the server (before starting).
    ///
    /// `present_value_access` is `"commandable"` (writes go through
    /// Priority_Array), `"writable"` (a write replaces Present_Value) or
    /// `"read_only"` (peers write it only while out of service).
    /// `value_source_tracking` serves Value_Source on a noncommandable object:
    /// the last writer, who alone may then correct it (#1552). A commandable
    /// object tracks its sources through Priority_Array already, so asking it
    /// to track raises ValueError. The parameter keywords are as on
    /// `add_analog_input`.
    #[pyo3(signature = (
        instance, name, units=62, *,
        present_value_access="commandable", value_source_tracking=false,
        audit_level=None, auditable_operations=None, audit_priority_filter=None,
        tags=None, profile_location=None, profile_name=None,
        high_limit=None, low_limit=None, deadband=None, limit_enable=None,
        time_delay=None, time_delay_normal=None,
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
        high_limit: Option<f32>,
        low_limit: Option<f32>,
        deadband: Option<f32>,
        limit_enable: Option<PyBitString>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let value = value_keywords(
            present_value_access,
            value_source_tracking,
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::AnalogValue {
                units,
                value,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::analog(
                high_limit,
                low_limit,
                deadband,
                limit_enable,
                time_delay,
                time_delay_normal,
            ),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }

    /// Add a Binary Value object to the server (before starting).
    ///
    /// `present_value_access` and `value_source_tracking` are as on
    /// `add_analog_value`, the parameter keywords as on `add_analog_input`.
    #[pyo3(signature = (
        instance, name, *,
        present_value_access="commandable", value_source_tracking=false,
        audit_level=None, auditable_operations=None, audit_priority_filter=None,
        tags=None, profile_location=None, profile_name=None,
        alarm_value=None, time_delay=None, time_delay_normal=None,
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
        alarm_value: Option<u32>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
        storage_path: Option<&str>,
        clear_written_reporting: bool,
    ) -> PyResult<()> {
        let value = value_keywords(
            present_value_access,
            value_source_tracking,
            audit_level,
            auditable_operations,
            audit_priority_filter,
        )?;
        let point = Registration::new(
            instance,
            name,
            RegistrationKind::BinaryValue {
                value,
                present_value: None,
            },
            object_profile(tags, profile_location, profile_name)?,
            AlgorithmKeywords::binary(alarm_value, time_delay, time_delay_normal),
        );
        self.register_point(point, storage(storage_path, clear_written_reporting))
    }
}

/// The `storage_path` and `clear_written_reporting` keywords.
fn storage(path: Option<&str>, clear: bool) -> Storage<'_> {
    Storage { path, clear }
}
