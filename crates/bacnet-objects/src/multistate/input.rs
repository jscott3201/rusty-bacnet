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
    reliability_before_out_of_service: Option<u32>,
    state_text: Vec<String>,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.18). Clause 13.2.2.1: "If the
    /// Event_Detection_Enable property is FALSE, then this state machine is not evaluated."
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
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
            reliability_before_out_of_service: None,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            event_detector: ChangeOfStateDetector::default(),
            event_detection_enable: true,
            event_history: EventHistory::default(),
        })
    }

    /// Set the alarm values (states that trigger OFFNORMAL).
    pub fn set_alarm_values(&mut self, values: Vec<u32>) {
        self.event_detector.alarm_values = values;
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

    crate::event::impl_builtin_intrinsic_reporting!(
        event_detector,
        event_history,
        present_value,
        reliability,
        event_detection_enable
    );
    impl_intrinsic_write_rollback!(event_detector, event_detection_enable, event_history);

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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(self.event_detection_enable));
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
            return result;
        }
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::MULTI_STATE_INPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
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
                self.event_detector
                    .alarm_values
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v as u64))
                    .collect(),
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
        if property == PropertyIdentifier::ALARM_VALUES {
            let values = decode_alarm_values_write(array_index, value)?;
            self.event_detector.alarm_values = values;
            return Ok(());
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                if !v {
                    self.event_detector.event_state = bacnet_types::enums::EventState::NORMAL;
                    self.event_detector.acked_transitions = 0b111;
                    self.event_detector.pending = None;
                    self.event_detector.fault_reliability = None;
                    self.event_history.reset();
                }
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if let Some(result) = write_generic_event_properties!(self, property, value) {
            return result;
        }
        if let Some(result) = common::write_out_of_service_with_reliability_restore(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        // Clause 12.18, while Out_Of_Service is TRUE: "the Present_Value property and
        // the Reliability property, if present and capable of taking on values other
        // than NO_FAULT_DETECTED, shall be writable to allow simulating specific
        // conditions or for testing purposes".
        // `is_writable_property` stays statically true because it describes capability.
        if property == PropertyIdentifier::RELIABILITY {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Enumerated(v) = value {
                if !common::is_reliability_value_valid(v) {
                    return Err(common::value_out_of_range_error());
                }
                self.reliability = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
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
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::STATE_TEXT,
            PropertyIdentifier::ALARM_VALUES,
        ];
        Cow::Borrowed(PROPS)
    }

    fn is_createable(&self) -> bool {
        true
    }
    fn set_reliability_internal(&mut self, reliability: u32) -> Result<(), Error> {
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        Ok(())
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Mirrors the MultiStateInput `write_property` arms. The generic event
        // set became writable with #229: Clause 12.18 requires the supported
        // Event_Enable value set to include (T, T, T), and these detectors
        // default to (F, F, F) with, previously, no commissioning path at all.
        common::is_multistate_input_writable(property)
            || common::is_generic_event_property_writable(property)
            || property == PropertyIdentifier::RELIABILITY
            || property == PropertyIdentifier::EVENT_DETECTION_ENABLE
            || property == PropertyIdentifier::ALARM_VALUES
    }
}

#[cfg(test)]
mod detection_enable_tests {
    use super::*;

    #[test]
    fn msi_detection_enable_resets_and_gates_intrinsic_reporting() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        msi.set_alarm_values(vec![2]);
        msi.event_detector.time_delay = 2;
        msi.set_present_value(2);
        assert_eq!(msi.evaluate_intrinsic_reporting(), None);
        assert!(msi.event_detector.pending.is_some());

        msi.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
        msi.event_detector.acked_transitions = 0;
        msi.event_detector.fault_reliability = Some(1);
        msi.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw())
        );
        assert_eq!(
            msi.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }
        );
        assert!(msi.event_detector.pending.is_none());
        assert!(msi.event_detector.fault_reliability.is_none());
        assert_eq!(msi.evaluate_intrinsic_reporting(), None);
        assert_eq!(msi.tick_intrinsic_reporting(), None);
        assert!(
            msi.event_detector.pending.is_none(),
            "evaluate/tick re-armed a countdown while detection is disabled"
        );
        assert!(msi
            .property_list()
            .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
        assert!(msi.is_writable_property(PropertyIdentifier::EVENT_DETECTION_ENABLE));
    }
}
