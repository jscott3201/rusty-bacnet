//! Endpoint-private ownership projection. There is no source flag in bacnet-objects.

use bacnet_objects::database::AuditOwnership;
use std::borrow::Cow;
use std::sync::{Arc, Weak};

use bacnet_objects::audit::{AuditReporterAuthority, AuditReporterObject};
use bacnet_objects::durable::DurableWrites;
use bacnet_objects::file::{FileConfiguration, FileStorage};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::{
    AccessEvents, AuditLog, AuditPolicy, BACnetObject, CalendarState, CommandRunner, CovReporting,
    EnrollmentEval, EventReporting, LifeSafety, LocalInput, LogBuffer, ReliabilityEval,
    ScheduleRunner, StagingSource, Timed,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

struct SourceReporter {
    owner: Weak<AuditOwnership>,
    wrapped: Box<dyn BACnetObject>,
}

// Installation belongs to the complete endpoint source owner. The weak lease
// cannot retain database membership after that owner and its workers quiesce.
pub(crate) fn install(
    slot: &mut Box<dyn BACnetObject>,
    owner: &Arc<AuditOwnership>,
) -> Result<(), Error> {
    // Allocate everything before touching the live entry. An inert built-in
    // Reporter is just a temporary move placeholder, never queried or published.
    // After the swap there is no allocation, fallible operation, await, or user
    // callback (including Drop: the overwritten placeholder is our built-in).
    let mut adapter = Box::new(SourceReporter {
        owner: Arc::downgrade(owner),
        wrapped: Box::new(AuditReporterObject::new(0, "")?),
    });
    std::mem::swap(slot, &mut adapter.wrapped);
    *slot = adapter;
    Ok(())
}

impl SourceReporter {
    fn active(&self) -> bool {
        self.owner.upgrade().is_some_and(|owner| owner.is_active())
    }
}

// Delegate every BACnetObject method, including defaulted/hidden hooks: inheriting
// a default here would silently discard a downstream object's override. Only the
// source property and its write gate belong to this adapter; the database keeps
// the active Reporter from being removed (`ObjectDatabase::check_removable`).
// `trait_tests` runs each method the trait declares and fails on one that does
// not reach the wrapped object unchanged.
impl BACnetObject for SourceReporter {
    fn device_authority_internal(&mut self) -> Option<bacnet_objects::device::DeviceAuthority<'_>> {
        self.wrapped.device_authority_internal()
    }

    fn audit_policy(&self) -> Option<&dyn AuditPolicy> {
        self.wrapped.audit_policy()
    }

    fn audit_policy_mut(&mut self) -> Option<&mut dyn AuditPolicy> {
        self.wrapped.audit_policy_mut()
    }

    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        self.wrapped.audit_reporter()
    }

    // The Reporter's configuration answers to this source owner: while it is
    // active, Monitored_Objects and a send delay are refused.
    fn audit_reporter_mut(&mut self) -> Option<AuditReporterAuthority<'_>> {
        let owner = &self.owner;
        self.wrapped
            .audit_reporter_mut()
            .map(|reporter| reporter.with_source_owner(owner))
    }

    fn object_identifier(&self) -> ObjectIdentifier {
        self.wrapped.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.wrapped.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if self.active() && property == PropertyIdentifier::AUDIT_SOURCE_REPORTER {
            Ok(PropertyValue::Boolean(true))
        } else {
            self.wrapped.read_property(property, array_index)
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if self.active() && property == PropertyIdentifier::AUDIT_SOURCE_REPORTER {
            return Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            ));
        }
        self.wrapped
            .write_property(property, array_index, value, priority)
    }

    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &bacnet_objects::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        if self.active() && property == PropertyIdentifier::AUDIT_SOURCE_REPORTER {
            return Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            ));
        }
        self.wrapped
            .write_property_from(property, array_index, value, priority, origin)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.wrapped.property_metadata()
    }

    fn timed(&mut self) -> Option<&mut dyn Timed> {
        self.wrapped.timed()
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        (!self.active() || property != PropertyIdentifier::AUDIT_SOURCE_REPORTER)
            && self.wrapped.is_writable_property(property)
    }

    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        self.wrapped.is_array_property(property)
    }

    fn is_list_property(&self, property: PropertyIdentifier) -> bool {
        self.wrapped.is_list_property(property)
    }

    fn staging_source(&mut self) -> Option<&mut dyn StagingSource> {
        self.wrapped.staging_source()
    }

    fn command_runner(&self) -> Option<&dyn CommandRunner> {
        self.wrapped.command_runner()
    }

    fn command_runner_mut(&mut self) -> Option<&mut dyn CommandRunner> {
        self.wrapped.command_runner_mut()
    }

    fn local_input(&mut self) -> Option<&mut dyn LocalInput> {
        self.wrapped.local_input()
    }

    fn access_events(&mut self) -> Option<&mut dyn AccessEvents> {
        self.wrapped.access_events()
    }

    fn life_safety(&mut self) -> Option<&mut dyn LifeSafety> {
        self.wrapped.life_safety()
    }

    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.wrapped.event_reporting()
    }

    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        self.wrapped.event_reporting_mut()
    }

    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        self.wrapped.enrollment_eval()
    }

    fn enrollment_eval_mut(&mut self) -> Option<&mut dyn EnrollmentEval> {
        self.wrapped.enrollment_eval_mut()
    }

    fn reliability_eval(&mut self) -> Option<&mut dyn ReliabilityEval> {
        self.wrapped.reliability_eval()
    }

    fn schedule_runner(&mut self) -> Option<&mut dyn ScheduleRunner> {
        self.wrapped.schedule_runner()
    }

    fn calendar_state(&self) -> Option<&dyn CalendarState> {
        self.wrapped.calendar_state()
    }

    fn cov(&self) -> Option<&dyn CovReporting> {
        self.wrapped.cov()
    }

    fn audit_log(&self) -> Option<&dyn AuditLog> {
        self.wrapped.audit_log()
    }

    fn audit_log_mut(&mut self) -> Option<&mut dyn AuditLog> {
        self.wrapped.audit_log_mut()
    }

    fn durable_writes(&mut self) -> Option<&mut dyn DurableWrites> {
        self.wrapped.durable_writes()
    }

    fn file_configuration(&self) -> Option<&dyn FileConfiguration> {
        self.wrapped.file_configuration()
    }

    fn file_configuration_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        self.wrapped.file_configuration_mut()
    }

    fn file_storage(&self) -> Option<&dyn FileStorage> {
        self.wrapped.file_storage()
    }

    fn file_storage_mut(&mut self) -> Option<&mut dyn FileStorage> {
        self.wrapped.file_storage_mut()
    }

    fn log_buffer(&self) -> Option<&dyn LogBuffer> {
        self.wrapped.log_buffer()
    }

    fn log_buffer_mut(&mut self) -> Option<&mut dyn LogBuffer> {
        self.wrapped.log_buffer_mut()
    }
}

#[cfg(test)]
#[path = "source_reporter_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "source_reporter_trait_tests.rs"]
mod trait_tests;
