use super::*;

// ---------------------------------------------------------------------------
// BinaryOutput (type 4)
// ---------------------------------------------------------------------------

/// BACnet Binary Output object.
///
/// Commandable binary output with 16-level priority array.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    feedback_value: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    reliability_before_out_of_service: Option<u32>,
    event_detection_enable: bool,
    active_text: String,
    inactive_text: String,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    pub(crate) event_history: EventHistory,
    /// Value source tracking (optional per spec — exposed via VALUE_SOURCE property).
    value_source: common::ValueSourceTracking,
}

impl BinaryOutputObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            feedback_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            priority_array: [None; 16],
            relinquish_default: 0,
            polarity: 0,
            reliability: 0,
            reliability_before_out_of_service: None,
            event_detection_enable: false,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
            event_detector: CommandFailureDetector::default(),
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

impl BACnetObject for BinaryOutputObject {
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
        if let Some(result) = self.event_history.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::BINARY_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::FEEDBACK_VALUE => {
                Ok(PropertyValue::Enumerated(self.feedback_value))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default))
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
            p if p == PropertyIdentifier::POLARITY => Ok(PropertyValue::Enumerated(self.polarity)),
            p if p == PropertyIdentifier::ACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.active_text.clone()))
            }
            p if p == PropertyIdentifier::INACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.inactive_text.clone()))
            }
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
            if let PropertyValue::Enumerated(e) = v {
                if e > 1 {
                    Err(common::value_out_of_range_error())
                } else {
                    Ok(e)
                }
            } else {
                Err(common::invalid_data_type_error())
            }
        });
        if property == PropertyIdentifier::PRESENT_VALUE {
            return common::write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Enumerated(e) = v {
                    if e > 1 {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(e)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }
        if property == PropertyIdentifier::FEEDBACK_VALUE {
            if let PropertyValue::Enumerated(e) = value {
                if e > 1 {
                    return Err(common::value_out_of_range_error());
                }
                self.feedback_value = e;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::ACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.active_text = s;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::INACTIVE_TEXT {
            if let PropertyValue::CharacterString(s) = value {
                self.inactive_text = s;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
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
        // Clause 12.7, while Out_Of_Service is TRUE: "the Present_Value property and
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
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            PropertyIdentifier::POLARITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
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
        // Mirrors the BinaryOutput `write_property` arms: commandable
        // (PRIORITY_ARRAY + PRESENT_VALUE) + common + text properties.
        common::is_commandable_property_writable(property)
            || common::is_common_writable(property)
            || common::is_generic_event_property_writable(property)
            || property == PropertyIdentifier::FEEDBACK_VALUE
            || property == PropertyIdentifier::ACTIVE_TEXT
            || property == PropertyIdentifier::INACTIVE_TEXT
            || property == PropertyIdentifier::RELIABILITY
            || property == PropertyIdentifier::EVENT_DETECTION_ENABLE
    }
}

#[cfg(test)]
#[path = "tests/command_failure.rs"]
mod command_failure_tests;
