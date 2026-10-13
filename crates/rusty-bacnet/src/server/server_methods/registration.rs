use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Add an Analog Input object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62, present_value=0.0))]
    fn add_analog_input(
        &self,
        instance: u32,
        name: &str,
        units: u32,
        present_value: f32,
    ) -> PyResult<()> {
        let mut ai = AnalogInputObject::new(instance, name, units).map_err(to_py_err)?;
        ai.set_present_value(present_value);
        self.push_pending(Box::new(ai))
    }

    /// Add an Analog Output object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_output(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let ao = AnalogOutputObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(ao))
    }

    /// Add a Binary Input object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_input(&self, instance: u32, name: &str) -> PyResult<()> {
        let bi = BinaryInputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bi))
    }

    /// Add a Binary Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let bo = BinaryOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bo))
    }

    /// Add a Multi-State Input object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_input(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let msi =
            MultiStateInputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(msi))
    }

    /// Add a Multi-State Output object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_output(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let mso =
            MultiStateOutputObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(mso))
    }

    /// Add a Multi-State Value object to the server (before starting).
    #[pyo3(signature = (instance, name, number_of_states))]
    fn add_multistate_value(
        &self,
        instance: u32,
        name: &str,
        number_of_states: u32,
    ) -> PyResult<()> {
        let msv =
            MultiStateValueObject::new(instance, name, number_of_states).map_err(to_py_err)?;
        self.push_pending(Box::new(msv))
    }

    /// Add a Calendar object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_calendar(&self, instance: u32, name: &str) -> PyResult<()> {
        let cal = CalendarObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(cal))
    }

    /// Add a Schedule object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_schedule(&self, instance: u32, name: &str) -> PyResult<()> {
        let sched = ScheduleObject::new(instance, name, PropertyValue::Null).map_err(to_py_err)?;
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

    /// Add a Trend Log object to the server (before starting).
    /// `total_record_count` seeds Total_Record_Count: the first record is
    /// numbered one past it, going from 2^32 - 1 on to 1 (#1537).
    #[pyo3(signature = (instance, name, buffer_size=100, *, total_record_count=0))]
    fn add_trend_log(
        &self,
        instance: u32,
        name: &str,
        buffer_size: u32,
        total_record_count: u32,
    ) -> PyResult<()> {
        let mut tl = TrendLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        tl.restore_log_buffer(total_record_count, [])
            .map_err(to_py_err)?;
        self.push_pending(Box::new(tl))
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

    /// Add a Lighting Output object to the server (before starting).
    ///
    /// Its Lighting_Command reads and writes as `application_data` holding
    /// the context-tagged BACnetLightingCommand, operation NONE until written
    /// (#1263). The object carries each command out (#1384): fades and ramps
    /// move Tracking_Value to the new level over time, steps change the level
    /// at once, and the warn commands blink and hold the level for
    /// Egress_Time when Blink_Warn_Enable is TRUE.
    ///
    /// A Present_Value or Relinquish_Default level above 0.0 and below 1.0 is
    /// stored as 1.0, and one outside 0.0 to 100.0 is refused with
    /// VALUE_OUT_OF_RANGE (#1385), but for Present_Value's warn values -1.0,
    /// -2.0 and -3.0. Tracking_Value reads the same level as Present_Value
    /// whenever no fade or ramp is running.
    #[pyo3(signature = (instance, name))]
    fn add_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = LightingOutputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Binary Lighting Output object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_lighting_output(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = BinaryLightingOutputObject::new(instance, name).map_err(to_py_err)?;
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
    #[pyo3(signature = (instance, name))]
    fn add_global_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GlobalGroupObject::new(instance, name).map_err(to_py_err)?;
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
        let mut obj = AccumulatorObject::new(instance, name, units).map_err(to_py_err)?;
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
        let obj = PulseConverterObject::new(instance, name, units).map_err(to_py_err)?;
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
    /// `event_type` defaults to `EventType.CHANGE_OF_BITSTRING`.
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
    #[pyo3(signature = (
        instance,
        name,
        buffer_size=100,
        log_received_notifications=false,
        *,
        total_record_count=0
    ))]
    fn add_event_log(
        &self,
        instance: u32,
        name: &str,
        buffer_size: u32,
        log_received_notifications: bool,
        total_record_count: u32,
    ) -> PyResult<()> {
        let mut obj = EventLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        obj.set_log_received_notifications(log_received_notifications);
        obj.restore_log_buffer(total_record_count, [])
            .map_err(to_py_err)?;
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
        units,
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
