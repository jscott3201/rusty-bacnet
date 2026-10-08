use super::*;
use crate::event::CommandFailureDetector;
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::property_metadata::PropertyMetadata;
use std::sync::Arc;

mod metadata;

// ---------------------------------------------------------------------------
// MultiStateOutput (type 14)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Output object.
///
/// Commandable multi-state output with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateOutputObject {
    profile: ProfileState,
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
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    reliability_evaluator: MultiStateReliabilityState,
    event_detection_enable: bool,
    state_text: Vec<String>,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    pub(crate) event_history: EventHistory,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
}

impl MultiStateOutputObject {
    /// Create a new Multi-state Output object; `number_of_states` must be at least 1.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, instance)?;
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            profile: ProfileState::default(),
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
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            reliability_evaluator: MultiStateReliabilityState::default(),
            event_detection_enable: false,
            state_text: (1..=number_of_states)
                .map(|i| format!("State {i}"))
                .collect(),
            event_detector: CommandFailureDetector::default(),
            event_history: EventHistory::default(),
            value_source: crate::command_source::ValueSourceTracking::default(),
        })
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name, number_of_states)?;
        object.profile = ProfileState::persistent(object.oid, persistence)?;
        Ok(object)
    }

    /// Provision independent optional Tags, Profile_Location and Profile_Name
    /// rows before registration. Tags is writable independently of commands and
    /// Out_Of_Service; the text rows are network read-only. Invalid configuration
    /// leaves the previous profile and its attached storage unchanged.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    /// Wait for queued save attempts to finish; writes report their own outcomes.
    /// This is not a success receipt. Unstaged synchronous writes block their caller.
    pub fn wait_for_tag_saves(&self) {
        self.profile.wait_for_saves();
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

    /// The values the object keeps that name a state, which a new count
    /// may not strand: the commands in Priority_Array, Present_Value and
    /// Relinquish_Default. Feedback_Value is left out: it is sensed, and
    /// outside the states it is reported as CONFIGURATION_ERROR rather than
    /// refused.
    fn held_states(&self) -> impl Iterator<Item = u32> + '_ {
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .chain([self.present_value, self.relinquish_default])
    }

    fn configuration_invalid(&self) -> bool {
        let is_invalid = |value: u32| !(1..=self.number_of_states).contains(&value);
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .any(is_invalid)
            || is_invalid(self.relinquish_default)
            || is_invalid(self.feedback_value)
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
    /// retained commands, defaults, feedback, and Present_Value are not repaired.
    pub fn set_number_of_states(&mut self, number_of_states: u32) -> Result<(), Error> {
        resize_state_text(
            &mut self.number_of_states,
            &mut self.state_text,
            number_of_states,
        )?;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (Unsigned
    /// 1..=Number_Of_States); after the store, Present_Value is resolved anew
    /// from the priority array so an empty array falls back to the new
    /// default immediately.
    ///
    /// Number_Of_States shrink interplay: if the state count ever shrinks
    /// below this value, the standard leaves adjustment of Priority_Array,
    /// Relinquish_Default, Present_Value, and Feedback_Value to local policy
    /// (Clause 12.19 / Table 12-22 Number_Of_States text). This implementation does
    /// NOT auto-adjust: out-of-range stored values are a configuration
    /// decision for the application to resolve; the object-owned evaluator
    /// reports CONFIGURATION_ERROR while that retained condition remains.
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if value < 1 || value > self.number_of_states {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
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

    crate::event::impl_builtin_intrinsic_reporting!(
        event_detector,
        event_history,
        [present_value, feedback_value],
        reliability,
        event_detection_enable,
        CommandFailureDetector::ALGORITHM
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
        if let Some(result) = self
            .value_source
            .read(property, array_index, &self.priority_array)
        {
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
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(self.event_detection_enable));
        }
        if let Some(value) = self.reliability_inhibit.read(property) {
            return Ok(value);
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
            return result;
        }
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.profile.read(property, array_index) {
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
            return self.value_source.correct(value, priority, origin);
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            let num_states = self.number_of_states;
            return crate::command_source::write_sourced_priority!(
                self,
                value,
                priority,
                origin,
                |v| {
                    if let PropertyValue::Unsigned(u) = v {
                        if u < 1 || u > num_states as u64 {
                            Err(common::value_out_of_range_error())
                        } else {
                            Ok(u as u32)
                        }
                    } else {
                        Err(common::invalid_data_type_error())
                    }
                }
            );
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
        if matches!(
            property,
            PropertyIdentifier::PRESENT_VALUE | PropertyIdentifier::VALUE_SOURCE
        ) {
            return Err(common::write_access_denied_error());
        }
        if property == PropertyIdentifier::FEEDBACK_VALUE {
            if let PropertyValue::Unsigned(u) = value {
                // Checked for representability but deliberately NOT range-checked against
                // Number_Of_States, unlike Present_Value. Clause 12.19 treats a
                // Feedback_Value outside the state set as a condition to be *reported* —
                // for the Number_Of_States-bounded properties apart from Present_Value,
                // an out-of-range value pins Reliability at CONFIGURATION_ERROR until
                // fixed — not as a value to refuse. Feedback_Value reflects a sensed quantity
                // determined by local policy, so it can legitimately fall
                // outside the configured range; refusing it would make CONFIGURATION_ERROR
                // unreachable. The object-owned evaluator applies that reliability.
                //
                // The u32 conversion is still checked. A BACnet Unsigned decodes from up
                // to 8 octets, so a bare `as u32` would wrap a large value back into the
                // valid state range — silently turning a disagreeing feedback into an
                // agreeing one and suppressing the COMMAND_FAILURE transition. That is a
                // representation limit, not a configuration limit, so it is enforced here
                // while the state-set range is not.
                self.feedback_value = common::u64_to_u32(u)?;
                let _ = self.recompute_reliability();
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
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
        // Clause 12.19 requires simulation/test writes while Out_Of_Service is TRUE:
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
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.profile.capability()
    }

    fn advance_monotonic_time_internal(&mut self, now: std::time::Duration) -> bool {
        self.profile.expire(now);
        false
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

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
    }
}

#[cfg(test)]
#[path = "output/tests.rs"]
mod command_failure_tests;

#[cfg(test)]
mod reliability_evaluator_tests;
