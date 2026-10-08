use super::*;
use crate::common::{
    read_analog_event_properties, read_generic_event_properties, write_analog_event_properties,
    write_generic_event_properties,
};
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::property_metadata::PropertyMetadata;
use std::sync::Arc;

mod metadata;

// ---------------------------------------------------------------------------
// AnalogOutput (type 1)
// ---------------------------------------------------------------------------

/// BACnet Analog Output object.
pub struct AnalogOutputObject {
    profile: ProfileState,
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Event_Detection_Enable (Clause 12.3). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    pub(crate) event_history: EventHistory,
    /// Value source tracking.
    value_source: crate::command_source::ValueSourceTracking,
}

impl AnalogOutputObject {
    /// Create a new Analog Output object.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance)?;
        Ok(Self {
            profile: ProfileState::default(),
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
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            min_pres_value: None,
            max_pres_value: None,
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
        units: u32,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name, units)?;
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

    /// Set minimum engineering-bound metadata; this is not a reliability fault limit.
    pub fn set_min_pres_value(&mut self, value: f32) {
        self.min_pres_value = Some(value);
    }

    /// Set maximum engineering-bound metadata; this is not a reliability fault limit.
    pub fn set_max_pres_value(&mut self, value: f32) {
        self.max_pres_value = Some(value);
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

impl BACnetObject for AnalogOutputObject {
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
        if let Some(result) = self.profile.read(property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ANALOG_OUTPUT.to_raw(),
            )),
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
                    if let PropertyValue::Real(f) = v {
                        if !f.is_finite() {
                            return Err(common::value_out_of_range_error());
                        }
                        Ok(f)
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
        // Clause 12.3 requires simulation/test writes while Out_Of_Service is TRUE:
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
        if let Some(result) = write_analog_event_properties!(self, property, value) {
            return result;
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

    fn supports_cov(&self) -> bool {
        true
    }

    fn cov_increment(&self) -> Option<f64> {
        Some(f64::from(self.cov_increment))
    }

    crate::event::impl_builtin_intrinsic_reporting!(
        event_detector,
        event_history,
        [present_value],
        reliability,
        event_detection_enable,
        OutOfRangeDetector::ALGORITHM
    );

    fn acknowledge_alarm(
        &mut self,
        transition_bit: bacnet_types::bitstring::EventTransitionBits,
    ) -> Result<(), bacnet_types::error::Error> {
        self.event_detector.acked_transitions |=
            transition_bit & bacnet_types::bitstring::EventTransitionBits::all();
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

    fn is_createable(&self) -> bool {
        true
    }

    fn creation_only_properties(&self) -> &'static [PropertyIdentifier] {
        super::CREATION_ONLY
    }

    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        super::initialize_units(&mut self.units, property, value)
    }
}

#[cfg(test)]
mod detection_enable_reset_tests {
    use super::*;

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn ao_disabling_detection_resets_event_history() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        assert_eq!(
            ao.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        ao.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ao.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
        ao.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            remaining: 2,
        });
        ao.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        ao.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        ao.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        ao.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            ao.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            ao.event_detector.event_state,
            bacnet_types::enums::EventState::NORMAL
        );
        assert_eq!(
            ao.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::all()
        );
        assert!(ao.event_detector.pending.is_none());
        assert!(ao.event_detector.fault_reliability.is_none());
        assert_eq!(
            ao.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            ao.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }
}
