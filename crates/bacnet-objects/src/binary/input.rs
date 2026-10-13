use super::*;
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};
use std::sync::Arc;

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
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ENABLE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::TIME_DELAY,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingOptional),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
        PropertyWriteCapability::ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        PropertyConformance::Optional,
        Some(PropertyPresenceCondition::IntrinsicReportingOptional),
        PropertyWriteCapability::ReadOnly,
    ),
    // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
    crate::event::options::REPORTING_OPTION_METADATA[0],
    crate::event::options::REPORTING_OPTION_METADATA[1],
    crate::event::options::REPORTING_OPTION_METADATA[2],
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
        PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
        PropertyConformance::Optional,
        None,
        PropertyWriteCapability::Always,
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
        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
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
    profile: ProfileState,
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    active_text: String,
    inactive_text: String,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.6). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
}

impl BinaryInputObject {
    /// Create a new Binary Input object; fails if `instance` exceeds the object-identifier range.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, instance)?;
        Ok(Self {
            profile: ProfileState::default(),
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            polarity: 0,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
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

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name)?;
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

    /// Mutate logical `Present_Value` on an unattached or application-owned object.
    ///
    /// The value is BACnet INACTIVE (`0`) or ACTIVE (`1`) **after Polarity**,
    /// not a raw electrical or physical state. This low-level helper bypasses
    /// running-server `Out_Of_Service` ownership, validation, intrinsic-event
    /// processing, and COV processing. Applications updating a live object
    /// should use `BACnetServer::set_present_value_local`.
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
    }

    /// Validate and store a `Present_Value` write, without any access check.
    ///
    /// Shared by the network and internal routes, which differ only in the
    /// `Out_Of_Service` condition each requires.
    fn apply_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Enumerated(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if v > 1 {
            return Err(common::value_out_of_range_error());
        }
        self.present_value = v;
        Ok(())
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

    fn durable_writes_internal(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.profile.capability()
    }

    fn advance_monotonic_time_internal(&mut self, now: std::time::Duration) -> bool {
        self.profile.expire(now);
        false
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(BINARY_INPUT_PROPERTY_METADATA);
        if self.profile.metadata().next().is_some() {
            rows.to_mut().extend(self.profile.metadata());
        }
        rows
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
            return result;
        }
        if let Some(result) = self.reliability_inhibit.write_out_of_service(
            &mut self.out_of_service,
            &mut self.reliability,
            &mut self.reliability_before_out_of_service,
            property,
            &value,
        ) {
            return result.map(|_| ());
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
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let metadata = self.property_metadata();
        crate::property_metadata::property_list_from_metadata(metadata.as_ref())
    }

    fn is_createable(&self) -> bool {
        true
    }

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        if self.out_of_service || self.reliability_inhibit.enabled() {
            return Err(common::write_access_denied_error());
        }
        if !common::is_reliability_value_valid(reliability) {
            return Err(common::value_out_of_range_error());
        }
        self.reliability = reliability;
        Ok(())
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        // Local safe-ownership policy: preserve the client's OOS simulation.
        if self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.apply_present_value(value)
    }

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
    }
}

#[cfg(test)]
#[path = "tests/input_output.rs"]
mod input_output_tests;
