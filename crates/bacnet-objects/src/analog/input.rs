use super::*;
use crate::common::{
    read_analog_event_properties, read_generic_event_properties, write_analog_event_properties,
    write_generic_event_properties,
};

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
    /// Event_Detection_Enable (Clause 12.2). Clause 13.2.2.1: "If the
    /// Event_Detection_Enable property is FALSE, then this state machine is not evaluated."
    event_detection_enable: bool,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    reliability_before_out_of_service: Option<u32>,
    reliability_inhibit: common::ReliabilityInhibitState,
    fault_out_of_range: FaultOutOfRangeState,
    /// Optional minimum engineering bound metadata for Present_Value.
    min_pres_value: Option<f32>,
    /// Optional maximum engineering bound metadata for Present_Value.
    max_pres_value: Option<f32>,
    pub(crate) event_history: EventHistory,
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
            event_detection_enable: true,
            reliability: 0,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            fault_out_of_range: FaultOutOfRangeState::default(),
            min_pres_value: None,
            max_pres_value: None,
            event_history: EventHistory::default(),
        })
    }

    /// Mutate `Present_Value` on an unattached or otherwise application-owned object.
    ///
    /// This low-level helper bypasses running-server `Out_Of_Service` ownership,
    /// validation, intrinsic-event processing, and COV processing. Applications
    /// updating a live object should use `BACnetServer::set_present_value_local`.
    pub fn set_present_value(&mut self, value: f32) {
        debug_assert!(
            value.is_finite(),
            "set_present_value called with non-finite value"
        );
        self.present_value = value;
    }

    /// Validate and store a `Present_Value` write, without any access check.
    ///
    /// Shared by the network and internal routes, which differ only in the
    /// `Out_Of_Service` condition each requires.
    fn apply_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Real(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if !v.is_finite() {
            return Err(common::value_out_of_range_error());
        }
        self.present_value = v;
        Ok(())
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set minimum engineering-bound metadata; this is not a reliability fault limit.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set maximum engineering-bound metadata; this is not a reliability fault limit.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }

    /// Configure the optional object-owned FAULT_OUT_OF_RANGE algorithm.
    ///
    /// Both limits become readable together. Equal limits are valid; non-finite
    /// limits and a low limit greater than the high limit are rejected without
    /// changing the prior configuration.
    pub fn configure_fault_out_of_range(&mut self, low: f32, high: f32) -> Result<(), Error> {
        self.fault_out_of_range.configure(low, high)
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
        if let Some(value) = self.reliability_inhibit.read(property) {
            return Ok(value);
        }
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = read_analog_event_properties!(self, property) {
            return result;
        }
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
            return result;
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(self.event_detection_enable));
        }
        if let Some(value) = self.fault_out_of_range.read_limit(property) {
            return Ok(value);
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
            return self.apply_present_value(value);
        }
        if let Some(result) = self.reliability_inhibit.write_inhibit(
            &mut self.reliability,
            self.out_of_service,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = self.reliability_inhibit.write_out_of_service(
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
        if let Some(result) = self.reliability_inhibit.write_client_reliability(
            self.out_of_service,
            &mut self.reliability,
            property,
            &value,
        ) {
            return result;
        }
        if let Some(result) = common::write_cov_increment(&mut self.cov_increment, property, &value)
        {
            return result;
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
        if let Some(result) = write_analog_event_properties!(self, property, value) {
            return result;
        }
        if let Some(result) = write_generic_event_properties!(self, property, value) {
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
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
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
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        self.fault_out_of_range.property_list(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    crate::event::impl_builtin_intrinsic_reporting!(
        event_detector,
        event_history,
        [present_value],
        reliability,
        event_detection_enable,
        OutOfRangeDetector::ALGORITHM
    );
    impl_intrinsic_write_rollback!(
        event_detector,
        event_detection_enable,
        event_history,
        reliability_inhibit,
        reliability,
        out_of_service,
        reliability_before_out_of_service,
        fault_out_of_range
    );

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
    }

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

    fn set_reliability_internal(&mut self, reliability: u32) -> Result<(), Error> {
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        self.fault_out_of_range.clear_ownership();
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
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return Ok(ReliabilityEvaluation::Unchanged);
        }
        self.fault_out_of_range
            .evaluate(self.present_value, &mut self.reliability)
    }

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
    }

    fn is_createable(&self) -> bool {
        true
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Mirrors the AnalogInput `write_property` arms.
        common::is_common_writable(property)
            || property == PropertyIdentifier::PRESENT_VALUE
            || property == PropertyIdentifier::RELIABILITY
            || property == PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT
            || property == PropertyIdentifier::COV_INCREMENT
            || common::is_event_property_writable(property)
            || property == PropertyIdentifier::EVENT_DETECTION_ENABLE
    }
}

#[cfg(test)]
mod detection_enable_reset_tests {
    use super::*;

    #[test]
    fn built_in_intrinsic_transition_is_a_proposal_until_committed() {
        use crate::event::EventTransitionCommit;

        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.event_detector.high_limit = 80.0;
        ai.event_detector.limit_enable = crate::event::LimitEnable::BOTH;
        ai.event_detector.event_enable = 0x07;
        ai.set_present_value(81.0);

        let outcome = ai
            .evaluate_intrinsic_reporting()
            .expect("out-of-range value should propose TO_OFFNORMAL");
        assert_eq!(
            outcome.change.to,
            bacnet_types::enums::EventState::HIGH_LIMIT
        );
        assert_eq!(
            ai.event_detector.event_state,
            bacnet_types::enums::EventState::NORMAL,
            "built-in evaluation must not confirm its own proposal"
        );
        assert_eq!(ai.event_detector.acked_transitions, 0b111);
        assert!(ai.event_detector.pending.is_none());

        ai.commit_event_transition_internal(EventTransitionCommit {
            coordinate: outcome.change.transition(),
            change: outcome.change,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(41),
            message_text: None,
        })
        .expect("built-in object should lend all transition state to the kernel");

        assert_eq!(
            ai.event_detector.event_state,
            bacnet_types::enums::EventState::HIGH_LIMIT
        );
        assert_eq!(ai.event_detector.acked_transitions, 0b110);
        assert_eq!(
            ai.event_history.time_stamps[0],
            BACnetTimeStamp::SequenceNumber(41)
        );
    }

    #[test]
    fn rejected_delayed_and_fault_reindication_proposals_remain_retryable() {
        use crate::event::{
            EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
        };
        use bacnet_types::enums::{EventState, Reliability};

        let mut delayed = AnalogInputObject::new(1, "AI-delayed", 62).unwrap();
        delayed.event_detector.high_limit = 80.0;
        delayed.event_detector.limit_enable = crate::event::LimitEnable::BOTH;
        delayed.event_detector.time_delay = 1;
        delayed.set_present_value(81.0);
        assert_eq!(delayed.evaluate_intrinsic_reporting(), None);
        let proposal = delayed.tick_intrinsic_reporting().unwrap();
        assert_eq!(
            delayed.event_detector.pending.as_ref().unwrap().remaining,
            1
        );

        let stale = EventTransitionCommit {
            change: EventStateChange {
                from: EventState::FAULT,
                to: proposal.change.to,
            },
            coordinate: proposal.change.transition(),
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(9),
            message_text: None,
        };
        assert_eq!(
            delayed.commit_event_transition_internal(stale),
            Err(EventTransitionCommitError::CurrentStateMismatch {
                expected: EventState::FAULT,
                actual: EventState::NORMAL,
            })
        );
        assert_eq!(
            delayed.event_detector.pending.as_ref().unwrap().remaining,
            1
        );
        assert_eq!(delayed.tick_intrinsic_reporting(), Some(proposal));

        let mut faulted = AnalogInputObject::new(2, "AI-fault", 62).unwrap();
        faulted.reliability = Reliability::OVER_RANGE.to_raw();
        let entry = faulted.evaluate_intrinsic_reporting().unwrap();
        crate::event::commit_test_proposal(&mut faulted, entry);
        faulted.reliability = Reliability::NO_SENSOR.to_raw();
        let reindication = faulted.evaluate_intrinsic_reporting().unwrap();
        assert_eq!(
            reindication.change,
            EventStateChange {
                from: EventState::FAULT,
                to: EventState::FAULT,
            }
        );
        assert_eq!(
            faulted.commit_event_transition_internal(EventTransitionCommit {
                change: reindication.change.clone(),
                coordinate: EventTransition::ToNormal,
                ack_required: false,
                timestamp: BACnetTimeStamp::SequenceNumber(10),
                message_text: None,
            }),
            Err(EventTransitionCommitError::CoordinateTargetMismatch {
                coordinate: EventTransition::ToNormal,
                target: EventState::FAULT,
            })
        );
        assert_eq!(
            faulted.event_detector.fault_reliability,
            Some(Reliability::OVER_RANGE.to_raw())
        );
        assert_eq!(faulted.evaluate_intrinsic_reporting(), Some(reindication));
    }

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn ai_disabling_detection_resets_event_history() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        assert_eq!(
            ai.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        ai.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ai.event_detector.acked_transitions = 0;
        ai.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            remaining: 2,
        });
        ai.event_detector.fault_reliability = Some(1);
        ai.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        ai.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        ai.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            ai.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            ai.event_detector.event_state,
            bacnet_types::enums::EventState::NORMAL
        );
        assert_eq!(ai.event_detector.acked_transitions, 0b111);
        assert!(ai.event_detector.pending.is_none());
        assert!(ai.event_detector.fault_reliability.is_none());
        assert_eq!(
            ai.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            ai.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }

    #[test]
    fn ai_write_rollback_restores_detection_state() {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        ai.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ai.event_detector.acked_transitions = 0b010;
        ai.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::NORMAL,
            remaining: 2,
        });
        ai.event_detector.fault_reliability = Some(1);
        ai.event_history.time_stamps[0] = BACnetTimeStamp::SequenceNumber(7);
        ai.event_history.original_to_states[0] = Some(EventState::HIGH_LIMIT);
        ai.event_history.message_texts[0] = "offnormal".into();
        let rollback = ai
            .capture_write_property_rollback(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                &PropertyValue::Boolean(false),
            )
            .unwrap();

        ai.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
        ai.restore_write_property_rollback(rollback).unwrap();

        assert!(ai.event_detection_enable);
        assert_eq!(
            ai.event_detector.event_state,
            bacnet_types::enums::EventState::HIGH_LIMIT
        );
        assert_eq!(ai.event_detector.acked_transitions, 0b010);
        assert_eq!(ai.event_detector.pending.unwrap().remaining, 2);
        assert_eq!(ai.event_detector.fault_reliability, Some(1));
        assert_eq!(
            ai.event_history.time_stamps[0],
            BACnetTimeStamp::SequenceNumber(7)
        );
        assert_eq!(
            ai.event_history.original_to_states[0],
            Some(EventState::HIGH_LIMIT)
        );
        assert_eq!(ai.event_history.message_texts[0], "offnormal");
    }
}

#[cfg(test)]
mod acknowledge_alarm_correlation_tests;

#[cfg(test)]
mod fault_out_of_range_non_finite_tests {
    use super::*;
    use bacnet_types::enums::{ErrorClass, ErrorCode, Reliability};

    fn assert_value_out_of_range(result: Result<ReliabilityEvaluation, Error>) {
        assert!(matches!(
            result,
            Err(Error::Protocol { class, code })
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32
        ));
    }

    #[test]
    fn ai_non_finite_monitored_values_preserve_reliability_status_and_ownership() {
        for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut normal = AnalogInputObject::new(1, "AI-normal", 62).unwrap();
            normal.configure_fault_out_of_range(10.0, 20.0).unwrap();
            let status_before = normal
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            normal.present_value = non_finite;

            assert_value_out_of_range(normal.evaluate_reliability_internal());
            assert_eq!(normal.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(normal.reliability, Reliability::NO_FAULT_DETECTED.to_raw());
            assert_eq!(
                normal
                    .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                    .unwrap(),
                status_before
            );
            assert!(normal.fault_out_of_range.owned_fault.is_none());

            normal.present_value = 9.0;
            assert_eq!(
                normal.evaluate_reliability_internal().unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::NO_FAULT_DETECTED.to_raw(),
                    new_reliability: Reliability::UNDER_RANGE.to_raw(),
                }
            );

            let mut owned = AnalogInputObject::new(2, "AI-owned", 62).unwrap();
            owned.configure_fault_out_of_range(10.0, 20.0).unwrap();
            owned.present_value = 9.0;
            owned.evaluate_reliability_internal().unwrap();
            let status_before = owned
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            owned.present_value = non_finite;

            assert_value_out_of_range(owned.evaluate_reliability_internal());
            assert_eq!(owned.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(owned.reliability, Reliability::UNDER_RANGE.to_raw());
            assert_eq!(
                owned
                    .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                    .unwrap(),
                status_before
            );
            assert!(matches!(
                owned.fault_out_of_range.owned_fault,
                Some(OwnedRangeFault::UnderRange)
            ));

            owned.present_value = 21.0;
            assert_eq!(
                owned.evaluate_reliability_internal().unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::UNDER_RANGE.to_raw(),
                    new_reliability: Reliability::OVER_RANGE.to_raw(),
                }
            );
        }
    }
}
