use super::super::*;

#[pymethods]
impl BACnetServer {
    #[new]
    #[pyo3(signature = (
        device_instance,
        device_name="BACnet Device",
        interface="0.0.0.0",
        port=0xBAC0,
        broadcast_address="255.255.255.255",
        transport="bip",
        sc_hub=None,
        sc_vmac=None,
        sc_ca_cert=None,
        sc_client_cert=None,
        sc_client_key=None,
        sc_heartbeat_interval_ms=None,
        sc_heartbeat_timeout_ms=None,
        ipv6_interface=None,
        dcc_password=None,
        reinit_password=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        device_instance: u32,
        device_name: &str,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        transport: &str,
        sc_hub: Option<String>,
        sc_vmac: Option<Vec<u8>>,
        sc_ca_cert: Option<String>,
        sc_client_cert: Option<String>,
        sc_client_key: Option<String>,
        sc_heartbeat_interval_ms: Option<u64>,
        sc_heartbeat_timeout_ms: Option<u64>,
        ipv6_interface: Option<String>,
        dcc_password: Option<String>,
        reinit_password: Option<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            device_instance,
            device_name: device_name.to_string(),
            transport_type: transport.to_string(),
            interface: interface.to_string(),
            port,
            broadcast_address: broadcast_address.to_string(),
            sc_hub,
            sc_vmac,
            sc_ca_cert,
            sc_client_cert,
            sc_client_key,
            sc_heartbeat_interval_ms,
            sc_heartbeat_timeout_ms,
            ipv6_interface,
            dcc_password,
            reinit_password,
            started: Arc::new(AtomicBool::new(false)),
            pending_objects: std::sync::Mutex::new(Vec::new()),
        }
    }

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

    /// Add a Binary Value object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_binary_value(&self, instance: u32, name: &str) -> PyResult<()> {
        let bv = BinaryValueObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(bv))
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
    #[pyo3(signature = (instance, name, notification_class=0))]
    fn add_notification_class(
        &self,
        instance: u32,
        name: &str,
        notification_class: u32,
    ) -> PyResult<()> {
        let mut nc = NotificationClass::new(instance, name).map_err(to_py_err)?;
        nc.notification_class = notification_class;
        self.push_pending(Box::new(nc))
    }

    /// Add a Trend Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let tl = TrendLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(tl))
    }

    /// Add a Loop (PID) object to the server (before starting).
    #[pyo3(signature = (instance, name, output_units=62))]
    fn add_loop(&self, instance: u32, name: &str, output_units: u32) -> PyResult<()> {
        let lp = LoopObject::new(instance, name, output_units).map_err(to_py_err)?;
        self.push_pending(Box::new(lp))
    }

    /// Add an Audit Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_audit_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let al = AuditLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
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

    /// Add an Analog Value object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_analog_value(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = AnalogValueObject::new(instance, name, units).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Command object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_command(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CommandObject::new(instance, name).map_err(to_py_err)?;
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

    /// Add a Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = GroupObject::new(instance, name).map_err(to_py_err)?;
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

    /// Add a Notification Forwarder object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_notification_forwarder(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = NotificationForwarderObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Alert Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_alert_enrollment(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AlertEnrollmentObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Door object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_door(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessDoorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Credential object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_credential(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessCredentialObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Point object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_point(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessPointObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Rights object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_rights(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessRightsObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access User object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_user(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessUserObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Access Zone object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_access_zone(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AccessZoneObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Credential Data Input object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_credential_data_input(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = CredentialDataInputObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Elevator Group object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_elevator_group(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = ElevatorGroupObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Escalator object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_escalator(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = EscalatorObject::new(instance, name).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Averaging object to the server (before starting).
    #[pyo3(signature = (instance, name))]
    fn add_averaging(&self, instance: u32, name: &str) -> PyResult<()> {
        let obj = AveragingObject::new(instance, name).map_err(to_py_err)?;
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

    /// Add an Accumulator object to the server (before starting).
    #[pyo3(signature = (instance, name, units=62))]
    fn add_accumulator(&self, instance: u32, name: &str, units: u32) -> PyResult<()> {
        let obj = AccumulatorObject::new(instance, name, units).map_err(to_py_err)?;
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

    /// Add a Network Port object to the server (before starting).
    #[pyo3(signature = (instance, name, network_type=0))]
    fn add_network_port(&self, instance: u32, name: &str, network_type: u32) -> PyResult<()> {
        let obj = NetworkPortObject::new(instance, name, network_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Enrollment object to the server (before starting).
    #[pyo3(signature = (instance, name, event_type=0))]
    fn add_event_enrollment(&self, instance: u32, name: &str, event_type: u32) -> PyResult<()> {
        let obj = EventEnrollmentObject::new(instance, name, event_type).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Channel object to the server (before starting).
    #[pyo3(signature = (instance, name, channel_number))]
    fn add_channel(&self, instance: u32, name: &str, channel_number: u32) -> PyResult<()> {
        let obj = ChannelObject::new(instance, name, channel_number).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Staging object to the server (before starting).
    #[pyo3(signature = (instance, name, num_stages))]
    fn add_staging(&self, instance: u32, name: &str, num_stages: usize) -> PyResult<()> {
        let obj = StagingObject::new(instance, name, num_stages).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Lift object to the server (before starting).
    #[pyo3(signature = (instance, name, num_floors))]
    fn add_lift(&self, instance: u32, name: &str, num_floors: usize) -> PyResult<()> {
        let obj = LiftObject::new(instance, name, num_floors).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add an Event Log object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_event_log(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = EventLogObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Trend Log Multiple object to the server (before starting).
    #[pyo3(signature = (instance, name, buffer_size=100))]
    fn add_trend_log_multiple(&self, instance: u32, name: &str, buffer_size: u32) -> PyResult<()> {
        let obj = TrendLogMultipleObject::new(instance, name, buffer_size).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}
