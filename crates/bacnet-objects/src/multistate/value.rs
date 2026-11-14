use super::*;
use crate::object_profile::TagsPersistence;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// MultiStateValue (type 19)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Value object.
///
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateValueObject {
    pub(super) core: IoCore,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    multistate: MultiStateProperties,
    present_value: u32,
    access: PresentValueAccess,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
    /// The last writer of a noncommandable Present_Value, once tracked (#1552).
    write_source: crate::command_source::SingleValueSource,
}

impl MultiStateValueObject {
    /// Create a new Multi-State Value object with a commandable Present_Value.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        Self::with_access(
            instance,
            name,
            number_of_states,
            PresentValueAccess::Commandable,
        )
    }

    /// Create a new Multi-State Value object whose Present_Value is written as `access` says.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
        access: PresentValueAccess,
    ) -> Result<Self, Error> {
        let core = IoCore::new(ObjectType::MULTI_STATE_VALUE, instance, name.into())?;
        Ok(Self {
            core,
            event_detector: ChangeOfStateDetector::default(),
            multistate: MultiStateProperties::new(number_of_states)?,
            present_value: 1,
            access,
            priority_array: [None; 16],
            relinquish_default: 1,
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
        number_of_states: u32,
        access: PresentValueAccess,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::with_access(instance, name, number_of_states, access)?;
        object.core.keep_tags(persistence)?;
        Ok(object)
    }

    /// Track the source of a noncommandable Present_Value (Clause 19.5,
    /// #1552), as [`AnalogValueObject::set_value_source_tracking`] does.
    ///
    /// [`AnalogValueObject::set_value_source_tracking`]: crate::analog::AnalogValueObject::set_value_source_tracking
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.write_source.set_enabled(enabled);
    }

    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
        let _ = self.recompute_reliability();
    }

    /// Whether a command, Relinquish_Default or an Alarm_Values entry names
    /// a state the object doesn't have.
    fn configuration_invalid(&self) -> bool {
        let is_invalid = |value: u32| !(1..=self.multistate.number_of_states).contains(&value);
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .any(is_invalid)
            || is_invalid(self.relinquish_default)
            || self
                .event_detector
                .alarm_values
                .iter()
                .copied()
                .any(is_invalid)
    }

    /// Set the alarm values and synchronously re-evaluate configuration Reliability.
    ///
    /// The list is checked as a network write of it is (#2415): more than
    /// 1,024 entries is RESOURCES / NO_SPACE_TO_WRITE_PROPERTY, and a zero
    /// or a state past Number_Of_States is VALUE_OUT_OF_RANGE, each naming
    /// its entry; the list held stays. On an object keeping its reporting
    /// settings, Alarm_Values a write saved win, until
    /// `clear_written_reporting` (#2331): the setter then refuses only what
    /// is wrong whatever the count (a zero, an over-long list) and returns
    /// `Ok`, leaving the saved list. Configure the list before
    /// `with_reporting_persistence` where you can. After it, a restored
    /// State_Text may have given the object fewer states than the list
    /// names (#2414), and with no list saved that refusal is the answer:
    /// handle it rather than treat it as fatal at start.
    pub fn set_alarm_values(&mut self, values: Vec<u32>) -> Result<(), Error> {
        #[cfg(feature = "event-reporting")]
        if self.kept_over_setter(PropertyIdentifier::ALARM_VALUES) {
            // The saved list wins, so the count it would meet doesn't matter.
            return check_alarm_values(&values, u32::MAX);
        }
        check_alarm_values(&values, self.multistate.number_of_states)?;
        self.event_detector.alarm_values = values;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (Unsigned
    /// 1..=Number_Of_States); after the store, Present_Value is resolved anew
    /// from the priority array so an empty array falls back to the new
    /// default immediately.
    ///
    /// Number_Of_States shrink interplay: if the state count ever shrinks
    /// below this value, retained Priority_Array, Relinquish_Default,
    /// Present_Value, and Alarm_Values are not auto-adjusted (Clause 12.20 /
    /// Table 12-23). An out-of-range retained value is a configuration decision
    /// for the application to resolve; the object-owned evaluator reports
    /// CONFIGURATION_ERROR while that condition remains.
    ///
    /// A restored State_Text may give the object fewer states than the
    /// application builds it with (#2414). A default set before
    /// `with_reporting_persistence` that the restored count lacks is held
    /// and shows as CONFIGURATION_ERROR; one set after it is refused, as a
    /// write of it is, since nothing saved stands in for it. Handle that
    /// refusal rather than treat it as fatal at start.
    pub fn set_relinquish_default(&mut self, value: u32) -> Result<(), Error> {
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        if value < 1 || value > self.multistate.number_of_states {
            return Err(common::value_out_of_range_error());
        }
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    fn checked_present_value(number_of_states: u32, value: PropertyValue) -> Result<u32, Error> {
        let PropertyValue::Unsigned(u) = value else {
            return Err(common::invalid_data_type_error());
        };
        if u < 1 || u > u64::from(number_of_states) {
            return Err(common::value_out_of_range_error());
        }
        Ok(u as u32)
    }

    /// The values the object keeps that name a state, which a new count
    /// may not strand: the commands in Priority_Array, Present_Value,
    /// Relinquish_Default and `alarm_values`, the Alarm_Values the count
    /// will meet.
    fn held_states<'a>(&'a self, alarm_values: &'a [u32]) -> impl Iterator<Item = u32> + 'a {
        self.priority_array
            .iter()
            .flatten()
            .copied()
            .chain([self.present_value, self.relinquish_default])
            .chain(alarm_values.iter().copied())
    }

    /// Alarm_Values, which names states too.
    fn alarm_values(&self) -> &[u32] {
        &self.event_detector.alarm_values
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
        if self.access == PresentValueAccess::ReadOnly && !self.core.out_of_service {
            return Err(common::write_access_denied_error());
        }
        // Clause 19.2: an otherwise permitted noncommandable NULL is a no-op.
        if value == PropertyValue::Null {
            return Ok(());
        }
        self.set_present_value_directly(value)
    }

    fn set_present_value_directly(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.present_value = Self::checked_present_value(self.multistate.number_of_states, value)?;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// The rows before the access projects them: Table 12-23's, then the
    /// profile's.
    fn rows(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(metadata::VALUE);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    /// Whether `property` is absent under the object's Present_Value access.
    fn excludes(&self, property: PropertyIdentifier) -> bool {
        self.access != PresentValueAccess::Commandable
            && self
                .access
                .excludes(&self.rows(), property, self.write_source.is_enabled())
    }
}

crate::io_core::impl_profile!(MultiStateValueObject);
super::impl_multistate!(MultiStateValueObject);
crate::write_config::reporting::impl_reporting_persistence!(
    MultiStateValueObject,
    states: multistate
);

impl BACnetObject for MultiStateValueObject {
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
        if self.excludes(property) {
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
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, |v: u32| PropertyValue::Unsigned(
                    v as u64
                ))
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Unsigned(self.relinquish_default as u64)),
            P::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            P::ALARM_VALUES => Ok(read_alarm_values(&self.event_detector)),
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
            return match self.access {
                PresentValueAccess::Commandable => {
                    self.value_source.correct(value, priority, origin)
                }
                _ => self.write_source.correct(array_index, value, origin),
            };
        }
        if property == P::PRESENT_VALUE {
            let number_of_states = self.multistate.number_of_states;
            return match self.access {
                PresentValueAccess::Commandable => {
                    crate::command_source::write_sourced_priority!(
                        self,
                        value,
                        priority,
                        origin,
                        |v| Self::checked_present_value(number_of_states, v)
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
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        if property == P::PRESENT_VALUE {
            if self.access == PresentValueAccess::Commandable {
                return Err(common::write_access_denied_error());
            }
            // A tracked source needs the writer (`write_property_from`).
            self.write_source.unsourced()?;
            return self.write_direct_present_value(array_index, value);
        }
        if property == P::VALUE_SOURCE {
            return Err(common::write_access_denied_error());
        }
        if property == P::RELINQUISH_DEFAULT {
            if let PropertyValue::Unsigned(u) = value {
                let v = common::u64_to_u32(u)?;
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.20 requires simulation/test writes while Out_Of_Service is TRUE:
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
        if property == P::ALARM_VALUES {
            write_alarm_values(
                &mut self.event_detector,
                array_index,
                value,
                self.multistate.number_of_states,
            )?;
            let _ = self.recompute_reliability();
            return Ok(());
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
        self.access
            .project(self.rows(), self.write_source.is_enabled())
    }

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

impl crate::traits::CovReporting for MultiStateValueObject {}

crate::event::impl_builtin_intrinsic_reporting!(
    MultiStateValueObject,
    [present_value],
    ChangeOfStateDetector::ALGORITHM
);

/// The Present_Value the application supplies while it isn't commandable,
/// published as Value_Source from `origin` when the object tracks its
/// source (#1552).
impl crate::traits::LocalInput for MultiStateValueObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::PresentValue { value, origin } = input else {
            return Err(crate::traits::not_supported());
        };
        let Some(origin) = origin else {
            return self.apply_local_present_value(value);
        };
        self.write_source.admit(&origin)?;
        self.apply_local_present_value(value)?;
        self.write_source.record(&origin);
        Ok(())
    }
}

impl MultiStateValueObject {
    /// The application's Present_Value, refused while it's commandable or
    /// Out_Of_Service is TRUE.
    fn apply_local_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        match self.access {
            PresentValueAccess::Commandable => Err(crate::traits::not_supported()),
            _ if self.core.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.set_present_value_directly(value)?;
                self.write_source.forget();
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod detection_enable_tests {
    use super::*;
    use crate::event::test_clock::{test_now, test_tick};
    use crate::traits::EventReporting as _;

    #[test]
    fn msv_detection_enable_resets_and_gates_intrinsic_reporting() {
        let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        msv.event_detector.alarm_values = vec![2];
        msv.event_detector.time_delay = 2;
        msv.write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
        assert_eq!(msv.evaluate_intrinsic_reporting(test_now()), None);
        assert!(msv.event_detector.pending.is_some());

        msv.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
        msv.event_detector.acked_transitions =
            bacnet_types::bitstring::EventTransitionBits::empty();
        msv.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        msv.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            msv.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw())
        );
        assert_eq!(
            msv.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }
        );
        assert!(msv.event_detector.pending.is_none());
        assert!(msv.event_detector.fault_reliability.is_none());
        assert_eq!(msv.evaluate_intrinsic_reporting(test_now()), None);
        assert_eq!(msv.tick_intrinsic_reporting(test_tick()), None);
        assert!(
            msv.event_detector.pending.is_none(),
            "evaluate/tick re-armed a countdown while detection is disabled"
        );
        assert!(crate::property_metadata::has_property(
            &msv.property_metadata(),
            PropertyIdentifier::EVENT_DETECTION_ENABLE
        ));
        assert!(crate::property_metadata::is_writable_property(
            &msv,
            PropertyIdentifier::EVENT_DETECTION_ENABLE
        ));
    }
}

#[cfg(test)]
mod reliability_evaluator_tests {
    use super::*;
    use bacnet_types::enums::Reliability;

    #[test]
    fn configuration_error_dominates_bypassed_invalid_present_value() {
        let mut msv = MultiStateValueObject::new(1, "MSV-dominance", 2).unwrap();
        msv.present_value = 3;
        msv.event_detector.alarm_values = vec![3];

        msv.reliability_eval()
            .unwrap()
            .evaluate_reliability()
            .unwrap();
        assert_eq!(
            msv.core.reliability,
            Reliability::CONFIGURATION_ERROR,
            "invalid configuration must dominate invalid Present_Value"
        );
        msv.event_detector.alarm_values = vec![1];
        msv.reliability_eval()
            .unwrap()
            .evaluate_reliability()
            .unwrap();
        assert_eq!(msv.core.reliability, Reliability::MULTI_STATE_OUT_OF_RANGE);
        msv.set_number_of_states(3).unwrap();
        assert_eq!(
            msv.core.reliability,
            Reliability::NO_FAULT_DETECTED,
            "count growth must synchronously recover the retained Present_Value"
        );
    }
}
