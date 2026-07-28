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
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    event_time_stamps: [BACnetTimeStamp; 3],
    event_message_texts: [String; 3],
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
            min_pres_value: None,
            max_pres_value: None,
            event_time_stamps: [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ],
            event_message_texts: [String::new(), String::new(), String::new()],
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

    /// Set the minimum present value for fault detection.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set the maximum present value for fault detection.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        if let Some(result) = read_analog_event_properties!(self, property) {
            return result;
        }
        if let Some(result) = read_generic_event_properties!(self, property) {
            return result;
        }
        if property == PropertyIdentifier::EVENT_DETECTION_ENABLE {
            return Ok(PropertyValue::Boolean(self.event_detection_enable));
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
                    BACnetTimeStamp::SequenceNumber(n) => n,
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
        // Clause 12.4, while Out_Of_Service is TRUE: "the Present_Value property and
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
                    self.event_time_stamps = [
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                        BACnetTimeStamp::SequenceNumber(0),
                    ];
                    self.event_message_texts = [String::new(), String::new(), String::new()];
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
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f32> {
        Some(self.cov_increment)
    }

    crate::impl_intrinsic_reporting!(
        event_detector,
        present_value,
        reliability,
        event_detection_enable
    );

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |= transition_bit & 0x07;
        Ok(())
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
        av.event_time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        av.event_message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

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
            av.event_time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            av.event_message_texts,
            [String::new(), String::new(), String::new()]
        );
    }
}
