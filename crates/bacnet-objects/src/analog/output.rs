use super::*;
use crate::object_profile::TagsPersistence;
use crate::prelude::*;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// AnalogOutput (type 1)
// ---------------------------------------------------------------------------

/// BACnet Analog Output object.
pub struct AnalogOutputObject {
    pub(super) core: IoCore,
    event_detector: OutOfRangeDetector,
    analog: AnalogProperties,
    present_value: f32,
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// Value source tracking.
    value_source: crate::command_source::ValueSourceTracking,
}

impl AnalogOutputObject {
    /// Create a new Analog Output object.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
    ) -> Result<Self, Error> {
        Ok(Self {
            core: IoCore::new(ObjectType::ANALOG_OUTPUT, instance, name.into())?,
            event_detector: OutOfRangeDetector::default(),
            analog: AnalogProperties::new(units),
            present_value: 0.0,
            priority_array: [None; 16],
            relinquish_default: 0.0,
            value_source: crate::command_source::ValueSourceTracking::default(),
        })
    }

    /// Build with application-owned Tags storage. Saved Tags override configured
    /// Tags only while [`Self::set_profile`] provisions the row. Loaded data must
    /// satisfy the shared Tags rules and opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::new(instance, name, units)?;
        object.core.keep_tags(persistence)?;
        Ok(object)
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

crate::io_core::impl_profile!(AnalogOutputObject);
super::impl_analog_properties!(AnalogOutputObject);
crate::write_config::reporting::impl_reporting_persistence!(AnalogOutputObject, in core);

impl BACnetObject for AnalogOutputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.core.oid
    }

    fn object_name(&self) -> &str {
        &self.core.name
    }

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
            return Ok(PropertyValue::Real(self.present_value));
        }
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(result) = self.analog.read(property) {
            return result;
        }
        match property {
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Real)
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Real(self.relinquish_default)),
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if matches!(property, P::PRESENT_VALUE | P::VALUE_SOURCE) {
            return Err(common::write_access_denied_error());
        }
        if property == P::RELINQUISH_DEFAULT {
            if let PropertyValue::Real(v) = value {
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.3 requires simulation/test writes while Out_Of_Service is TRUE:
        // Present_Value is writable, as is Reliability when that property exists
        // and supports values beyond NO_FAULT_DETECTED; the core takes the
        // Reliability write. `is_writable_property` stays statically true
        // because it describes capability.
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            return result.map(drop);
        }
        if let Some(result) = self.analog.write(property, &value) {
            return result;
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
        // Only independently configured engineering bounds: AO has no fault limits.
        let mut rows = Cow::Borrowed(metadata::OUTPUT);
        self.analog.add_rows(&mut rows);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    crate::traits::lend_cov_reporting!();

    crate::traits::lend_event_reporting!();

    crate::traits::lend_reliability_eval!();
}

crate::io_core::impl_reliability_eval!(AnalogOutputObject);

crate::event::impl_builtin_intrinsic_reporting!(
    AnalogOutputObject,
    event_detector,
    core.event_history,
    [present_value],
    core.reliability,
    core.event_detection_enable,
    OutOfRangeDetector::ALGORITHM
);

#[cfg(test)]
mod detection_enable_reset_tests {
    use super::*;

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn ao_disabling_detection_resets_event_history() {
        let mut ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        assert_eq!(
            ao.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        ao.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        ao.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
        ao.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            due: crate::event::test_clock::in_seconds(2),
        });
        ao.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        ao.core.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        ao.core.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

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
            ao.core.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            ao.core.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }
}
