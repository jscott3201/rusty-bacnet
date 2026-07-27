use super::*;

// ---------------------------------------------------------------------------
// MultiStateInput (type 13)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Input object.
///
/// Read-only multi-state point. Present_Value is writable only when out-of-service.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    state_text: Vec<String>,
    /// Alarm_Values — state values that trigger OFFNORMAL.
    alarm_values: Vec<u32>,
    /// Fault_Values — state values that indicate a fault.
    fault_values: Vec<u32>,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
}

impl MultiStateInputObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, instance)?;
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: 0,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            alarm_values: Vec::new(),
            fault_values: Vec::new(),
            event_detector: ChangeOfStateDetector::default(),
        })
    }

    /// Set the alarm values (states that trigger OFFNORMAL).
    pub fn set_alarm_values(&mut self, values: Vec<u32>) {
        self.alarm_values = values.clone();
        self.event_detector.alarm_values = values;
    }

    /// Set the fault values (states that indicate a fault).
    pub fn set_fault_values(&mut self, values: Vec<u32>) {
        self.fault_values = values;
    }

    /// Set the present value (used by application to update input state).
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for MultiStateInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    crate::impl_intrinsic_reporting!(event_detector, present_value, reliability);

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::MULTI_STATE_INPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
            p if p == PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            p if p == PropertyIdentifier::STATE_TEXT => match array_index {
                None => Ok(PropertyValue::List(
                    self.state_text
                        .iter()
                        .map(|s| PropertyValue::CharacterString(s.clone()))
                        .collect(),
                )),
                Some(0) => Ok(PropertyValue::Unsigned(self.state_text.len() as u64)),
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => Ok(
                    PropertyValue::CharacterString(self.state_text[(idx - 1) as usize].clone()),
                ),
                _ => Err(common::invalid_array_index_error()),
            },
            p if p == PropertyIdentifier::ALARM_VALUES => Ok(PropertyValue::List(
                self.alarm_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::FAULT_VALUES => Ok(PropertyValue::List(
                self.fault_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EVENT_ENABLE => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.event_enable << 5],
            }),
            p if p == PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.event_detector.acked_transitions << 5],
            }),
            p if p == PropertyIdentifier::NOTIFICATION_CLASS => Ok(PropertyValue::Unsigned(
                self.event_detector.notification_class as u64,
            )),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Unsigned(v) = value {
                if v < 1 || v > self.number_of_states as u64 {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v as u32;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::STATE_TEXT {
            match array_index {
                Some(idx) if idx >= 1 && (idx as usize) <= self.state_text.len() => {
                    if let PropertyValue::CharacterString(s) = value {
                        self.state_text[(idx - 1) as usize] = s;
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                }
                None => return Err(common::write_access_denied_error()),
                _ => return Err(common::invalid_array_index_error()),
            }
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
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATE_TEXT,
            PropertyIdentifier::ALARM_VALUES,
            PropertyIdentifier::FAULT_VALUES,
        ];
        Cow::Borrowed(PROPS)
    }

    fn is_createable(&self) -> bool {
        true
    }
    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Mirrors the MultiStateInput `write_property` arms.
        common::is_multistate_input_writable(property)
    }
}
