use super::*;
use crate::common::{
    read_analog_event_properties, read_generic_event_properties, write_analog_event_properties,
    write_generic_event_properties,
};
use crate::object_profile::{ObjectProfile, ProfileState, TagsPersistence};
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;
use std::sync::Arc;

mod metadata;
#[cfg(test)]
mod nonfinite_tests;

// ---------------------------------------------------------------------------
// AnalogValue (type 2)
// ---------------------------------------------------------------------------

/// BACnet Analog Value object.
pub struct AnalogValueObject {
    profile: ProfileState,
    audit_policy: crate::audit::ObjectAuditPolicy,
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    units: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    access: PresentValueAccess,
    /// 16-level priority array. `None` = no command at that level.
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// COV_Increment: minimum change threshold for COV notifications.
    /// Default 0.0 means notify on any write (including no-change).
    /// Set to a positive value for delta-based filtering.
    cov_increment: f32,
    event_detector: OutOfRangeDetector,
    /// Event_Detection_Enable (Clause 12.4). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    fault_out_of_range: FaultOutOfRangeState,
    min_pres_value: Option<f32>,
    max_pres_value: Option<f32>,
    pub(crate) event_history: EventHistory,
    /// Value source tracking.
    value_source: crate::command_source::ValueSourceTracking,
    /// The last writer of a noncommandable Present_Value, once tracked (#1552).
    write_source: crate::command_source::SingleValueSource,
}

impl AnalogValueObject {
    /// Provision independently optional Audit properties before registration.
    /// Raw object configuration bypasses server notification ownership; use
    /// BACnetServer::write_local or WP/WPM for live property mutations.
    pub fn set_audit_policy(&mut self, policy: crate::audit::ObjectAuditPolicy) {
        self.audit_policy = policy;
    }

    /// Create a new Analog Value object with a commandable Present_Value.
    pub fn new(instance: u32, name: impl Into<String>, units: u32) -> Result<Self, Error> {
        Self::with_access(instance, name, units, PresentValueAccess::Commandable)
    }

    /// Create a new Analog Value object whose Present_Value is written as `access` says.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        units: u32,
        access: PresentValueAccess,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance)?;
        Ok(Self {
            profile: ProfileState::default(),
            audit_policy: crate::audit::ObjectAuditPolicy::default(),
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            units,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            access,
            priority_array: [None; 16],
            relinquish_default: 0.0,
            cov_increment: 0.0,
            event_detector: OutOfRangeDetector::default(),
            event_detection_enable: true,
            reliability: Reliability::NO_FAULT_DETECTED,
            reliability_before_out_of_service: None,
            reliability_inhibit: common::ReliabilityInhibitState::default(),
            fault_out_of_range: FaultOutOfRangeState::default(),
            min_pres_value: None,
            max_pres_value: None,
            event_history: EventHistory::default(),
            value_source: crate::command_source::ValueSourceTracking::default(),
            write_source: crate::command_source::SingleValueSource::default(),
        })
    }

    /// Build with application-owned Tags storage and the requested Present_Value access.
    /// Saved Tags override configured Tags only while [`Self::set_profile`]
    /// provisions the row. Loaded data must satisfy the shared Tags rules and
    /// the opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        units: u32,
        access: PresentValueAccess,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::with_access(instance, name, units, access)?;
        object.profile = ProfileState::persistent(object.oid, persistence)?;
        Ok(object)
    }

    /// Provision independent optional Tags, Profile_Location and Profile_Name
    /// rows before registration, in every Present_Value access mode. Tags is
    /// writable; the text rows are network read-only. Invalid configuration
    /// leaves the previous profile and its attached storage unchanged.
    pub fn set_profile(&mut self, profile: ObjectProfile) -> Result<(), Error> {
        self.profile.provision(profile)
    }

    /// Wait for queued save attempts to finish; writes report their own outcomes.
    /// This is not a success receipt. Unstaged synchronous writes block their caller.
    pub fn wait_for_tag_saves(&self) {
        self.profile.wait_for_saves();
    }

    /// Track the source of a noncommandable Present_Value (Clause 19.5,
    /// #1552), provisioned before registration like the audit policy.
    ///
    /// On, the object serves Value_Source: the last writer, network or
    /// local, which alone may then correct it. A write must name its writer
    /// (`write_property_from`), and an application update through
    /// `set_present_value_internal` leaves no source (NONE). A commandable
    /// object always tracks its sources through Priority_Array, so this
    /// changes nothing on one.
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.write_source.set_enabled(enabled);
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
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        common::reject_non_finite(value)?;
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    /// A noncommandable Present_Value write, network-equivalent.
    fn write_direct_present_value(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        if array_index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        if self.access == PresentValueAccess::ReadOnly && !self.out_of_service {
            return Err(common::write_access_denied_error());
        }
        // Clause 19.2: an otherwise permitted noncommandable NULL is a no-op.
        if value == PropertyValue::Null {
            return Ok(());
        }
        self.present_value = Self::checked_present_value(value)?;
        Ok(())
    }

    fn checked_present_value(value: PropertyValue) -> Result<f32, Error> {
        let PropertyValue::Real(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if !v.is_finite() {
            return Err(common::value_out_of_range_error());
        }
        Ok(v)
    }
}

impl BACnetObject for AnalogValueObject {
    fn audit_policy_authority_internal(
        &mut self,
    ) -> Option<crate::audit::AuditPolicyAuthority<'_>> {
        Some(crate::audit::AuditPolicyAuthority::new(
            &mut self.audit_policy,
        ))
    }

    fn audit_object_policy_internal(&self) -> crate::audit::ObjectAuditPolicy {
        self.audit_policy
    }

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
        if metadata::excludes(self, property) {
            return Err(common::unknown_property_error());
        }
        let source = match self.access {
            PresentValueAccess::Commandable => {
                self.value_source
                    .read(property, array_index, &self.priority_array)
            }
            _ => self.write_source.read(property, array_index),
        };
        if let Some(result) = source {
            return result;
        }
        if let Some(result) = self.profile.read(property, array_index) {
            return result;
        }

        if let Some(result) = self.audit_policy.read(property, array_index) {
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
            return match self.access {
                PresentValueAccess::Commandable => {
                    self.value_source.correct(value, priority, origin)
                }
                _ => self.write_source.correct(array_index, value, origin),
            };
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            return match self.access {
                PresentValueAccess::Commandable => {
                    crate::command_source::write_sourced_priority!(
                        self,
                        value,
                        priority,
                        origin,
                        Self::checked_present_value
                    )
                }
                PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                    self.write_source.admit(origin)?;
                    // A NULL is a no-op (Clause 19.2), so it has no writer.
                    let null = value == PropertyValue::Null;
                    self.write_direct_present_value(array_index, value)?;
                    if !null {
                        self.write_source.record(origin);
                    }
                    Ok(())
                }
            };
        }
        self.write_property(property, array_index, value, priority)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if metadata::excludes(self, property) {
            return Err(common::unknown_property_error());
        }
        if let Some(result) = self.profile.write(property, array_index, &value) {
            return result;
        }
        if let Some(result) = self
            .audit_policy
            .write(property, array_index, &value, priority)
        {
            return result;
        }

        if property == PropertyIdentifier::PRESENT_VALUE {
            if self.access == PresentValueAccess::Commandable {
                return Err(common::write_access_denied_error());
            }
            // A tracked source needs the writer (`write_property_from`).
            self.write_source.unsourced()?;
            return self.write_direct_present_value(array_index, value);
        }
        if property == PropertyIdentifier::VALUE_SOURCE {
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
        // Clause 12.4 requires simulation/test writes while Out_Of_Service is TRUE:
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
        self.fault_out_of_range.clear_ownership();
        Ok(())
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        match self.access {
            PresentValueAccess::Commandable => {
                Err(common::optional_functionality_not_supported_error())
            }
            _ if self.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.present_value = Self::checked_present_value(value)?;
                self.write_source.forget();
                Ok(())
            }
        }
    }

    fn set_present_value_from_internal(
        &mut self,
        value: PropertyValue,
        origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.write_source.admit(origin)?;
        self.set_present_value_internal(value)?;
        self.write_source.record(origin);
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
        av.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
        av.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            remaining: 2,
        });
        av.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
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
        assert_eq!(
            av.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::all()
        );
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
            assert_eq!(normal.reliability, Reliability::NO_FAULT_DETECTED);
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
                    old_reliability: Reliability::NO_FAULT_DETECTED,
                    new_reliability: Reliability::UNDER_RANGE,
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
            assert_eq!(owned.reliability, Reliability::UNDER_RANGE);
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
                    old_reliability: Reliability::UNDER_RANGE,
                    new_reliability: Reliability::OVER_RANGE,
                }
            );
        }
    }
}
