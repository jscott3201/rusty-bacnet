use super::*;

// ---------------------------------------------------------------------------
// AnalogInput (type 0)
// ---------------------------------------------------------------------------

/// BACnet Analog Input object.
pub struct AnalogInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    /// Optional minimum present value for fault detection.
    min_pres_value: Option<f32>,
    /// Optional maximum present value for fault detection.
    max_pres_value: Option<f32>,
    /// Event_Time_Stamps[3]: to-offnormal, to-fault, to-normal.
    event_time_stamps: [BACnetTimeStamp; 3],
    /// Event_Message_Texts[3]: to-offnormal, to-fault, to-normal.
    event_message_texts: [String; 3],
}

impl AnalogInputObject {
    /// Create a new Analog Input object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let _oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance)?;
        Ok(Self {
            oid: _oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            cov_increment: 0.0,
            event_detector: OutOfRangeDetector::default(),
            reliability: 0,
            min_pres_value: None,
            max_pres_value: None,
            event_time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            event_message_texts: [String::new(), String::new(), String::new()],
        })
    }

    /// Set the present value (used by the application to update sensor readings).
    pub fn set_present_value(&mut self, value: f32) {
        debug_assert!(
            value.is_finite(),
            "set_present_value called with non-finite value"
        );
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the minimum present value for fault detection.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set the maximum present value for fault detection.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }
}

impl BACnetObject for AnalogInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // IN_ALARM: override STATUS_FLAGS with event_state before common macro
        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                self.event_detector.event_state.to_raw(),
            ));
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = read_event_properties!(self, property) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_INPUT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::COV_INCREMENT => {
                Ok(PropertyValue::Real(self.cov_increment))
            }
            p if p == PropertyIdentifier::MIN_PRES_VALUE => match self.min_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::MAX_PRES_VALUE => match self.max_pres_value {
                Some(v) => Ok(PropertyValue::Real(v)),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        // AI present-value is writable only when out-of-service
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Real(v) = value {
                if !v.is_finite() {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
        }
        if let Some(result) = write_event_properties!(self, property, value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::UNITS,
            PropertyIdentifier::COV_INCREMENT,
            PropertyIdentifier::HIGH_LIMIT,
            PropertyIdentifier::LOW_LIMIT,
            PropertyIdentifier::DEADBAND,
            PropertyIdentifier::LIMIT_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    crate::impl_intrinsic_reporting!(event_detector, present_value, reliability);

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }

    fn is_createable(&self) -> bool {
        true
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Mirrors the AnalogInput `write_property` arms.
        common::is_common_writable(property)
            || property == PropertyIdentifier::PRESENT_VALUE
            || property == PropertyIdentifier::RELIABILITY
            || property == PropertyIdentifier::COV_INCREMENT
            || common::is_event_property_writable(property)
    }
}
