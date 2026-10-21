use super::super::*;
use bacnet_types::enums::EngineeringUnits;

#[pymethods]
impl BACnetServer {
    /// Add a Calendar object to the server (before starting).
    ///
    /// `date_list` seeds Date_List with calendar entry mappings, the form a
    /// typed read of it gives (#1499): `kind` `date` with `date`,
    /// `date_range` with `start_date` and `end_date`, or `week_n_day` with
    /// `month`, `week_of_month` and `day_of_week`. An entry the Calendar
    /// refuses raises BacnetProtocolError and nothing is registered.
    #[pyo3(signature = (instance, name, *, date_list=None))]
    fn add_calendar(
        &self,
        instance: u32,
        name: &str,
        date_list: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let entries = date_list
            .map(crate::types::calendar_entries_from_py)
            .transpose()?
            .unwrap_or_default();
        let mut cal = CalendarObject::new(instance, name).map_err(to_py_err)?;
        for entry in entries {
            cal.add_date_entry(entry).map_err(to_py_err)?;
        }
        self.push_pending(Box::new(cal))
    }

    /// Add a Schedule object to the server (before starting).
    ///
    /// The keywords take the forms typed reads of the properties give
    /// (#1499): `weekly_schedule` seven lists of `(time, value)` pairs,
    /// Monday first; `exception_schedule` a list of mappings of `period` (a
    /// calendar entry mapping or a Calendar's ObjectIdentifier),
    /// `time_values` and `priority`; `effective_period` a `(start_date,
    /// end_date)` pair. What the Schedule refuses, such as a time given
    /// twice in a day or a priority outside 1 to 16, raises
    /// BacnetProtocolError and nothing is registered.
    #[pyo3(signature = (
        instance, name, *, weekly_schedule=None, exception_schedule=None, effective_period=None
    ))]
    fn add_schedule(
        &self,
        instance: u32,
        name: &str,
        weekly_schedule: Option<&Bound<'_, PyAny>>,
        exception_schedule: Option<&Bound<'_, PyAny>>,
        effective_period: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let week = weekly_schedule
            .map(crate::types::weekly_schedule_from_py)
            .transpose()?;
        let exceptions = exception_schedule
            .map(crate::types::special_events_from_py)
            .transpose()?
            .unwrap_or_default();
        let period = effective_period
            .map(|period| crate::types::date_range_from_py(period, "effective_period"))
            .transpose()?;
        let sched = schedule(instance, name, week, exceptions, period).map_err(to_py_err)?;
        self.push_pending(Box::new(sched))
    }

    /// Add a Notification Class object to the server (before starting).
    ///
    /// With `storage_path`, a Recipient_List a client writes is kept in that
    /// file and restored when the server is built again. A write whose list
    /// cannot be saved is refused with DEVICE / OPERATIONAL_PROBLEM, and the
    /// old list stays. Without it the list lives in memory only. Give each
    /// class its own file: one that holds another object's list, or that
    /// this backend did not write, raises BacnetError here.
    ///
    /// `recipients` seeds Recipient_List with `Destination` mappings, as
    /// `add_notification_forwarder` does (#1364): more than 32, or an
    /// address MAC past 18 octets, raises BacnetProtocolError and nothing is
    /// registered. With `storage_path`, a saved, written list wins, and
    /// until a write sets the list the seed applies at every start.
    #[pyo3(signature = (
        instance,
        name,
        notification_class=0,
        storage_path=None,
        *,
        recipients=None
    ))]
    fn add_notification_class(
        &self,
        instance: u32,
        name: &str,
        notification_class: u32,
        storage_path: Option<&str>,
        recipients: Option<Vec<Bound<'_, PyAny>>>,
    ) -> PyResult<()> {
        let recipients = crate::types::destinations(recipients)?;
        let mut nc = match storage_path {
            Some(path) => {
                let storage =
                    Arc::new(FileNotificationClassPersistence::new(path).map_err(to_py_err)?);
                NotificationClass::with_persistence(instance, name, storage).map_err(to_py_err)?
            }
            None => NotificationClass::new(instance, name).map_err(to_py_err)?,
        };
        nc.notification_class = notification_class;
        for destination in recipients {
            nc.add_destination(destination).map_err(to_py_err)?;
        }
        self.push_pending(Box::new(nc))
    }

    /// Add an Audit Log object to the server (before starting).
    #[pyo3(signature = (instance, name, storage_path, buffer_size=100))]
    fn add_audit_log(
        &self,
        instance: u32,
        name: &str,
        storage_path: &str,
        buffer_size: u32,
    ) -> PyResult<()> {
        let storage = Arc::new(FileAuditLogPersistence::new(storage_path).map_err(to_py_err)?);
        let al = AuditLogObject::new(instance, name, buffer_size, storage).map_err(to_py_err)?;
        self.push_pending(Box::new(al))
    }

    /// Add an Audit Reporter object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_audit_reporter(&self, instance: u32, name: &str) -> PyResult<()> {
        let ar = AuditReporterObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(ar))
    }

    // -----------------------------------------------------------------------
    // Pattern A: new(instance, name) — simple two-param constructors
    // -----------------------------------------------------------------------

    /// Add a Command object to the server (before starting).
    ///
    /// `action` is the Action array: one list of `ActionCommand` mappings per
    /// element, so writing N to Present_Value runs list N. `action_text`
    /// serves Action_Text and needs one text per list. Shapes, Python types
    /// and each command's device identifier, which must be a Device, are
    /// checked here (TypeError / ValueError); the object's own setters refuse
    /// what else BACnet doesn't allow, such as a priority outside 1 to 16 or
    /// a text count that differs from the list count, as a protocol error
    /// (VALUE_OUT_OF_RANGE).
    #[pyo3(signature = (instance, name, *, action=None, action_text=None))]
    fn add_command(
        &self,
        instance: u32,
        name: &str,
        action: Option<Bound<'_, PyAny>>,
        action_text: Option<Vec<String>>,
    ) -> PyResult<()> {
        let action = action
            .map(|action| crate::types::action_lists_from_py(&action))
            .transpose()?;
        let obj = command(instance, name, action, action_text).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Timer object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_timer(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimerObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Load Control object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_load_control(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LoadControlObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Program object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_program(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ProgramObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Life Safety Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_life_safety_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LifeSafetyZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Group object to the server (before starting); its Present_Value
    /// is rebuilt from `members` on each read, one result per member, in
    /// order.
    ///
    /// `members` takes the `read_property_multiple` spec shape, checked as
    /// the endpoint owners' `add_group` checks it: a member listing no
    /// properties, or a group's Present_Value, is a ValueError naming its
    /// position and the rule; any property identifier is taken.
    #[pyo3(signature = (instance, name, members=None))]
    fn add_group(
        &self,
        instance: u32,
        name: &str,
        members: Option<Vec<crate::types::PyReadAccessSpec>>,
    ) -> PyResult<()> {
        let members = crate::group_members::members(members);
        let obj = crate::group_members::group(instance, name, &members)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Global Group object to the server (before starting).
    ///
    /// `members` seeds Group_Members with property references, each a
    /// `(object, property)` or `(object, property, array_index)` tuple or a
    /// `DeviceObjectPropertyReference` mapping, the form a typed read of it
    /// gives (#1499). A member naming this server's Device is stored in its
    /// local form; a device identifier that isn't a Device raises ValueError.
    #[pyo3(signature = (instance, name, *, members=None))]
    fn add_global_group(
        &self,
        instance: u32,
        name: &str,
        members: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let members = members
            .map(|members| self.members_from_py(members, "members"))
            .transpose()?
            .unwrap_or_default();
        let mut obj = GlobalGroupObject::new(instance, name).map_err(to_py_err)?;
        obj.set_group_members(members).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Structured View object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_structured_view(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = StructuredViewObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Alert Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name, initial_source))]
    fn add_alert_enrollment(
        &self,
        instance: u32,
        name: &str,
        initial_source: PyObjectIdentifier,
    ) -> PyResult<()> {
        let obj = AlertEnrollmentObject::new(instance, name, initial_source.to_rust())
            .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Credential object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_credential(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessCredentialObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Elevator Group object to the server (before starting).
    ///
    /// `machine_room_id` names the Positive Integer Value object served as
    /// Machine_Room_ID. Any other object type raises a protocol error
    /// (VALUE_OUT_OF_RANGE). Omit it to keep the "no room number" default.
    #[pyo3(signature = (instance, name, machine_room_id=None))]
    fn add_elevator_group(
        &self,
        instance: u32,
        name: &str,
        machine_room_id: Option<PyObjectIdentifier>,
    ) -> PyResult<()> {
        let obj = elevator_group(instance, name, machine_room_id.map(|oid| oid.to_rust()))
            .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Escalator object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_escalator(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = EscalatorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Value types — all take new(instance, name)
    // -----------------------------------------------------------------------

    /// Add an Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = IntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Positive Integer Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_positive_integer_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = PositiveIntegerValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Large Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_large_analog_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LargeAnalogValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Character String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_character_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CharacterStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Octet String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_octet_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = OctetStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Bit String Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_bit_string_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BitStringValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimeValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Date Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DatePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Time Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = TimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a DateTime Pattern Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_date_time_pattern_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = DateTimePatternValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    // -----------------------------------------------------------------------
    // Pattern B: new(instance, name, extra_param) — three-param constructors
    // -----------------------------------------------------------------------

    /// Add an Accumulator object to the server (before starting). `scale`
    /// sets Scale: a float for a float scale, an int for a power-of-ten
    /// scale. `prescale`, a `(multiplier, modulo_divide)` pair, serves the
    /// optional Prescale (#1487); a `modulo_divide` of 0 raises ValueError.
    #[pyo3(signature = (instance, name, units=62, *, scale=None, prescale=None))]
    fn add_accumulator(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        scale: Option<&Bound<'_, PyAny>>,
        prescale: Option<(u32, u32)>,
    ) -> PyResult<()> {
        let mut obj = AccumulatorObject::new(instance, name, EngineeringUnits::from_raw(units))
            .map_err(to_py_err)?;
        if let Some(scale) = scale {
            obj.set_scale(crate::types::scale_from_py(scale)?);
        }
        if let Some((multiplier, modulo_divide)) = prescale {
            // It fits the type, but a divisor of 0 converts no pulse.
            if modulo_divide == 0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "prescale's modulo_divide must be at least 1",
                ));
            }
            obj.set_prescale(bacnet_types::constructed::BACnetPrescale {
                multiplier,
                modulo_divide,
            });
        }
        self.push_pending(Box::new(obj))
    }

    /// Add a Pulse Converter object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_pulse_converter(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = PulseConverterObject::new(instance, name, EngineeringUnits::from_raw(units))
            .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a File object to the server (before starting).
    #[pyo3(signature = (instance, name, file_type="application/octet-stream"))]
    fn add_file(&self, instance: u32, name: &str, file_type: &str) -> PyResult<()> {
        let obj = FileObject::new(instance, name, file_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Enrollment object to the server (before starting).
    ///
    /// `event_type` defaults to `EventType.CHANGE_OF_BITSTRING`. Event_Type
    /// follows Event_Parameters; this type applies only while the legacy
    /// octet-string parameters, which name no algorithm, are held (#1868).
    #[pyo3(signature = (
        instance,
        name,
        event_type=PyEventType { inner: EventType::CHANGE_OF_BITSTRING }
    ))]
    fn add_event_enrollment(
        &self,
        instance: u32,
        name: &str,
        event_type: PyEventType,
    ) -> PyResult<()> {
        let obj =
            EventEnrollmentObject::new(instance, name, event_type.to_rust()).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an explicitly configured local-target Staging object before starting.
    #[pyo3(signature = (
        instance,
        name,
        present_value,
        min_present_value,
        units,
        priority_for_writing,
        stages,
        target_references,
        stage_names=None
    ))]
    fn add_staging(
        &self,
        instance: u32,
        name: &str,
        present_value: f32,
        min_present_value: f32,
        units: u32,
        priority_for_writing: u8,
        stages: Vec<(f32, Vec<bool>, f32)>,
        target_references: Vec<PyObjectIdentifier>,
        stage_names: Option<Vec<String>>,
    ) -> PyResult<()> {
        let config = staging_config(
            present_value,
            min_present_value,
            units,
            priority_for_writing,
            stages,
            target_references,
            stage_names,
        );
        let obj = StagingObject::new(instance, name, config).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lift object to the server (before starting).
    #[pyo3(signature = (instance, name, num_floors))]
    fn add_lift(&self, instance: u32, name: &str, num_floors: usize) -> PyResult<()> {
        let obj = LiftObject::new(instance, name, num_floors).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Log object to the server (before starting). With
    /// `log_received_notifications` it also records the event notifications
    /// the server receives, a few per source each second.
    /// `total_record_count` seeds Total_Record_Count, as on `add_trend_log`.
    /// With `storage_path` (a `str`) the log is kept in that file pair, as
    /// on `add_trend_log`: a log reopened from it keeps its saved records
    /// and settings, ignores `total_record_count`, and appends a
    /// LOG_INTERRUPTED record (#1541); `buffer_size` wins over the saved
    /// Buffer_Size.
    /// `log_received_notifications` isn't saved and applies on every start.
    #[pyo3(signature = (
        instance,
        name,
        buffer_size=100,
        log_received_notifications=false,
        *,
        storage_path=None,
        total_record_count=0
    ))]
    fn add_event_log(
        &self,
        instance: u32,
        name: &str,
        buffer_size: u32,
        log_received_notifications: bool,
        storage_path: Option<&str>,
        total_record_count: u32,
    ) -> PyResult<()> {
        let mut obj = match storage_path {
            Some(path) => {
                let storage = bacnet_objects::log_persistence::FileLogPersistence::new(path)
                    .map_err(to_py_err)?;
                EventLogObject::with_persistence(instance, name, buffer_size, Arc::new(storage))
            }
            None => EventLogObject::new(instance, name, buffer_size),
        }
        .map_err(to_py_err)?;
        obj.set_log_received_notifications(log_received_notifications);
        if !obj.reopened() {
            obj.restore_log_buffer(total_record_count, [])
                .map_err(to_py_err)?;
        }
        self.push_pending(Box::new(obj))
    }
}

fn staging_config(
    present_value: f32,
    min_present_value: f32,
    units: u32,
    priority_for_writing: u8,
    stages: Vec<(f32, Vec<bool>, f32)>,
    target_references: Vec<PyObjectIdentifier>,
    stage_names: Option<Vec<String>>,
) -> StagingConfig {
    StagingConfig {
        present_value,
        min_present_value,
        units: EngineeringUnits::from_raw(units),
        priority_for_writing,
        stages: stages
            .into_iter()
            .map(|(limit, values, deadband)| BACnetStageLimitValue {
                limit,
                values,
                deadband,
            })
            .collect(),
        target_references: target_references
            .into_iter()
            .map(|reference| BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: reference.to_rust(),
            })
            .collect(),
        stage_names,
    }
}

/// Build an Elevator Group, applying the optional Machine_Room_ID through the
/// object's own validating setter.
fn elevator_group(
    instance: u32,
    name: &str,
    machine_room_id: Option<bacnet_types::primitives::ObjectIdentifier>,
) -> Result<ElevatorGroupObject, bacnet_types::error::Error> {
    let mut obj = ElevatorGroupObject::new(instance, name)?;
    if let Some(oid) = machine_room_id {
        obj.set_machine_room_id(oid)?;
    }
    Ok(obj)
}

/// Build a Schedule, applying the optional Weekly_Schedule, Exception_Schedule
/// and Effective_Period through the object's own validating setters.
fn schedule(
    instance: u32,
    name: &str,
    week: Option<Vec<Vec<bacnet_types::constructed::BACnetTimeValue>>>,
    exceptions: Vec<bacnet_types::constructed::BACnetSpecialEvent>,
    period: Option<bacnet_types::constructed::BACnetDateRange>,
) -> Result<ScheduleObject, bacnet_types::error::Error> {
    let mut sched = ScheduleObject::new(instance, name, PropertyValue::Null)?;
    for (day, entries) in week.into_iter().flatten().enumerate() {
        sched.set_weekly_schedule(day, entries)?;
    }
    for event in exceptions {
        sched.add_exception(event)?;
    }
    if let Some(period) = period {
        sched.set_effective_period(period)?;
    }
    Ok(sched)
}

/// Build a Command, applying the optional Action lists and Action_Text through
/// the object's own validating setters, Action first.
fn command(
    instance: u32,
    name: &str,
    action: Option<Vec<bacnet_types::constructed::BACnetActionList>>,
    action_text: Option<Vec<String>>,
) -> Result<CommandObject, bacnet_types::error::Error> {
    let mut obj = CommandObject::new(instance, name)?;
    if let Some(action) = action {
        obj.set_action(action)?;
    }
    if let Some(texts) = action_text {
        obj.set_action_text(texts)?;
    }
    Ok(obj)
}

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;
