use super::*;
use crate::object_profile::TagsPersistence;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// BinaryInput (type 3)
// ---------------------------------------------------------------------------

/// BACnet Binary Input object.
///
/// Read-only binary point. Present_Value is writable only when out-of-service.
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryInputObject {
    pub(super) core: IoCore,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    binary: BinaryProperties,
    present_value: u32,
    /// Polarity: 0 = normal, 1 = reverse.
    polarity: u32,
}

impl BinaryInputObject {
    /// Create a new Binary Input object; fails if `instance` exceeds the object-identifier range.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            core: IoCore::new(ObjectType::BINARY_INPUT, instance, name.into())?,
            event_detector: alarm_value_detector(),
            binary: BinaryProperties::new(),
            present_value: 0,
            polarity: 0,
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
        object.core.keep_tags(persistence)?;
        Ok(object)
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
}

crate::io_core::impl_profile!(BinaryInputObject);
crate::io_core::impl_reliability_eval!(BinaryInputObject);
crate::write_config::reporting::impl_reporting_persistence!(BinaryInputObject, in core);
super::impl_state_texts!(BinaryInputObject);

impl BACnetObject for BinaryInputObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.core.oid
    }

    fn object_name(&self) -> &str {
        &self.core.name
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

    crate::traits::lend_cov_reporting!();

    crate::traits::lend_event_reporting!();

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(value) = self.binary.read(property) {
            return Ok(value);
        }
        match property {
            P::PRESENT_VALUE => Ok(PropertyValue::Enumerated(self.present_value)),
            P::POLARITY => Ok(PropertyValue::Enumerated(self.polarity)),
            P::ALARM_VALUE => Ok(read_alarm_value(&self.event_detector)),
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
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            return result.map(drop);
        }
        if property == P::ALARM_VALUE {
            return write_alarm_value(&mut self.event_detector, value);
        }
        if let Some(result) = self.binary.write(property, value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
        ))
    }

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

impl crate::creation::Creatable for BinaryInputObject {}

impl crate::traits::CovReporting for BinaryInputObject {}

crate::event::impl_builtin_intrinsic_reporting!(
    BinaryInputObject,
    event_detector,
    core.event_history,
    [present_value],
    core.reliability,
    core.event_detection_enable,
    ChangeOfStateDetector::ALGORITHM
);

/// The Present_Value the application supplies.
impl crate::traits::LocalInput for BinaryInputObject {
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
#[path = "tests/input_output.rs"]
mod input_output_tests;
