use super::*;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;

#[path = "value/metadata.rs"]
mod metadata;

// ---------------------------------------------------------------------------
// BinaryValue (type 5)
// ---------------------------------------------------------------------------

/// BACnet Binary Value object.
///
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryValueObject {
    audit_policy: crate::audit::ObjectAuditPolicy,
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // 0 = inactive, 1 = active
    out_of_service: bool,
    status_flags: StatusFlags,
    access: PresentValueAccess,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Reliability; NO_FAULT_DETECTED until a fault is evaluated or simulated.
    reliability: Reliability,
    reliability_before_out_of_service: Option<Reliability>,
    reliability_inhibit: common::ReliabilityInhibitState,
    active_text: String,
    inactive_text: String,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    /// Event_Detection_Enable (Clause 12.8). A FALSE value suspends
    /// event-state-machine evaluation under Clause 13.2.2.1.
    event_detection_enable: bool,
    pub(crate) event_history: EventHistory,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
    /// The last writer of a noncommandable Present_Value, once tracked (#1552).
    write_source: crate::command_source::SingleValueSource,
}

impl BinaryValueObject {
    /// Provision independently optional Audit properties before registration.
    /// Raw object configuration bypasses server notification ownership; use
    /// BACnetServer::write_local or WP/WPM for live property mutations.
    pub fn set_audit_policy(&mut self, policy: crate::audit::ObjectAuditPolicy) {
        self.audit_policy = policy;
    }

    /// Create a new Binary Value object with a commandable Present_Value.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Self::with_access(instance, name, PresentValueAccess::Commandable)
    }

    /// Create a new Binary Value object whose Present_Value is written as `access` says.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        access: PresentValueAccess,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance)?;
        Ok(Self {
            audit_policy: crate::audit::ObjectAuditPolicy::default(),
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // inactive
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            access,
            priority_array: [None; 16],
            relinquish_default: 0,
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
            value_source: crate::command_source::ValueSourceTracking::default(),
            write_source: crate::command_source::SingleValueSource::default(),
        })
    }

    /// Track the source of a noncommandable Present_Value (Clause 19.5,
    /// #1552), as [`AnalogValueObject::set_value_source_tracking`] does.
    ///
    /// [`AnalogValueObject::set_value_source_tracking`]: crate::analog::AnalogValueObject::set_value_source_tracking
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.write_source.set_enabled(enabled);
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
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        if value > 1 {
            return Err(common::value_out_of_range_error());
        }
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

    fn checked_present_value(value: PropertyValue) -> Result<u32, Error> {
        let PropertyValue::Enumerated(e) = value else {
            return Err(common::invalid_data_type_error());
        };
        if e > 1 {
            return Err(common::value_out_of_range_error());
        }
        Ok(e)
    }
}

impl BACnetObject for BinaryValueObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
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
        // Clause 12.8 requires simulation/test writes while Out_Of_Service is TRUE:
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
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
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

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.reliability_inhibit.enabled()
    }
}

#[cfg(test)]
#[path = "tests/value.rs"]
mod value_tests;
