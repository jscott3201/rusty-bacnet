use super::*;
use crate::object_profile::TagsPersistence;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

// ---------------------------------------------------------------------------
// BinaryValue (type 5)
// ---------------------------------------------------------------------------

/// BACnet Binary Value object.
///
/// Uses Enumerated values: 0 = inactive, 1 = active.
pub struct BinaryValueObject {
    pub(super) core: IoCore,
    /// CHANGE_OF_STATE event detector.
    event_detector: ChangeOfStateDetector,
    binary: BinaryProperties,
    present_value: u32, // 0 = inactive, 1 = active
    audit_policy: crate::audit::ObjectAuditPolicy,
    access: PresentValueAccess,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
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
        Ok(Self {
            core: IoCore::new(ObjectType::BINARY_VALUE, instance, name.into())?,
            event_detector: alarm_value_detector(),
            binary: BinaryProperties::new(),
            present_value: 0, // inactive
            audit_policy: crate::audit::ObjectAuditPolicy::default(),
            access,
            priority_array: [None; 16],
            relinquish_default: 0,
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
        access: PresentValueAccess,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::with_access(instance, name, access)?;
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
        if self.access == PresentValueAccess::ReadOnly && !self.core.out_of_service {
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

    /// The rows before the access projects them: Table 12-10's, then the
    /// audit rows when provisioned and the profile's.
    fn rows(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(metadata::VALUE);
        if self.audit_policy != crate::audit::ObjectAuditPolicy::default() {
            rows.to_mut().extend(self.audit_policy.metadata());
        }
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

crate::io_core::impl_profile!(BinaryValueObject);
crate::io_core::impl_reliability_eval!(BinaryValueObject);
crate::write_config::reporting::impl_reporting_persistence!(BinaryValueObject);
super::impl_state_texts!(BinaryValueObject);

impl BACnetObject for BinaryValueObject {
    crate::traits::lend_audit_policy!();

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
            return Ok(PropertyValue::Enumerated(self.present_value));
        }
        if let Some(result) = self.audit_policy.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(value) = self.binary.read(property) {
            return Ok(value);
        }
        match property {
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Enumerated(self.relinquish_default)),
            P::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            P::ALARM_VALUE => Ok(read_alarm_value(&self.event_detector)),
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
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        if let Some(result) = self
            .audit_policy
            .write(property, array_index, &value, priority)
        {
            return result;
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
            if let PropertyValue::Enumerated(e) = value {
                return self.set_relinquish_default(e);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.8 requires simulation/test writes while Out_Of_Service is TRUE:
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

impl crate::creation::Creatable for BinaryValueObject {}

impl crate::traits::CovReporting for BinaryValueObject {}

crate::traits::impl_audit_policy!(BinaryValueObject);

crate::event::impl_builtin_intrinsic_reporting!(
    BinaryValueObject,
    [present_value],
    ChangeOfStateDetector::ALGORITHM
);

/// The Present_Value the application supplies while it isn't commandable,
/// published as Value_Source from `origin` when the object tracks its
/// source (#1552).
impl crate::traits::LocalInput for BinaryValueObject {
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

impl BinaryValueObject {
    /// The application's Present_Value, refused while it's commandable or
    /// Out_Of_Service is TRUE.
    fn apply_local_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        match self.access {
            PresentValueAccess::Commandable => Err(crate::traits::not_supported()),
            _ if self.core.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.present_value = Self::checked_present_value(value)?;
                self.write_source.forget();
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/value.rs"]
mod value_tests;
