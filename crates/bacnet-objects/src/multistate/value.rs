use super::*;

// ---------------------------------------------------------------------------
// MultiStateValue (type 19)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Value object.
///
/// Commandable multi-state value with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    reliability_before_out_of_service: Option<u32>,
    state_text: Vec<String>,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.20). Clause 13.2.2.1: "If the
    /// Event_Detection_Enable property is FALSE, then this state machine is not evaluated."
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
    /// Value source tracking (optional per spec — exposed via VALUE_SOURCE property).
    value_source: common::ValueSourceTracking,
}

impl MultiStateValueObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, instance)?;
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 1,
            reliability: 0,
            reliability_before_out_of_service: None,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            event_detector: ChangeOfStateDetector::default(),
            event_detection_enable: true,
            event_history: EventHistory::default(),
            value_source: common::ValueSourceTracking::default(),
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for MultiStateValueObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    crate::impl_intrinsic_reporting!(
        event_detector,
        present_value,
        reliability,
        event_detection_enable
    );

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
                ObjectType::MULTI_STATE_VALUE.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::NUMBER_OF_STATES => {
                Ok(PropertyValue::Unsigned(self.number_of_states as u64))
            }
            p if p == PropertyIdentifier::VALUE_SOURCE => {
                Ok(self.value_source.value_source.clone())
            }
            p if p == PropertyIdentifier::LAST_COMMAND_TIME => Ok(PropertyValue::Unsigned(
                match self.value_source.last_command_time {
                    BACnetTimeStamp::SequenceNumber(n) => n,
                    _ => 0,
                },
            )),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: u32| PropertyValue::Unsigned(
                    v as u64
                ))
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Unsigned(self.relinquish_default as u64))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
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
        priority: Option<u8>,
    ) -> Result<(), Error> {
        {
            let num_states = self.number_of_states;
            common::write_priority_array_direct!(self, property, array_index, value, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            let num_states = self.number_of_states;
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Unsigned(u) = v {
                    if u < 1 || u > num_states as u64 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(u as u32)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
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
        // Clause 12.20, while Out_Of_Service is TRUE: "the Present_Value property and
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
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
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
        // Mirrors the MultiStateValue `write_property` arms. The generic event
        // set became writable with #229: Clause 12.20 requires the supported
        // Event_Enable value set to include (T, T, T), and these detectors
        // default to (F, F, F) with, previously, no commissioning path at all.
        common::is_multistate_commandable_writable(property)
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
    fn msv_detection_enable_resets_and_gates_intrinsic_reporting() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        msv.event_detector.alarm_values = vec![2];
        msv.event_detector.time_delay = 2;
        msv.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            Some(8),
        )
        .unwrap();
        assert_eq!(msv.evaluate_intrinsic_reporting(), None);
        assert!(msv.event_detector.pending.is_some());

        msv.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
        msv.event_detector.acked_transitions = 0;
        msv.event_detector.fault_reliability = Some(1);
        msv.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw())
        );
        assert_eq!(
            msv.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }
        );
        assert!(msv.event_detector.pending.is_none());
        assert!(msv.event_detector.fault_reliability.is_none());
        assert_eq!(msv.evaluate_intrinsic_reporting(), None);
        assert_eq!(msv.tick_intrinsic_reporting(), None);
        assert!(
            msv.event_detector.pending.is_none(),
            "evaluate/tick re-armed a countdown while detection is disabled"
        );
        assert!(msv
            .property_list()
            .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
        assert!(msv.is_writable_property(PropertyIdentifier::EVENT_DETECTION_ENABLE));
    }
}
