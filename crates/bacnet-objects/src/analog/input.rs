use super::*;
use crate::object_profile::TagsPersistence;
use crate::prelude::*;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// AnalogInput (type 0)
// ---------------------------------------------------------------------------

/// BACnet Analog Input object.
pub struct AnalogInputObject {
    pub(super) core: IoCore,
    event_detector: OutOfRangeDetector,
    analog: AnalogProperties,
    present_value: f32,
    fault_out_of_range: FaultOutOfRangeState,
}

impl AnalogInputObject {
    /// Create a new Analog Input object.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
    ) -> Result<Self, Error> {
        Ok(Self {
            core: IoCore::new(ObjectType::ANALOG_INPUT, instance, name.into())?,
            event_detector: OutOfRangeDetector::default(),
            analog: AnalogProperties::new(units),
            present_value: 0.0,
            fault_out_of_range: FaultOutOfRangeState::default(),
        })
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name, units)?;
        object.core.keep_tags(persistence)?;
        Ok(object)
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
}

crate::io_core::impl_profile!(AnalogInputObject);
super::impl_analog_properties!(AnalogInputObject);
super::impl_fault_out_of_range!(AnalogInputObject);
crate::write_config::reporting::impl_reporting_persistence!(AnalogInputObject, in core);

impl BACnetObject for AnalogInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.core.oid
    }

    fn object_name(&self) -> &str {
        &self.core.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(result) = self.analog.read(property) {
            return result;
        }
        if let Some(value) = self.fault_out_of_range.read_limit(property) {
            return Ok(value);
        }
        match property {
            P::PRESENT_VALUE => Ok(PropertyValue::Real(self.present_value)),
            P::PROPERTY_LIST => {
                crate::property_metadata::read_property_list(&self.property_metadata(), array_index)
            }
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        // AI present-value is writable only when out-of-service
        if property == P::PRESENT_VALUE {
            if !self.core.out_of_service {
                return Err(common::write_access_denied_error());
            }
            return self.apply_present_value(value);
        }
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            return result;
        }
        if let Some(result) = self.analog.write(property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
        ))
    }

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.durable_capability()
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(metadata::INPUT);
        self.fault_out_of_range.add_rows(&mut rows);
        self.analog.add_rows(&mut rows);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    crate::traits::lend_cov_reporting!();

    crate::traits::lend_event_reporting!();

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

crate::event::impl_builtin_intrinsic_reporting!(
    AnalogInputObject,
    event_detector,
    core.event_history,
    [present_value],
    core.reliability,
    core.event_detection_enable,
    OutOfRangeDetector::ALGORITHM
);

/// The Present_Value the application supplies.
impl crate::traits::LocalInput for AnalogInputObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::PresentValue { value, .. } = input else {
            return Err(crate::traits::not_supported());
        };
        // Local safe-ownership policy: preserve the client's OOS simulation.
        if self.core.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.apply_present_value(value)
    }
}

#[cfg(test)]
mod detection_enable_reset_tests {
    use super::*;
    use crate::event::test_clock::test_now;
    #[cfg(feature = "event-reporting")]
    use crate::event::test_clock::test_tick;
    use crate::traits::EventReporting as _;
    use bacnet_types::error::RemoteError;

    #[test]
    fn built_in_intrinsic_transition_is_a_proposal_until_committed() {
        use crate::event::EventTransitionCommit;

        let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        ai.event_detector.high_limit = 80.0;
        ai.event_detector.limit_enable = bacnet_types::bitstring::LimitEnable::all();
        ai.event_detector.event_enable = bacnet_types::bitstring::EventTransitionBits::all();
        ai.set_present_value(81.0);

        let outcome = ai
            .evaluate_intrinsic_reporting(test_now())
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
        assert_eq!(
            ai.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::all()
        );
        assert!(ai.event_detector.pending.is_none());

        ai.commit_event_transition(EventTransitionCommit {
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
        assert_eq!(
            ai.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::TO_FAULT
                | bacnet_types::bitstring::EventTransitionBits::TO_NORMAL
        );
        assert_eq!(
            ai.core.event_history.time_stamps[0],
            BACnetTimeStamp::SequenceNumber(41)
        );
    }

    #[cfg(feature = "event-reporting")]
    #[test]
    fn rejected_delayed_and_fault_reindication_proposals_remain_retryable() {
        use crate::event::{
            EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
        };
        use bacnet_types::enums::{EngineeringUnits, EventState, Reliability};

        let mut delayed =
            AnalogInputObject::new(1, "AI-delayed", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        delayed.event_detector.high_limit = 80.0;
        delayed.event_detector.limit_enable = bacnet_types::bitstring::LimitEnable::all();
        delayed.event_detector.time_delay = 1;
        delayed.set_present_value(81.0);
        assert_eq!(delayed.evaluate_intrinsic_reporting(test_now()), None);
        let proposal = delayed.tick_intrinsic_reporting(test_tick()).unwrap();
        // Fire-ready: still pending, due now (the countdown held it at 1).
        assert_eq!(
            delayed.event_detector.pending.as_ref().unwrap().remaining(),
            0
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
            delayed.commit_event_transition(stale),
            Err(EventTransitionCommitError::CurrentStateMismatch {
                expected: EventState::FAULT,
                actual: EventState::NORMAL,
            })
        );
        assert_eq!(
            delayed.event_detector.pending.as_ref().unwrap().remaining(),
            0
        );
        assert_eq!(
            delayed.tick_intrinsic_reporting(test_tick()),
            Some(proposal)
        );

        let mut faulted =
            AnalogInputObject::new(2, "AI-fault", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        faulted.core.reliability = Reliability::OVER_RANGE;
        let entry = faulted.evaluate_intrinsic_reporting(test_now()).unwrap();
        crate::event::commit_test_proposal(&mut faulted, entry);
        faulted.core.reliability = Reliability::NO_SENSOR;
        let reindication = faulted.evaluate_intrinsic_reporting(test_now()).unwrap();
        assert_eq!(
            reindication.change,
            EventStateChange {
                from: EventState::FAULT,
                to: EventState::FAULT,
            }
        );
        assert_eq!(
            faulted.commit_event_transition(EventTransitionCommit {
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
            Some(Reliability::OVER_RANGE)
        );
        assert_eq!(
            faulted.evaluate_intrinsic_reporting(test_now()),
            Some(reindication)
        );
    }

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn ai_disabling_detection_resets_event_history() {
        let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        assert_eq!(
            ai.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        ai.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ai.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
        ai.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            due: crate::event::test_clock::in_seconds(2),
        });
        ai.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        ai.core.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        ai.core.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

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
        assert_eq!(
            ai.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::all()
        );
        assert!(ai.event_detector.pending.is_none());
        assert!(ai.event_detector.fault_reliability.is_none());
        assert_eq!(
            ai.core.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            ai.core.event_history.original_from_states,
            [None, None, None]
        );
        assert_eq!(ai.core.event_history.original_to_states, [None, None, None]);
        assert_eq!(
            ai.core.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }

    #[test]
    fn ai_rejected_detection_write_preserves_hidden_state() {
        let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        ai.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ai.event_detector.acked_transitions =
            bacnet_types::bitstring::EventTransitionBits::TO_FAULT;
        ai.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::NORMAL,
            due: crate::event::test_clock::in_seconds(2),
        });
        ai.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        ai.core.event_history.time_stamps[0] = BACnetTimeStamp::SequenceNumber(7);
        ai.core.event_history.original_from_states[0] = Some(EventState::NORMAL);
        ai.core.event_history.original_to_states[0] = Some(EventState::HIGH_LIMIT);
        ai.core.event_history.message_texts[0] = "offnormal".into();
        let error = ai
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Unsigned(0),
                None,
            )
            .unwrap_err();
        assert!(
            matches!(error, bacnet_types::error::Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::INVALID_DATA_TYPE)
        );

        assert!(ai.core.event_detection_enable);
        assert_eq!(
            ai.event_detector.event_state,
            bacnet_types::enums::EventState::HIGH_LIMIT
        );
        assert_eq!(
            ai.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::TO_FAULT
        );
        assert_eq!(ai.event_detector.pending.unwrap().remaining(), 2);
        assert_eq!(
            ai.event_detector.fault_reliability,
            Some(bacnet_types::enums::Reliability::NO_SENSOR)
        );
        assert_eq!(
            ai.core.event_history.time_stamps[0],
            BACnetTimeStamp::SequenceNumber(7)
        );
        assert_eq!(
            ai.core.event_history.original_from_states[0],
            Some(EventState::NORMAL)
        );
        assert_eq!(
            ai.core.event_history.original_to_states[0],
            Some(EventState::HIGH_LIMIT)
        );
        assert_eq!(ai.core.event_history.message_texts[0], "offnormal");
    }
}

#[cfg(test)]
mod acknowledge_alarm_correlation_tests;

#[cfg(test)]
mod fault_out_of_range_non_finite_tests {
    use super::*;
    use bacnet_types::enums::{ErrorClass, ErrorCode, Reliability};
    use bacnet_types::error::RemoteError;

    fn assert_value_out_of_range(result: Result<ReliabilityEvaluation, Error>) {
        assert!(matches!(
            result,
            Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::VALUE_OUT_OF_RANGE
        ));
    }

    #[test]
    fn ai_non_finite_monitored_values_preserve_reliability_status_and_ownership() {
        for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut normal =
                AnalogInputObject::new(1, "AI-normal", EngineeringUnits::DEGREES_CELSIUS).unwrap();
            normal.configure_fault_out_of_range(10.0, 20.0).unwrap();
            let status_before = normal
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            normal.present_value = non_finite;

            assert_value_out_of_range(normal.reliability_eval().unwrap().evaluate_reliability());
            assert_eq!(normal.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(normal.core.reliability, Reliability::NO_FAULT_DETECTED);
            assert_eq!(
                normal
                    .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                    .unwrap(),
                status_before
            );
            assert!(normal.fault_out_of_range.owned_fault.is_none());

            normal.present_value = 9.0;
            assert_eq!(
                normal
                    .reliability_eval()
                    .unwrap()
                    .evaluate_reliability()
                    .unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::NO_FAULT_DETECTED,
                    new_reliability: Reliability::UNDER_RANGE,
                }
            );

            let mut owned =
                AnalogInputObject::new(2, "AI-owned", EngineeringUnits::DEGREES_CELSIUS).unwrap();
            owned.configure_fault_out_of_range(10.0, 20.0).unwrap();
            owned.present_value = 9.0;
            owned
                .reliability_eval()
                .unwrap()
                .evaluate_reliability()
                .unwrap();
            let status_before = owned
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            owned.present_value = non_finite;

            assert_value_out_of_range(owned.reliability_eval().unwrap().evaluate_reliability());
            assert_eq!(owned.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(owned.core.reliability, Reliability::UNDER_RANGE);
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
                owned
                    .reliability_eval()
                    .unwrap()
                    .evaluate_reliability()
                    .unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::UNDER_RANGE,
                    new_reliability: Reliability::OVER_RANGE,
                }
            );
        }
    }
}
