use super::*;
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};

const BINARY_INPUT_PROPERTY_METADATA: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_NAME,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::DESCRIPTION,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::WhenOutOfService,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_STATE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ENABLE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::TIME_DELAY,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::POLARITY,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::WhenOutOfService,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::PairedText),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::INACTIVE_TEXT,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::PairedText),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::ALARM_VALUE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReporting),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        PropertyConformance::RequiredRead,
        None,
        PropertyWriteCapability::ReadOnly,
    ),
];

// ---------------------------------------------------------------------------
// BinaryInput (type 3)
// ---------------------------------------------------------------------------

/// BACnet Binary Input object.
///
/// Read-only binary point. Present_Value is writable only when out-of-service.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    reliability_before_out_of_service: Option<u32>,
    active_text: String,
    inactive_text: String,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.6). Clause 13.2.2.1: "If the
    /// Event_Detection_Enable property is FALSE, then this state machine is not evaluated."
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
}

impl BinaryInputObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            polarity: 0,
            reliability: 0,
            reliability_before_out_of_service: None,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
            event_detector: ChangeOfStateDetector {
                alarm_values: vec![1],
                ..Default::default()
            },
            event_detection_enable: true,
            event_history: EventHistory::default(),
        })
    }

    /// Set the present value (used by application to update input state).
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for BinaryInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        Cow::Borrowed(BINARY_INPUT_PROPERTY_METADATA)
    }

    fn supports_cov(&self) -> bool {
        true
    }

    crate::impl_intrinsic_reporting!(
        event_detector,
        present_value,
        reliability,
        event_detection_enable
    );
    impl_intrinsic_write_rollback!(event_detector, event_detection_enable, event_history);

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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_INPUT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::POLARITY => Ok(PropertyValue::Enumerated(self.polarity)),
            p if p == PropertyIdentifier::ACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.active_text.clone()))
            }
            p if p == PropertyIdentifier::INACTIVE_TEXT => {
                Ok(PropertyValue::CharacterString(self.inactive_text.clone()))
            }
            p if p == PropertyIdentifier::ALARM_VALUE => {
                // Construction and writes keep exactly one value; ACTIVE is the
                // defensive construction default if a future reset empties it.
                Ok(PropertyValue::Enumerated(
                    self.event_detector
                        .alarm_values
                        .first()
                        .copied()
                        .unwrap_or(1),
                ))
            }
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
        if property == PropertyIdentifier::PRESENT_VALUE {
            if !self.out_of_service {
                return Err(common::write_access_denied_error());
            }
            if let PropertyValue::Enumerated(v) = value {
                if v > 1 {
                    return Err(common::value_out_of_range_error());
                }
                self.present_value = v;
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
        if property == PropertyIdentifier::ALARM_VALUE {
            if let PropertyValue::Enumerated(v) = value {
                if v > 1 {
                    return Err(common::value_out_of_range_error());
                }
                self.event_detector.alarm_values = vec![v];
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
        let metadata = self.property_metadata();
        crate::property_metadata::property_list_from_metadata(metadata.as_ref())
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
}

#[cfg(test)]
#[path = "tests/input_output.rs"]
mod input_output_tests;
