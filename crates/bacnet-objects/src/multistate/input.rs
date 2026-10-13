use super::*;
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::property_metadata::PropertyMetadata;
use std::sync::Arc;

mod metadata;

// ---------------------------------------------------------------------------
// MultiStateInput (type 13)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Input object.
///
/// Read-only multi-state point. Present_Value is writable only when out-of-service.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateInputObject {
    profile: ProfileState,
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    number_of_states: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    reliability_evaluator: MultiStateReliabilityState,
    state_text: Vec<String>,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.18). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
}

impl MultiStateInputObject {
    /// Create a new Multi-state Input object; `number_of_states` must be at least 1.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, instance)?;
        require_nonzero_states(number_of_states)?;
        Ok(Self {
            profile: ProfileState::default(),
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 1,
            number_of_states,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
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
        })
    }

    /// Set the alarm values (states that trigger OFFNORMAL).
    ///
    /// Unlike a network write, which refuses a state past Number_Of_States
    /// (#1429), this takes any state.
    pub fn set_alarm_values(&mut self, values: Vec<u32>) {
        self.event_detector.alarm_values = values;
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
    /// rows before registration. Tags is writable in either Out_Of_Service state;
    /// the text rows are network read-only. Invalid configuration leaves the
    /// previous profile and its attached storage unchanged.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    /// Wait for queued save attempts to finish; writes report their own outcomes.
    /// This is not a success receipt. Unstaged synchronous writes block their caller.
    pub fn wait_for_tag_saves(&self) {
        self.profile.wait_for_saves();
    }

    /// Mutate `Present_Value` on an unattached or otherwise application-owned object.
    ///
    /// This low-level helper bypasses running-server `Out_Of_Service` ownership,
    /// range validation, intrinsic-event processing, and COV processing, while
    /// retaining this object's reliability recomputation. Applications updating
    /// a live object should use `BACnetServer::set_present_value_local`.
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
        let _ = self.recompute_reliability();
    }

    /// Validate and store a `Present_Value` write, without any access check.
    ///
    /// Shared by the network and internal routes, which differ only in the
    /// `Out_Of_Service` condition each requires.
    fn apply_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Unsigned(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if v < 1 || v > self.number_of_states as u64 {
            return Err(common::value_out_of_range_error());
        }
        self.present_value = v as u32;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// Change the locally configured number of states.
    ///
    /// The BACnet `Number_Of_States` property remains read-only. A successful
    /// local change retains the State_Text prefix, truncates its tail on
    /// shrink, appends constructor-style labels on growth, and immediately
    /// re-evaluates Reliability without repairing Present_Value.
    pub fn set_number_of_states(&mut self, number_of_states: u32) -> Result<(), Error> {
        resize_state_text(
            &mut self.number_of_states,
            &mut self.state_text,
            number_of_states,
        )?;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// The values the object keeps that name a state, which a new count
    /// may not strand: Present_Value and Alarm_Values.
    fn held_states(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.present_value).chain(self.event_detector.alarm_values.iter().copied())
    }

    fn recompute_reliability(&mut self) -> ReliabilityEvaluation {
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return ReliabilityEvaluation::Unchanged;
        }
        self.reliability_evaluator.evaluate(
            false,
            self.present_value,
            self.number_of_states,
            &mut self.reliability,
        )
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
        if let Some(result) = self.profile.read(property, array_index) {
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
            return self.apply_present_value(value);
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
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
            self.event_detector.alarm_values = values;
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
        // Clause 12.18 requires simulation/test writes while Out_Of_Service is TRUE:
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

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        // Local safe-ownership policy: preserve the client's OOS simulation.
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.apply_present_value(value)
    }

    fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(self.recompute_reliability())
    }

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
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
        msi.event_detector.acked_transitions =
            bacnet_types::bitstring::EventTransitionBits::empty();
        msi.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
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

#[cfg(test)]
mod reliability_safety_net_tests {
    use super::*;
    use crate::traits::ReliabilityEvaluation;
    use bacnet_types::enums::Reliability;

    #[test]
    fn periodic_hook_repairs_one_stale_bypassed_state_then_is_unchanged() {
        let mut msi = MultiStateInputObject::new(1, "MSI-stale", 2).unwrap();
        msi.present_value = 3;

        assert_eq!(
            msi.evaluate_reliability_internal().unwrap(),
            ReliabilityEvaluation::Changed {
                old_reliability: Reliability::NO_FAULT_DETECTED,
                new_reliability: Reliability::MULTI_STATE_OUT_OF_RANGE,
            }
        );
        assert_eq!(
            msi.evaluate_reliability_internal().unwrap(),
            ReliabilityEvaluation::Unchanged
        );
    }
}
