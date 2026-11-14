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
///
/// The application feeds it from either side (#2580): the logical
/// Present_Value ([`Self::set_present_value`], or
/// [`LocalInputValue::PresentValue`](crate::traits::LocalInputValue::PresentValue)),
/// or the raw state its physical input reads ([`Self::set_physical_input`],
/// or [`LocalInputValue::PhysicalInput`](crate::traits::LocalInputValue::PhysicalInput)),
/// which the object turns into Present_Value through Polarity. Either way
/// the object keeps the physical state, so a later Polarity change or a
/// return to service derives Present_Value from it again.
pub struct BinaryInputObject {
    pub(super) core: IoCore,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    binary: BinaryProperties,
    present_value: u32,
    /// The physical input's state, ON as `true`, once the application has
    /// supplied one, directly or as the logical value it implies; `None`
    /// before that.
    physical_input: Option<bool>,
    polarity: Polarity,
}

impl BinaryInputObject {
    /// Create a new Binary Input object; fails if `instance` exceeds the object-identifier range.
    ///
    /// Polarity starts NORMAL and Present_Value INACTIVE, with no physical
    /// input state until the application supplies one.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        Ok(Self {
            core: IoCore::new(ObjectType::BINARY_INPUT, instance, name.into())?,
            event_detector: alarm_value_detector(),
            binary: BinaryProperties::new(),
            present_value: 0,
            physical_input: None,
            polarity: Polarity::NORMAL,
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
    /// not a raw electrical or physical state: a raw state goes to
    /// [`Self::set_physical_input`]. In service the object also takes the
    /// physical input state that gives `value` under the current Polarity,
    /// so a later Polarity change derives Present_Value from that state.
    /// Out of service it sets the simulated value only, as a client's write
    /// would. This low-level helper bypasses running-server `Out_Of_Service`
    /// ownership, validation, intrinsic-event processing, and COV
    /// processing. Applications updating a live object should use
    /// `BACnetServer::set_present_value_local`.
    pub fn set_present_value(&mut self, value: u32) {
        self.present_value = value;
        if !self.core.out_of_service {
            self.physical_input = Some(across_polarity(is_active(value), self.polarity));
        }
    }

    /// Take the state the application read from the physical input, ON as
    /// `true` (#2580).
    ///
    /// In service, Present_Value becomes that state through Polarity:
    /// ACTIVE for ON under NORMAL, INACTIVE for ON under REVERSE (Table
    /// 12-7). While Out_Of_Service is TRUE, Present_Value doesn't follow
    /// the input (Clause 12.6.10): the object keeps the state and serves the
    /// value a client simulates, and the return to service derives
    /// Present_Value from the latest state. Like [`Self::set_present_value`]
    /// this runs no event or COV processing; on a running server use
    /// `BACnetServer::set_physical_input_local`.
    pub fn set_physical_input(&mut self, on: bool) {
        self.physical_input = Some(on);
        self.follow_physical_input();
    }

    /// Set Polarity, NORMAL or REVERSE (#2580); any other value fails with
    /// PROPERTY / VALUE_OUT_OF_RANGE and changes nothing. A new object's
    /// is NORMAL, and a client can't write it.
    ///
    /// In service, Present_Value is derived again from the physical input
    /// state the object holds, so a steady input changes Present_Value when
    /// Polarity changes (Clause 12.6.11); before the application has
    /// supplied any state, Present_Value stays as it is. While
    /// Out_Of_Service is TRUE, Polarity has no effect on Present_Value, and
    /// the return to service derives it with the new Polarity. This runs no
    /// event or COV processing; on a running server use
    /// `BACnetServer::set_polarity_local`.
    pub fn set_polarity(&mut self, polarity: Polarity) -> Result<(), Error> {
        self.polarity = checked_polarity(polarity)?;
        self.follow_physical_input();
        Ok(())
    }

    /// In service, and once the application has supplied a physical input
    /// state, make Present_Value that state through Polarity.
    fn follow_physical_input(&mut self) {
        if let Some(on) = self.physical_input
            && !self.core.out_of_service
        {
            self.present_value = u32::from(across_polarity(on, self.polarity));
        }
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
crate::write_config::reporting::impl_reporting_persistence!(BinaryInputObject);
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
            P::POLARITY => Ok(PropertyValue::Enumerated(self.polarity.to_raw())),
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
            if result? == crate::io_core::CoreWrite::OutOfService {
                // Back in service, Present_Value follows the input again.
                self.follow_physical_input();
            }
            return Ok(());
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
    [present_value],
    ChangeOfStateDetector::ALGORITHM
);

/// The logical Present_Value the application supplies, the physical input
/// state it read, and Polarity.
impl crate::traits::LocalInput for BinaryInputObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        use crate::traits::LocalInputValue as Input;
        match input {
            Input::PresentValue { value, .. } => {
                // Local safe-ownership policy: preserve the client's OOS simulation.
                if self.core.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                self.apply_present_value(value)?;
                // The physical state this logical value implies.
                self.physical_input = Some(across_polarity(
                    is_active(self.present_value),
                    self.polarity,
                ));
                Ok(())
            }
            // The hardware's state is taken out of service too: only
            // Present_Value is the client's then.
            Input::PhysicalInput(on) => {
                self.set_physical_input(on);
                Ok(())
            }
            Input::Polarity(polarity) => self.set_polarity(polarity),
            _ => Err(crate::traits::not_supported()),
        }
    }
}

#[cfg(test)]
#[path = "tests/input_output.rs"]
mod input_output_tests;
