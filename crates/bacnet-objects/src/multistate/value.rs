use super::*;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;

mod metadata;

// ---------------------------------------------------------------------------
// MultiStateValue (type 19)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Value object.
///
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    access: PresentValueAccess,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    reliability_evaluator: MultiStateReliabilityState,
    state_text: Vec<String>,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.20). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
    /// The last writer of a noncommandable Present_Value, once tracked (#1552).
    write_source: crate::command_source::SingleValueSource,
}

impl MultiStateValueObject {
    /// Create a new Multi-State Value object with a commandable Present_Value.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        Self::with_access(
            instance,
            name,
            number_of_states,
            PresentValueAccess::Commandable,
        )
    }

    /// Create a new Multi-State Value object whose Present_Value is written as `access` says.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
        access: PresentValueAccess,
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
            access,
            priority_array: [None; 16],
            relinquish_default: 1,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            reliability_evaluator: MultiStateReliabilityState::default(),
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            event_detector: ChangeOfStateDetector::default(),
            event_detection_enable: true,
            event_history: EventHistory::default(),
            value_source: crate::command_source::ValueSourceTracking::default(),
            write_source: crate::command_source::SingleValueSource::default(),
        })
    }

    /// Track the source of a noncommandable Present_Value (Clause 19.5,
    /// #1552), as [`AnalogValueObject::set_value_source_tracking`] does.
    ///
    /// [`AnalogValueObject::set_value_source_tracking`]: crate::analog::AnalogValueObject::set_value_source_tracking
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.write_source.set_enabled(enabled);
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
        let _ = self.recompute_reliability();
    }

    fn configuration_invalid(&self) -> bool {
        let is_invalid = |value: u32| !(1..=self.number_of_states).contains(&value);
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .any(is_invalid)
            || is_invalid(self.relinquish_default)
            || self
                .event_detector
                .alarm_values
                .iter()
                .copied()
                .any(is_invalid)
    }

    fn recompute_reliability(&mut self) -> ReliabilityEvaluation {
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return ReliabilityEvaluation::Unchanged;
        }
        let configuration_invalid = self.configuration_invalid();
        self.reliability_evaluator.evaluate(
            configuration_invalid,
            self.present_value,
            self.number_of_states,
            &mut self.reliability,
        )
    }

    /// Change the locally configured number of states.
    ///
    /// The BACnet `Number_Of_States` property remains read-only. A successful
    /// local change resizes State_Text and immediately re-evaluates Reliability;
    /// retained commands, defaults, alarms, and Present_Value are not repaired.
    pub fn set_number_of_states(&mut self, number_of_states: u32) -> Result<(), Error> {
        resize_state_text(
            &mut self.number_of_states,
            &mut self.state_text,
            number_of_states,
        )?;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// Set the alarm values and synchronously re-evaluate configuration Reliability.
    ///
    /// Unlike a network write, which refuses a state past Number_Of_States
    /// (#1429), this takes any state; one past the count shows as
    /// CONFIGURATION_ERROR.
    pub fn set_alarm_values(&mut self, values: Vec<u32>) {
        self.event_detector.alarm_values = values;
        let _ = self.recompute_reliability();
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (Unsigned
    /// 1..=Number_Of_States); after the store, Present_Value is resolved anew
    /// from the priority array so an empty array falls back to the new
    /// default immediately.
    ///
    /// Number_Of_States shrink interplay: if the state count ever shrinks
    /// below this value, retained Priority_Array, Relinquish_Default,
    /// Present_Value, and Alarm_Values are not auto-adjusted (Clause 12.20 /
    /// Table 12-23). An out-of-range retained value is a configuration decision
    /// for the application to resolve; the object-owned evaluator reports
    /// CONFIGURATION_ERROR while that condition remains.
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        if value < 1 || value > self.number_of_states {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    fn checked_present_value(number_of_states: u32, value: PropertyValue) -> Result<u32, Error> {
        let PropertyValue::Unsigned(u) = value else {
            return Err(common::invalid_data_type_error());
        };
        if u < 1 || u > u64::from(number_of_states) {
            return Err(common::value_out_of_range_error());
        }
        Ok(u as u32)
    }

    /// The values the object keeps that name a state, which a new count
    /// may not strand: the commands in Priority_Array, Present_Value,
    /// Relinquish_Default and Alarm_Values.
    fn held_states(&self) -> impl Iterator<Item = u32> + '_ {
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .chain([self.present_value, self.relinquish_default])
            .chain(self.event_detector.alarm_values.iter().copied())
    }

    /// A noncommandable Present_Value write, network-equivalent.
    fn write_direct_present_value(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        if array_index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        if self.access == PresentValueAccess::ReadOnly && !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        // Clause 19.2: an otherwise permitted noncommandable NULL is a no-op.
        if value == PropertyValue::Null {
            return Ok(());
        }
        self.set_present_value_directly(value)
    }

    fn set_present_value_directly(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.present_value = Self::checked_present_value(self.number_of_states, value)?;
        let _ = self.recompute_reliability();
        Ok(())
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

    crate::event::impl_builtin_intrinsic_reporting!(
        event_detector,
        event_history,
        [present_value],
        reliability,
        event_detection_enable,
        ChangeOfStateDetector::ALGORITHM
    );

    fn acknowledge_alarm_correlated_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<(), Error> {
        if !self.event_detection_enable {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::NO_ALARM_CONFIGURED.to_raw() as u32,
            });
        }
        self.event_history.acknowledge_correlated(
            &mut self.event_detector.acked_transitions,
            event_state,
            timestamp,
        )
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if metadata::excludes(self, property) {
            return Err(common::unknown_property_error());
        }
        let source = match self.access {
            PresentValueAccess::Commandable => {
                self.value_source
                    .read(property, array_index, &self.priority_array)
            }
            _ => self.write_source.read(property, array_index),
        };
        if let Some(result) = source {
            return result;
        }

        if property == PropertyIdentifier::STATUS_FLAGS {
            return Ok(common::compute_status_flags(
                self.status_flags,
                self.reliability,
                self.out_of_service,
                self.event_detector.event_state,
            ));
        }
        if let Some(value) = self.reliability_inhibit.read(property) {
            return Ok(value);
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

    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        if matches!(
            property,
            PropertyIdentifier::PRESENT_VALUE | PropertyIdentifier::VALUE_SOURCE
        ) && array_index.is_some()
        {
            return Err(common::property_is_not_an_array_error());
        }
        if property == PropertyIdentifier::VALUE_SOURCE {
            return match self.access {
                PresentValueAccess::Commandable => {
                    self.value_source.correct(value, priority, origin)
                }
                _ => self.write_source.correct(array_index, value, origin),
            };
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            let number_of_states = self.number_of_states;
            return match self.access {
                PresentValueAccess::Commandable => {
                    crate::command_source::write_sourced_priority!(
                        self,
                        value,
                        priority,
                        origin,
                        |v| Self::checked_present_value(number_of_states, v)
                    )
                }
                PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                    self.write_source.admit(origin)?;
                    // A NULL is a no-op (Clause 19.2), so it has no writer.
                    let null = value == PropertyValue::Null;
                    self.write_direct_present_value(array_index, value)?;
                    if !null {
                        self.write_source.record(origin);
                    }
                    Ok(())
                }
            };
        }
        self.write_property(property, array_index, value, priority)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if metadata::excludes(self, property) {
            return Err(common::unknown_property_error());
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            if self.access == PresentValueAccess::Commandable {
                return Err(common::write_access_denied_error());
            }
            // A tracked source needs the writer (`write_property_from`).
            self.write_source.unsourced()?;
            return self.write_direct_present_value(array_index, value);
        }
        if property == PropertyIdentifier::VALUE_SOURCE {
            return Err(common::write_access_denied_error());
        }
        if property == PropertyIdentifier::STATE_TEXT {
            // Written whole, State_Text sets Number_Of_States too (#1443).
            let held: Vec<u32> = self.held_states().collect();
            write_state_text(
                &mut self.number_of_states,
                &mut self.state_text,
                held,
                array_index,
                value,
            )?;
            let _ = self.recompute_reliability();
            return Ok(());
        }
        if property == PropertyIdentifier::ALARM_VALUES {
            let values = decode_alarm_values_write(array_index, value, self.number_of_states)?;
            self.set_alarm_values(values);
            return Ok(());
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.event_detection_enable = v;
                if !v {
                    self.event_detector.event_state = bacnet_types::enums::EventState::NORMAL;
                    self.event_detector.acked_transitions =
                        bacnet_types::bitstring::EventTransitionBits::all();
                    self.event_detector.pending = None;
                    self.event_detector.fault_reliability = None;
                    self.event_history.reset();
                }
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
        if let Some(result) =
            self.event_history
                .write(property, array_index, &value, self.event_detection_enable)
        {
            return result;
        }
        if let Some(result) = write_generic_event_properties!(self, property, value) {
            return result;
        }
        if let Some(result) = self.reliability_inhibit.write_inhibit(
            &mut self.reliability,
            self.out_of_service,
            property,
            &value,
        ) {
            result?;
            let _ = self.recompute_reliability();
            return Ok(());
        }
        if let Some(result) = self.reliability_inhibit.write_out_of_service(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            if result? == crate::reliability_inhibit::OutOfServiceWrite::Applied {
                let _ = self.recompute_reliability();
            }
            return Ok(());
        }
        if let Some(result) = common::write_object_name(&mut self.name, property, &value) {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        // Clause 12.20 requires simulation/test writes while Out_Of_Service is TRUE:
        // Present_Value is writable, as is Reliability when that property exists
        // and supports values beyond NO_FAULT_DETECTED.
        // `is_writable_property` stays statically true because it describes capability.
        if let Some(result) = self.reliability_inhibit.write_client_reliability(
            self.out_of_service,
            &mut self.reliability,
            property,
            &value,
        ) {
            return result;
        }
        if property == PropertyIdentifier::RELINQUISH_DEFAULT {
            if let PropertyValue::Unsigned(u) = value {
                let v = common::u64_to_u32(u)?;
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn is_createable(&self) -> bool {
        true
    }
    fn creation_only_properties(&self) -> &'static [PropertyIdentifier] {
        CREATION_ONLY
    }
    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let held: Vec<u32> = self.held_states().collect();
        initialize_states(
            &mut self.number_of_states,
            &mut self.state_text,
            held,
            property,
            value,
        )?;
        let _ = self.recompute_reliability();
        Ok(())
    }
    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        self.reliability_evaluator.clear_ownership();
        Ok(())
    }

    fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(self.recompute_reliability())
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        match self.access {
            PresentValueAccess::Commandable => {
                Err(common::optional_functionality_not_supported_error())
            }
            _ if self.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.set_present_value_directly(value)?;
                self.write_source.forget();
                Ok(())
            }
        }
    }

    fn set_present_value_from_internal(
        &mut self,
        value: PropertyValue,
        origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.write_source.admit(origin)?;
        self.set_present_value_internal(value)?;
        self.write_source.record(origin);
        Ok(())
    }

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
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
        msv.write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
        assert_eq!(msv.evaluate_intrinsic_reporting(), None);
        assert!(msv.event_detector.pending.is_some());

        msv.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
        msv.event_detector.acked_transitions =
            bacnet_types::bitstring::EventTransitionBits::empty();
        msv.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
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

#[cfg(test)]
mod reliability_evaluator_tests {
    use super::*;
    use bacnet_types::enums::Reliability;

    #[test]
    fn configuration_error_dominates_bypassed_invalid_present_value() {
        let mut msv = MultiStateValueObject::new(1, "MSV-dominance", 2).unwrap();
        msv.present_value = 3;
        msv.event_detector.alarm_values = vec![3];

        msv.evaluate_reliability_internal().unwrap();
        assert_eq!(
            msv.reliability,
            Reliability::CONFIGURATION_ERROR,
            "invalid configuration must dominate invalid Present_Value"
        );
        msv.event_detector.alarm_values = vec![1];
        msv.evaluate_reliability_internal().unwrap();
        assert_eq!(msv.reliability, Reliability::MULTI_STATE_OUT_OF_RANGE);
        msv.set_number_of_states(3).unwrap();
        assert_eq!(
            msv.reliability,
            Reliability::NO_FAULT_DETECTED,
            "count growth must synchronously recover the retained Present_Value"
        );
    }
}
