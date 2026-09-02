use super::*;
use crate::common::{
    read_analog_event_properties, read_generic_event_properties, write_analog_event_properties,
    write_generic_event_properties,
};

// ---------------------------------------------------------------------------
// AnalogValue (type 2)
// ---------------------------------------------------------------------------

/// BACnet Analog Value object.
pub struct AnalogValueObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// 16-level priority array. `None` = no command at that level.
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Event_Detection_Enable (Clause 12.4). Clause 13.2.2.1: "If the
    /// Event_Detection_Enable property is FALSE, then this state machine is not evaluated."
    event_detection_enable: bool,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    reliability_before_out_of_service: Option<u32>,
    reliability_inhibit: common::ReliabilityInhibitState,
    fault_out_of_range: FaultOutOfRangeState,
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    pub(crate) event_history: EventHistory,
    /// Value source tracking.
    value_source: common::ValueSourceTracking,
}

impl AnalogValueObject {
    /// Create a new Analog Value object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0.0,
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
            value_source: common::ValueSourceTracking::default(),
        })
    }

    /// Set the present value directly (bypasses priority array; use when out-of-service
    /// or for initialisation before the priority-array mechanism takes over).
    pub fn set_present_value(&mut self, value: f32) {
        debug_assert!(
            value.is_finite(),
            "set_present_value called with non-finite value"
        );
        self.present_value = value;
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

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (finite Real);
    /// after the store, Present_Value is resolved anew from the priority
    /// array so an empty array falls back to the new default immediately.
    pub fn set_relinquish_default(&mut self, value: f32) -> Result<(), Error> {
        common::reject_non_finite(value)?;
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }
}

impl BACnetObject for AnalogValueObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::UNITS => Ok(PropertyValue::Enumerated(self.units)),
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Real)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Real(self.relinquish_default))
            }
            p if p == PropertyIdentifier::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            p if p == PropertyIdentifier::VALUE_SOURCE => {
                Ok(self.value_source.value_source.clone())
            }
            p if p == PropertyIdentifier::LAST_COMMAND_TIME => Ok(PropertyValue::Unsigned(
                match self.value_source.last_command_time {
                    BACnetTimeStamp::SequenceNumber(n) => u64::from(n),
                    _ => 0,
                },
            )),
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
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        common::write_priority_array_direct!(self, property, array_index, value, |v| {
            if let PropertyValue::Real(f) = v {
                if !f.is_finite() {
                    return Err(common::value_out_of_range_error());
                }
                Ok(f)
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Real(f) = v {
                    if !f.is_finite() {
                        return Err(common::value_out_of_range_error());
                    }
                    Ok(f)
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
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
        // Clause 12.4, while Out_Of_Service is TRUE: "the Present_Value property and
        // the Reliability property, if present and capable of taking on values other
        // than NO_FAULT_DETECTED, shall be writable to allow simulating specific
        // conditions or for testing purposes".
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
            if let PropertyValue::Real(v) = value {
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
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
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
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

    /// AnalogValue is NOT createable: `handle_create_object` has no branch for
    /// it, so PICS must not advertise createability the runtime rejects.
    fn is_createable(&self) -> bool {
        false
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        // Mirrors the AnalogValue `write_property` arms. Same set as
        // AnalogOutput (commandable + common + event properties).
        common::is_commandable_property_writable(property)
            || common::is_common_writable(property)
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

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn av_disabling_detection_resets_event_history() {
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        av.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        av.event_detector.acked_transitions = 0;
        av.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            remaining: 2,
        });
        av.event_detector.fault_reliability = Some(1);
        av.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        av.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        av.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            av.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            av.event_detector.event_state,
            bacnet_types::enums::EventState::NORMAL
        );
        assert_eq!(av.event_detector.acked_transitions, 0b111);
        assert!(av.event_detector.pending.is_none());
        assert!(av.event_detector.fault_reliability.is_none());
        assert_eq!(
            av.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            av.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }
}

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
    fn av_non_finite_monitored_values_preserve_reliability_status_and_ownership() {
        for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut normal = AnalogValueObject::new(1, "AV-normal", 62).unwrap();
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

            let mut owned = AnalogValueObject::new(2, "AV-owned", 62).unwrap();
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
