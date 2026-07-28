use super::*;
use crate::common::{read_generic_event_properties, write_generic_event_properties};
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
    reliability_before_out_of_service: Option<u32>,
    event_detection_enable: bool,
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
            reliability_before_out_of_service: None,
            event_detection_enable: false,
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

    crate::impl_intrinsic_reporting!(
        event_detector,
        present_value,
        feedback_value,
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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(self.event_detection_enable));
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
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
                // Checked for representability but deliberately NOT range-checked against
                // Number_Of_States, unlike Present_Value. Clause 12.19 treats a
                // Feedback_Value outside the state set as a condition to be *reported* —
                // "If any of those properties other than Present_Value are out of range,
                // the value of the Reliability property shall remain CONFIGURATION_ERROR"
                // — not as a value to refuse. Feedback_Value reflects a sensed quantity
                // whose determination is "a local matter", so it can legitimately fall
                // outside the configured range; refusing it would make CONFIGURATION_ERROR
                // unreachable. Setting that reliability is tracked as #226.
                //
                // The u32 conversion is still checked. A BACnet Unsigned decodes from up
                // to 8 octets, so a bare `as u32` would wrap a large value back into the
                // valid state range — silently turning a disagreeing feedback into an
                // agreeing one and suppressing the COMMAND_FAILURE transition. That is a
                // representation limit, not a configuration limit, so it is enforced here
                // while the state-set range is not.
                self.feedback_value = common::u64_to_u32(u)?;
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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                if !v {
                    self.event_detector.event_state = bacnet_types::enums::EventState::NORMAL;
                    self.event_detector.acked_transitions = 0b111;
                    self.event_detector.pending = None;
                    self.event_detector.fault_reliability = None;
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
        // Clause 12.19, while Out_Of_Service is TRUE: "the Present_Value property and
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
            PropertyIdentifier::FEEDBACK_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::ACKED_TRANSITIONS,
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
        // Mirrors the MultiStateOutput `write_property` arms.
        common::is_multistate_commandable_writable(property)
            || property == PropertyIdentifier::RELIABILITY
            || common::is_generic_event_property_writable(property)
            || property == PropertyIdentifier::FEEDBACK_VALUE
            || property == PropertyIdentifier::EVENT_DETECTION_ENABLE
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

    fn set_detection_enabled(object: &mut MultiStateOutputObject, enabled: bool) {
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(enabled),
                None,
            )
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

    /// `feedback_value` initializes to match the initial `Present_Value` so that enabling
    /// detection on an untouched object does not immediately report a command failure. This
    /// states that property directly rather than relying on it incidentally: several other
    /// tests in this module also fail if the initializer changes, but each does so as a side
    /// effect of asserting something else, which is a fragile thing to depend on.
    #[test]
    fn fresh_object_reports_nothing_when_detection_is_enabled() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        set_detection_enabled(&mut mso, true);

        assert_eq!(mso.evaluate_intrinsic_reporting(), None);
        assert_eq!(
            mso.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
    }

    /// A BACnet Unsigned decodes from up to 8 octets, so an unchecked `as u32` would wrap
    /// a large Feedback_Value back into the valid state range and suppress the very
    /// transition this object type exists to report. 0x1_0000_0002 truncates to 2, which
    /// would read as agreeing with a Present_Value of 2.
    #[test]
    fn oversized_feedback_value_is_rejected_rather_than_wrapped_into_agreement() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        set_detection_enabled(&mut mso, true);
        write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

        assert!(mso
            .write_property(
                PropertyIdentifier::FEEDBACK_VALUE,
                None,
                PropertyValue::Unsigned(0x1_0000_0002),
                None,
            )
            .is_err());

        // The rejected write left the feedback value alone, so Present_Value 2 against the
        // initial feedback of 1 still disagrees and still reports COMMAND_FAILURE.
        assert_eq!(
            mso.read_property(PropertyIdentifier::FEEDBACK_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        let outcome = mso.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
    }

    #[test]
    fn command_failure_uses_present_and_feedback_not_alarm_values() {
        let mut disagreeing = MultiStateOutputObject::new(1, "MSO-disagree", 3).unwrap();
        set_detection_enabled(&mut disagreeing, true);
        write_unsigned(&mut disagreeing, PropertyIdentifier::PRESENT_VALUE, 2);

        let outcome = disagreeing.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);

        let mut agreeing = MultiStateOutputObject::new(2, "MSO-agree", 3).unwrap();
        set_detection_enabled(&mut agreeing, true);
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
        set_detection_enabled(&mut mso, true);
        write_unsigned(&mut mso, PropertyIdentifier::TIME_DELAY, 2);
        write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

        assert_eq!(mso.evaluate_intrinsic_reporting(), None);
        assert_eq!(mso.tick_intrinsic_reporting(), None);
        let outcome = mso.tick_intrinsic_reporting().unwrap();
        assert_eq!(outcome.change.to, EventState::OFFNORMAL);
        assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
    }

    #[test]
    fn event_enable_to_offnormal_bit_controls_distribution() {
        for (encoded, expected) in [(0x20, true), (0x00, false)] {
            let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
            set_detection_enabled(&mut mso, true);
            mso.write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![encoded],
                },
                None,
            )
            .unwrap();
            write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);
            assert_eq!(
                mso.evaluate_intrinsic_reporting().unwrap().distribute,
                expected
            );
        }
    }

    #[test]
    fn generic_event_properties_round_trip_and_match_pics() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        let writes = [
            (
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyValue::Enumerated(1),
            ),
            (
                PropertyIdentifier::NOTIFICATION_CLASS,
                PropertyValue::Unsigned(42),
            ),
        ];
        for (property, value) in writes {
            mso.write_property(property, None, value.clone(), None)
                .unwrap();
            assert_eq!(mso.read_property(property, None).unwrap(), value);
        }

        // Acked_Transitions is readable but NOT writable: only the AcknowledgeAlarm service
        // may change it. A property write would assign where the service ORs, so it could
        // both fabricate and erase acknowledgments, and it would break the Clause 12.19
        // requirement that the field sit at its initial condition while
        // Event_Detection_Enable is FALSE.
        assert!(mso
            .write_property(
                PropertyIdentifier::ACKED_TRANSITIONS,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![0x80],
                },
                None,
            )
            .is_err());
        assert!(!mso.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));

        for property in [
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ] {
            assert!(mso.property_list().contains(&property));
            assert!(mso.is_writable_property(property));
        }
        assert!(mso
            .write_property(
                PropertyIdentifier::EVENT_STATE,
                None,
                PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
                None,
            )
            .is_err());
        assert!(!mso.is_writable_property(PropertyIdentifier::EVENT_STATE));
    }

    #[test]
    fn detection_enable_is_a_disabled_by_default_invariant() {
        let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        write_unsigned(&mut mso, PropertyIdentifier::PRESENT_VALUE, 2);

        assert_eq!(mso.evaluate_intrinsic_reporting(), None);
        assert_eq!(mso.tick_intrinsic_reporting(), None);
        assert_eq!(
            mso.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            }
        );

        set_detection_enabled(&mut mso, true);
        assert_eq!(
            mso.evaluate_intrinsic_reporting().unwrap().change.to,
            EventState::OFFNORMAL
        );
        mso.event_detector.acked_transitions = 0;
        set_detection_enabled(&mut mso, false);

        assert_eq!(
            mso.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }
        );
        assert_eq!(mso.evaluate_intrinsic_reporting(), None);
        assert_eq!(mso.tick_intrinsic_reporting(), None);

        mso.reliability = 1;
        assert_eq!(
            mso.read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0x40],
            }
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert!(mso
            .property_list()
            .contains(&PropertyIdentifier::EVENT_DETECTION_ENABLE));
        assert!(mso.is_writable_property(PropertyIdentifier::EVENT_DETECTION_ENABLE));
    }
}
