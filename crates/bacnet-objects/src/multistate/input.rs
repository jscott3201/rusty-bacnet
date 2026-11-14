use super::*;
use crate::object_profile::TagsPersistence;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// MultiStateInput (type 13)
// ---------------------------------------------------------------------------

/// BACnet Multi-State Input object.
///
/// Read-only multi-state point. Present_Value is writable only when out-of-service.
/// Present_Value is Unsigned, range 1..=number_of_states.
pub struct MultiStateInputObject {
    pub(super) core: IoCore,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    multistate: MultiStateProperties,
    present_value: u32,
}

impl MultiStateInputObject {
    /// Create a new Multi-state Input object; `number_of_states` must be at least 1.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        number_of_states: u32,
    ) -> Result<Self, Error> {
        let core = IoCore::new(ObjectType::MULTI_STATE_INPUT, instance, name.into())?;
        Ok(Self {
            core,
            event_detector: ChangeOfStateDetector::default(),
            multistate: MultiStateProperties::new(number_of_states)?,
            present_value: 1,
        })
    }

    /// Set the alarm values (states that trigger OFFNORMAL).
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
        Ok(())
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

    /// Mutate `Present_Value` on an unattached or otherwise application-owned object.
    ///
    /// This low-level helper bypasses running-server `Out_Of_Service` ownership,
    /// range validation, intrinsic-event processing, and COV processing, while
    /// retaining this object's reliability recomputation. Applications updating
    /// a live object should use `BACnetServer::set_present_value_local`.
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
        let _ = self.recompute_reliability();
    }

    /// Validate and store a `Present_Value` write, without any access check.
    ///
    /// Shared by the network and internal routes, which differ only in the
    /// `Out_Of_Service` condition each requires.
    fn apply_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        let PropertyValue::Unsigned(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if v < 1 || v > self.multistate.number_of_states as u64 {
            return Err(common::value_out_of_range_error());
        }
        self.present_value = v as u32;
        let _ = self.recompute_reliability();
        Ok(())
    }

    /// The values the object keeps that name a state, which a new count
    /// may not strand: Present_Value and `alarm_values`, the Alarm_Values
    /// the count will meet.
    fn held_states<'a>(&'a self, alarm_values: &'a [u32]) -> impl Iterator<Item = u32> + 'a {
        core::iter::once(self.present_value).chain(alarm_values.iter().copied())
    }

    /// Alarm_Values, which names states too.
    fn alarm_values(&self) -> &[u32] {
        &self.event_detector.alarm_values
    }

    /// An Input holds nothing else that names a state: only Present_Value
    /// can be outside them.
    fn configuration_invalid(&self) -> bool {
        false
    }
}

crate::io_core::impl_profile!(MultiStateInputObject);
super::impl_multistate!(MultiStateInputObject);
crate::write_config::reporting::impl_reporting_persistence!(
    MultiStateInputObject,
    in core,
    states: multistate
);

impl BACnetObject for MultiStateInputObject {
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
            P::ALARM_VALUES => Ok(read_alarm_values(&self.event_detector)),
            P::PROPERTY_LIST => {
                crate::property_metadata::read_property_list(&self.property_metadata(), array_index)
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if property == P::PRESENT_VALUE {
            if !self.core.out_of_service {
                return Err(common::write_access_denied_error());
            }
            return self.apply_present_value(value);
        }
        // Clause 12.18 requires simulation/test writes while Out_Of_Service is TRUE:
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
            return write_alarm_values(
                &mut self.event_detector,
                array_index,
                value,
                self.multistate.number_of_states,
            );
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
        let mut rows = Cow::Borrowed(metadata::INPUT);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

impl crate::traits::CovReporting for MultiStateInputObject {}

crate::event::impl_builtin_intrinsic_reporting!(
    MultiStateInputObject,
    event_detector,
    core.event_history,
    [present_value],
    core.reliability,
    core.event_detection_enable,
    ChangeOfStateDetector::ALGORITHM
);

/// The Present_Value the application supplies.
impl crate::traits::LocalInput for MultiStateInputObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::PresentValue { value, .. } = input else {
            return Err(crate::traits::not_supported());
        };
        // Local safe-ownership policy: preserve the client's OOS simulation.
        if self.core.out_of_service {
            return Err(common::write_access_denied_error());
        }
        self.apply_present_value(value)
    }
}

#[cfg(test)]
mod detection_enable_tests {
    use super::*;
    use crate::event::test_clock::{test_now, test_tick};
    use crate::traits::EventReporting as _;

    #[test]
    fn msi_detection_enable_resets_and_gates_intrinsic_reporting() {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        msi.set_alarm_values(vec![2]).unwrap();
        msi.event_detector.time_delay = 2;
        msi.set_present_value(2);
        assert_eq!(msi.evaluate_intrinsic_reporting(test_now()), None);
        assert!(msi.event_detector.pending.is_some());

        msi.event_detector.event_state = bacnet_types::enums::EventState::OFFNORMAL;
        msi.event_detector.acked_transitions =
            bacnet_types::bitstring::EventTransitionBits::empty();
        msi.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        msi.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            msi.read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw())
        );
        assert_eq!(
            msi.read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
                .unwrap(),
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }
        );
        assert!(msi.event_detector.pending.is_none());
        assert!(msi.event_detector.fault_reliability.is_none());
        assert_eq!(msi.evaluate_intrinsic_reporting(test_now()), None);
        assert_eq!(msi.tick_intrinsic_reporting(test_tick()), None);
        assert!(
            msi.event_detector.pending.is_none(),
            "evaluate/tick re-armed a countdown while detection is disabled"
        );
        assert!(crate::property_metadata::has_property(
            &msi.property_metadata(),
            PropertyIdentifier::EVENT_DETECTION_ENABLE
        ));
        assert!(crate::property_metadata::is_writable_property(
            &msi,
            PropertyIdentifier::EVENT_DETECTION_ENABLE
        ));
    }
}

#[cfg(test)]
mod reliability_safety_net_tests {
    use super::*;
    use crate::traits::ReliabilityEvaluation;
    use bacnet_types::enums::Reliability;

    #[test]
    fn periodic_hook_repairs_one_stale_bypassed_state_then_is_unchanged() {
        let mut msi = MultiStateInputObject::new(1, "MSI-stale", 2).unwrap();
        msi.present_value = 3;

        assert_eq!(
            msi.reliability_eval()
                .unwrap()
                .evaluate_reliability()
                .unwrap(),
            ReliabilityEvaluation::Changed {
                old_reliability: Reliability::NO_FAULT_DETECTED,
                new_reliability: Reliability::MULTI_STATE_OUT_OF_RANGE,
            }
        );
        assert_eq!(
            msi.reliability_eval()
                .unwrap()
                .evaluate_reliability()
                .unwrap(),
            ReliabilityEvaluation::Unchanged
        );
    }
}
