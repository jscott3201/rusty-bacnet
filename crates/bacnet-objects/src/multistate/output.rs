use super::*;
use crate::event::CommandFailureDetector;

// ---------------------------------------------------------------------------
// MultiStateOutput (type 14)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Output object.
///
/// Commandable multi-state output with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    feedback_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    state_text: Vec<String>,
    alarm_values: Vec<u32>,
    fault_values: Vec<u32>,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    /// Value source tracking (optional per spec — exposed via VALUE_SOURCE property).
    value_source: common::ValueSourceTracking,
}

impl MultiStateOutputObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, instance)?;
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            feedback_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 1,
            reliability: 0,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            alarm_values: Vec::new(),
            fault_values: Vec::new(),
            event_detector: CommandFailureDetector::default(),
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

impl BACnetObject for MultiStateOutputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn supports_cov(&self) -> bool {
        true
    }

    crate::impl_intrinsic_reporting!(event_detector, present_value, feedback_value, reliability);

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
                ObjectType::MULTI_STATE_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::FEEDBACK_VALUE => {
                Ok(PropertyValue::Unsigned(self.feedback_value as u64))
            }
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(
                self.event_detector.event_state.to_raw(),
            )),
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
        if property == PropertyIdentifier::FEEDBACK_VALUE {
            if let PropertyValue::Unsigned(u) = value {
                // Deliberately not range-checked against Number_Of_States, unlike
                // Present_Value. Clause 12.19 treats a Feedback_Value outside the state
                // set as a condition to be *reported* — "If any of those properties other
                // than Present_Value are out of range, the value of the Reliability
                // property shall remain CONFIGURATION_ERROR" — not as a value to refuse.
                // Feedback_Value reflects a sensed quantity whose determination is "a
                // local matter", so it can legitimately fall outside the configured
                // range. Rejecting the write would make CONFIGURATION_ERROR unreachable.
                // Setting that reliability is tracked as #226.
                self.feedback_value = u as u32;
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
            PropertyIdentifier::FEEDBACK_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::NUMBER_OF_STATES,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
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
        // Mirrors the MultiStateOutput `write_property` arms.
        common::is_multistate_commandable_writable(property)
            || property == PropertyIdentifier::FEEDBACK_VALUE
    }
}

#[cfg(test)]
mod command_failure_tests {
    use super::*;
    use bacnet_types::enums::{EventState, EventType};

    fn write_unsigned(
        object: &mut MultiStateOutputObject,
        property: PropertyIdentifier,
        value: u64,
    ) {
        object
            .write_property(property, None, PropertyValue::Unsigned(value), None)
            .unwrap();
    }

    #[test]
    fn feedback_value_round_trips_and_is_advertised_writable() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();

        write_unsigned(&mut mso, PropertyIdentifier::FEEDBACK_VALUE, 2);

        assert_eq!(
            mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert!(mso
            .property_list()
            .contains(&PropertyIdentifier::FEEDBACK_VALUE));
        assert!(mso.is_writable_property(PropertyIdentifier::FEEDBACK_VALUE));
        assert!(mso
            .write_property(
                PropertyIdentifier::FEEDBACK_VALUE,
                None,
                PropertyValue::Enumerated(2),
                None,
            )
            .is_err());
    }

    /// Clause 12.19 defines an out-of-range Feedback_Value as a reportable condition
    /// (Reliability CONFIGURATION_ERROR, see #226), not a value to refuse. Refusing it
    /// would make that reliability unreachable, so the write is accepted even though
    /// Present_Value at the same value would be rejected.
    #[test]
    fn feedback_value_outside_the_state_set_is_accepted_unlike_present_value() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();

        write_unsigned(&mut mso, PropertyIdentifier::FEEDBACK_VALUE, 7);
        assert_eq!(
            mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(7)
        );

        assert!(mso
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(7),
                None,
            )
            .is_err());
    }

    #[test]
    fn command_failure_uses_present_and_feedback_not_alarm_values() {
        let mut disagreeing = MultiStateOutputObject::new(1, "MSO-disagree", 3).unwrap();
        write_unsigned(&mut disagreeing, PropertyIdentifier::PRESENT_VALUE, 2);

        let outcome = disagreeing.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

        let mut agreeing = MultiStateOutputObject::new(2, "MSO-agree", 3).unwrap();
        agreeing.alarm_values = vec![2];
        write_unsigned(&mut agreeing, PropertyIdentifier::FEEDBACK_VALUE, 2);
        write_unsigned(&mut agreeing, PropertyIdentifier::PRESENT_VALUE, 2);

        assert_eq!(agreeing.evaluate_intrinsic_reporting(), None);
        assert_eq!(
            agreeing
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
    }

    #[test]
    fn time_delay_gates_command_failure() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        mso.event_detector.time_delay = 2;
        write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

        assert_eq!(mso.evaluate_intrinsic_reporting(), None);
        assert_eq!(mso.tick_intrinsic_reporting(), None);
        let outcome = mso.tick_intrinsic_reporting().unwrap();
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
    }
}
