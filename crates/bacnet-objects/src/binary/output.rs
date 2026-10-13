use super::*;
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::property_metadata::PropertyMetadata;
use std::sync::Arc;

#[path = "output/metadata.rs"]
mod metadata;

// ---------------------------------------------------------------------------
// BinaryOutput (type 4)
// ---------------------------------------------------------------------------

/// BACnet Binary Output object.
///
/// Commandable binary output with 16-level priority array.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryOutputObject {
    profile: ProfileState,
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
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    event_detection_enable: bool,
    active_text: String,
    inactive_text: String,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    pub(crate) event_history: EventHistory,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
}

impl BinaryOutputObject {
    /// Create a new Binary Output object; fails if `instance` exceeds the object-identifier range.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance)?;
        Ok(Self {
            profile: ProfileState::default(),
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
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            event_detection_enable: false,
            active_text: "Active".into(),
            inactive_text: "Inactive".into(),
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
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name)?;
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
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (BinaryPV 0 or 1);
    /// after the store, Present_Value is resolved anew from the priority
    /// array so an empty array falls back to the new default immediately.
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if value > 1 {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
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
            return crate::command_source::write_sourced_priority!(
                self,
                value,
                priority,
                origin,
                |v| {
                    if let PropertyValue::Enumerated(e) = v {
                        if e > 1 {
                            Err(common::value_out_of_range_error())
                        } else {
                            Ok(e)
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
        if property == PropertyIdentifier::RELINQUISH_DEFAULT {
            if let PropertyValue::Enumerated(e) = value {
                return self.set_relinquish_default(e);
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
        // Clause 12.7 requires simulation/test writes while Out_Of_Service is TRUE:
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

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
    }
}

#[cfg(test)]
#[path = "tests/command_failure.rs"]
mod command_failure_tests;
