use super::*;
use crate::event::CommandFailureDetector;
use crate::object_profile::TagsPersistence;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// MultiStateOutput (type 14)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Output object.
///
/// Commandable multi-state output with 16-level priority array.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateOutputObject {
    pub(super) core: IoCore,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    multistate: MultiStateProperties,
    present_value: u32,
    feedback_value: u32,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
}

impl MultiStateOutputObject {
    /// Create a new Multi-state Output object; `number_of_states` must be at least 1.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let mut core = IoCore::new(ObjectType::MULTI_STATE_OUTPUT, instance, name.into())?;
        let multistate = MultiStateProperties::new(number_of_states)?;
        // A Multi-state Output starts with event detection off, as a
        // Binary Output does.
        core.event_detection_enable = false;
        Ok(Self {
            core,
            event_detector: CommandFailureDetector::default(),
            multistate,
            present_value: 1,
            feedback_value: 1,
            priority_array: [None; 16],
            relinquish_default: 1,
            value_source: crate::command_source::ValueSourceTracking::default(),
        })
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name, number_of_states)?;
        object.core.keep_tags(persistence)?;
        Ok(object)
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
        let _ = self.recompute_reliability();
    }

    /// The values the object keeps that name a state, which a new count
    /// may not strand: the commands in Priority_Array, Present_Value and
    /// Relinquish_Default. Feedback_Value is left out: it is sensed, and
    /// outside the states it is reported as CONFIGURATION_ERROR rather than
    /// refused. An Output has no Alarm_Values, so `_alarm_values` is always
    /// empty; the Input and Value take theirs the same way.
    fn held_states(&self, _alarm_values: &[u32]) -> impl Iterator<Item = u32> + '_ {
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .chain([self.present_value, self.relinquish_default])
    }

    /// None: an Output has no Alarm_Values.
    fn alarm_values(&self) -> &[u32] {
        &[]
    }

    /// Whether a command, Relinquish_Default or Feedback_Value names a
    /// state the object doesn't have.
    fn configuration_invalid(&self) -> bool {
        let is_invalid = |value: u32| !(1..=self.multistate.number_of_states).contains(&value);
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .any(is_invalid)
            || is_invalid(self.relinquish_default)
            || is_invalid(self.feedback_value)
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (Unsigned
    /// 1..=Number_Of_States); after the store, Present_Value is resolved anew
    /// from the priority array so an empty array falls back to the new
    /// default immediately.
    ///
    /// Number_Of_States shrink interplay: if the state count ever shrinks
    /// below this value, the standard leaves adjustment of Priority_Array,
    /// Relinquish_Default, Present_Value, and Feedback_Value to local policy
    /// (Clause 12.19 / Table 12-22 Number_Of_States text). This implementation does
    /// NOT auto-adjust: out-of-range stored values are a configuration
    /// decision for the application to resolve; the object-owned evaluator
    /// reports CONFIGURATION_ERROR while that retained condition remains.
    ///
    /// A restored State_Text may give the object fewer states than the
    /// application builds it with (#2414). A default set before
    /// `with_reporting_persistence` that the restored count lacks is held
    /// and shows as CONFIGURATION_ERROR; one set after it is refused, as a
    /// write of it is, since nothing saved stands in for it. Handle that
    /// refusal rather than treat it as fatal at start.
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if value < 1 || value > self.multistate.number_of_states {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }
}

crate::io_core::impl_profile!(MultiStateOutputObject);
super::impl_multistate!(MultiStateOutputObject);
crate::write_config::reporting::impl_reporting_persistence!(
    MultiStateOutputObject,
    states: multistate
);

impl BACnetObject for MultiStateOutputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.core.oid
    }

    fn object_name(&self) -> &str {
        &self.core.name
    }

    crate::traits::lend_cov_reporting!();

    crate::traits::lend_event_reporting!();

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        if let Some(result) = self
            .value_source
            .read(property, array_index, &self.priority_array)
        {
            return result;
        }
        if property == P::PRESENT_VALUE {
            return Ok(PropertyValue::Unsigned(self.present_value as u64));
        }
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(result) = self.multistate.read(property, array_index) {
            return result;
        }
        match property {
            P::FEEDBACK_VALUE => Ok(PropertyValue::Unsigned(self.feedback_value as u64)),
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: u32| PropertyValue::Unsigned(
                    v as u64
                ))
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Unsigned(self.relinquish_default as u64)),
            P::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            P::PROPERTY_LIST => {
                crate::property_metadata::read_property_list(&self.property_metadata(), array_index)
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        if property == P::VALUE_SOURCE {
            return self.value_source.correct(value, priority, origin);
        }
        if property == P::PRESENT_VALUE {
            let num_states = self.multistate.number_of_states;
            return crate::command_source::write_sourced_priority!(
                self,
                value,
                priority,
                origin,
                |v| {
                    if let PropertyValue::Unsigned(u) = v {
                        if u < 1 || u > num_states as u64 {
                            Err(common::value_out_of_range_error())
                        } else {
                            Ok(u as u32)
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if matches!(property, P::PRESENT_VALUE | P::VALUE_SOURCE) {
            return Err(common::write_access_denied_error());
        }
        if property == P::FEEDBACK_VALUE {
            if let PropertyValue::Unsigned(u) = value {
                // Checked for representability but deliberately NOT range-checked against
                // Number_Of_States, unlike Present_Value. Clause 12.19 treats a
                // Feedback_Value outside the state set as a condition to be *reported* —
                // for the Number_Of_States-bounded properties apart from Present_Value,
                // an out-of-range value pins Reliability at CONFIGURATION_ERROR until
                // fixed — not as a value to refuse. Feedback_Value reflects a sensed quantity
                // determined by local policy, so it can legitimately fall
                // outside the configured range; refusing it would make CONFIGURATION_ERROR
                // unreachable. The object-owned evaluator applies that reliability.
                //
                // The u32 conversion is still checked. A BACnet Unsigned decodes from up
                // to 8 octets, so a bare `as u32` would wrap a large value back into the
                // valid state range — silently turning a disagreeing feedback into an
                // agreeing one and suppressing the COMMAND_FAILURE transition. That is a
                // representation limit, not a configuration limit, so it is enforced here
                // while the state-set range is not.
                self.feedback_value = common::u64_to_u32(u)?;
                let _ = self.recompute_reliability();
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == P::RELINQUISH_DEFAULT {
            if let PropertyValue::Unsigned(u) = value {
                let v = common::u64_to_u32(u)?;
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.19 requires simulation/test writes while Out_Of_Service is TRUE:
        // Present_Value is writable, as is Reliability when that property exists
        // and supports values beyond NO_FAULT_DETECTED; the core takes the
        // Reliability write. `is_writable_property` stays statically true
        // because it describes capability.
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            return self.core_written(result);
        }
        if property == P::STATE_TEXT {
            return self.write_state_text(array_index, value);
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
        let mut rows = Cow::Borrowed(metadata::OUTPUT);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    crate::traits::lend_reliability_eval!();
}

impl crate::traits::CovReporting for MultiStateOutputObject {}

crate::event::impl_builtin_intrinsic_reporting!(
    MultiStateOutputObject,
    [present_value, feedback_value],
    CommandFailureDetector::ALGORITHM
);

#[cfg(test)]
#[path = "output/tests.rs"]
mod command_failure_tests;

#[cfg(test)]
mod reliability_evaluator_tests;
