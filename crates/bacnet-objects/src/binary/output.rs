use super::*;
use crate::object_profile::TagsPersistence;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// BinaryOutput (type 4)
// ---------------------------------------------------------------------------

/// BACnet Binary Output object.
///
/// Commandable binary output with 16-level priority array.
/// Uses Enumerated values: 0 = inactive, 1 = active.
///
/// The application drives its physical output to
/// [`Self::physical_output`], the commanded Present_Value through Polarity
/// (#2580). Feedback_Value stays logical: the application reports the
/// controlled device's state as an INACTIVE or ACTIVE Present_Value would
/// name it.
pub struct BinaryOutputObject {
    pub(super) core: IoCore,
    /// COMMAND_FAILURE event detector.
    event_detector: CommandFailureDetector,
    binary: BinaryProperties,
    present_value: u32,
    feedback_value: u32,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
    polarity: Polarity,
    /// The physical output's state, ON as `true`, as it was driven when
    /// Out_Of_Service last became TRUE: what [`Self::physical_output`]
    /// gives while it stays TRUE.
    held_output: bool,
    /// Implemented paired command-source tracking (Clause 19.5).
    value_source: crate::command_source::ValueSourceTracking,
}

impl BinaryOutputObject {
    /// Create a new Binary Output object; fails if `instance` exceeds the object-identifier range.
    ///
    /// Polarity starts NORMAL, and Present_Value INACTIVE, the
    /// Relinquish_Default, so the physical output starts OFF.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let mut core = IoCore::new(ObjectType::BINARY_OUTPUT, instance, name.into())?;
        // A Binary Output starts with event detection off (#216).
        core.event_detection_enable = false;
        Ok(Self {
            core,
            event_detector: CommandFailureDetector::default(),
            binary: BinaryProperties::new(),
            present_value: 0,
            feedback_value: 0,
            priority_array: [None; 16],
            relinquish_default: 0,
            polarity: Polarity::NORMAL,
            held_output: false,
            value_source: crate::command_source::ValueSourceTracking::default(),
        })
    }

    /// The state to drive the physical output to, ON as `true` (#2580).
    ///
    /// In service it is Present_Value through Polarity: ON for ACTIVE under
    /// NORMAL, OFF for ACTIVE under REVERSE (Table 12-9). While
    /// Out_Of_Service is TRUE the output is decoupled from Present_Value
    /// (Clause 12.7.10): this stays at the state it had when Out_Of_Service
    /// became TRUE, while commands keep moving Present_Value and a Polarity
    /// change waits too. The return to service drives the output from the
    /// Present_Value and Polarity of that moment. A running server gives it
    /// through `BACnetServer::physical_output_local`.
    pub fn physical_output(&self) -> bool {
        if self.core.out_of_service {
            self.held_output
        } else {
            across_polarity(is_active(self.present_value), self.polarity)
        }
    }

    /// Set Polarity, NORMAL or REVERSE (#2580); any other value fails with
    /// PROPERTY / VALUE_OUT_OF_RANGE and changes nothing. A new object's
    /// is NORMAL, and a client can't write it.
    ///
    /// Present_Value doesn't change. In service, [`Self::physical_output`]
    /// follows at once (Clause 12.7.11); while Out_Of_Service is TRUE the
    /// output stays as it is, and the return to service drives it with the
    /// new Polarity. On a running server use
    /// `BACnetServer::set_polarity_local`.
    pub fn set_polarity(&mut self, polarity: Polarity) -> Result<(), Error> {
        self.polarity = checked_polarity(polarity)?;
        Ok(())
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
        object.core.keep_tags(persistence)?;
        Ok(object)
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

crate::io_core::impl_profile!(BinaryOutputObject);
crate::io_core::impl_reliability_eval!(BinaryOutputObject);
crate::write_config::reporting::impl_reporting_persistence!(BinaryOutputObject);
super::impl_state_texts!(BinaryOutputObject);

impl BACnetObject for BinaryOutputObject {
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
            return Ok(PropertyValue::Enumerated(self.present_value));
        }
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(value) = self.binary.read(property) {
            return Ok(value);
        }
        match property {
            P::FEEDBACK_VALUE => Ok(PropertyValue::Enumerated(self.feedback_value)),
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Enumerated(self.relinquish_default)),
            P::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            P::POLARITY => Ok(PropertyValue::Enumerated(self.polarity.to_raw())),
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if matches!(property, P::PRESENT_VALUE | P::VALUE_SOURCE) {
            return Err(common::write_access_denied_error());
        }
        if property == P::FEEDBACK_VALUE {
            if let PropertyValue::Enumerated(e) = value {
                if e > 1 {
                    return Err(common::value_out_of_range_error());
                }
                self.feedback_value = e;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == P::RELINQUISH_DEFAULT {
            if let PropertyValue::Enumerated(e) = value {
                return self.set_relinquish_default(e);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.7 requires simulation/test writes while Out_Of_Service is TRUE:
        // Present_Value is writable, as is Reliability when that property exists
        // and supports values beyond NO_FAULT_DETECTED; the core takes the
        // Reliability write. `is_writable_property` stays statically true
        // because it describes capability.
        let driven = self.physical_output();
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            if result? == crate::io_core::CoreWrite::OutOfService {
                // Out of service the output stays where it was driven. Any
                // other Boolean write leaves it as it was: back in service
                // `physical_output` reads no held state, and TRUE again
                // stores the state already held.
                self.held_output = driven;
            }
            return Ok(());
        }
        if let Some(result) = self.binary.write(property, value) {
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
        // This binary implementation has no instance-conditional bounds.
        let mut rows = Cow::Borrowed(metadata::OUTPUT);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

/// Polarity, the one input a Binary Output takes: its Present_Value is
/// commanded, never supplied.
impl crate::traits::LocalInput for BinaryOutputObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::Polarity(polarity) = input else {
            return Err(crate::traits::not_supported());
        };
        self.set_polarity(polarity)
    }
}

impl crate::creation::Creatable for BinaryOutputObject {}

impl crate::traits::CovReporting for BinaryOutputObject {}

crate::event::impl_builtin_intrinsic_reporting!(
    BinaryOutputObject,
    [present_value, feedback_value],
    CommandFailureDetector::ALGORITHM
);

#[cfg(test)]
#[path = "tests/command_failure.rs"]
mod command_failure_tests;
