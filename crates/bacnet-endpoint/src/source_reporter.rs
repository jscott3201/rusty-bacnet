//! Endpoint-private ownership projection. There is no source flag in bacnet-objects.

use bacnet_objects::database::AuditOwnership;
use std::borrow::Cow;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bacnet_objects::audit::{
    AuditLogForwarding, AuditLogNotificationSink, AuditLogStorage, AuditReporterObject,
};
use bacnet_objects::clock::ClockReader;
use bacnet_objects::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentReliabilityCommit,
};
use bacnet_objects::file::{FileConfiguration, FileStorage};
use bacnet_objects::log_buffer::LogRecordIdentity;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::staging::StagingWritePlan;
use bacnet_objects::traits::{
    BACnetObject, LifeSafetyOperationOutcome, MonotonicClock, ReliabilityEvaluation,
};
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetLogRecord, BACnetObjectSelector,
};
use bacnet_types::enums::{
    AuditLevel, ErrorClass, ErrorCode, EventState, LifeSafetyOperation, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};

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
// source property and its write gate, plus deletion, belong to this adapter.
impl BACnetObject for SourceReporter {
    fn device_authority_internal(&mut self) -> Option<bacnet_objects::device::DeviceAuthority<'_>> {
        self.wrapped.device_authority_internal()
    }

    fn audit_object_policy_internal(&self) -> bacnet_objects::audit::ObjectAuditPolicy {
        self.wrapped.audit_object_policy_internal()
    }

    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        self.wrapped.audit_reporter_internal()
    }

    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
    ) -> Result<(), Error> {
        if self.active() && selectors.is_some() {
            return Err(Error::Encoding(
                "source READ does not support Monitored_Objects".into(),
            ));
        }
        self.wrapped
            .configure_audit_reporter_internal(level, operations, confirmed, selectors, priorities)
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
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            });
        }
        self.wrapped
            .write_property(property, array_index, value, priority)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.wrapped.property_metadata()
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.wrapped.property_list()
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.wrapped.bind_clock_internal(clock)
    }

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.wrapped.advance_time_internal(elapsed)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.wrapped.bind_monotonic_clock_internal(clock)
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.wrapped.advance_monotonic_time_internal(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.wrapped.next_monotonic_deadline_internal()
    }

    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        self.wrapped.cov_snapshot_internal()
    }

    fn binary_lighting_blink_count_internal(&self) -> u64 {
        self.wrapped.binary_lighting_blink_count_internal()
    }

    fn is_writable_property(&self, property: PropertyIdentifier) -> bool {
        (!self.active() || property != PropertyIdentifier::AUDIT_SOURCE_REPORTER)
            && self.wrapped.is_writable_property(property)
    }

    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        self.wrapped.is_array_property(property)
    }

    fn is_createable(&self) -> bool {
        self.wrapped.is_createable()
    }

    fn is_deleteable(&self) -> bool {
        !self.active() && self.wrapped.is_deleteable()
    }

    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.wrapped.required_properties()
    }

    fn supports_cov(&self) -> bool {
        self.wrapped.supports_cov()
    }

    fn take_staging_write_plan_internal(&mut self) -> Option<StagingWritePlan> {
        self.wrapped.take_staging_write_plan_internal()
    }

    fn staging_generation_internal(&self) -> Option<u64> {
        self.wrapped.staging_generation_internal()
    }

    fn complete_staging_write_plan_internal(&mut self, generation: u64, success: bool) -> bool {
        self.wrapped
            .complete_staging_write_plan_internal(generation, success)
    }

    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        self.wrapped.enrollment_summary_capability_internal()
    }

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        self.wrapped.supports_cov_property(property)
    }

    fn cov_increment(&self) -> Option<f32> {
        self.wrapped.cov_increment()
    }

    fn set_overridden(&mut self, overridden: bool) {
        self.wrapped.set_overridden(overridden)
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.wrapped.evaluate_intrinsic_reporting()
    }

    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.wrapped.tick_intrinsic_reporting()
    }

    fn commit_event_transition_internal(
        &mut self,
        commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        self.wrapped.commit_event_transition_internal(commit)
    }

    fn commit_event_enrollment_reliability_internal(
        &mut self,
        commit: EventEnrollmentReliabilityCommit,
    ) -> Result<(), EventTransitionCommitError> {
        self.wrapped
            .commit_event_enrollment_reliability_internal(commit)
    }

    fn tick_schedule(
        &mut self,
        day_of_week: u8,
        hour: u8,
        minute: u8,
    ) -> Option<(PropertyValue, Vec<(ObjectIdentifier, u32)>)> {
        self.wrapped.tick_schedule(day_of_week, hour, minute)
    }

    fn acknowledge_alarm(&mut self, transition_bit: u8) -> Result<(), Error> {
        self.wrapped.acknowledge_alarm(transition_bit)
    }

    fn acknowledge_alarm_correlated_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<(), Error> {
        self.wrapped
            .acknowledge_alarm_correlated_internal(event_state, timestamp)
    }

    fn acknowledge_alarm_correlated_detailed_internal(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        self.wrapped
            .acknowledge_alarm_correlated_detailed_internal(event_state, timestamp)
    }

    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        self.wrapped.apply_life_safety_operation(operation)
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.wrapped
            .set_life_safety_operation_expected_internal(operation)
    }

    fn set_event_state_internal(&mut self, state: EventState) -> Result<(), Error> {
        self.wrapped.set_event_state_internal(state)
    }

    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        self.wrapped.enrollment_eval_state_internal()
    }

    fn set_enrollment_eval_state_internal(
        &mut self,
        state: EventEnrollmentEvalState,
    ) -> Result<(), Error> {
        self.wrapped.set_enrollment_eval_state_internal(state)
    }

    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        self.wrapped.enrollment_eval_source_internal()
    }

    fn set_enrollment_eval_source_internal(
        &mut self,
        source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        self.wrapped.set_enrollment_eval_source_internal(source)
    }

    fn set_acked_transitions_internal(
        &mut self,
        transition_bit: u8,
        acknowledged: bool,
    ) -> Result<(), Error> {
        self.wrapped
            .set_acked_transitions_internal(transition_bit, acknowledged)
    }

    fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
        self.wrapped.evaluate_reliability_internal()
    }

    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        self.wrapped.reliability_evaluation_inhibited_internal()
    }

    fn set_reliability_internal(&mut self, reliability: u32) -> Result<(), Error> {
        self.wrapped.set_reliability_internal(reliability)
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.wrapped.set_present_value_internal(value)
    }

    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        self.wrapped.audit_log_storage_internal()
    }

    fn audit_log_forwarding_internal(&self) -> Option<Arc<AuditLogForwarding>> {
        self.wrapped.audit_log_forwarding_internal()
    }

    fn set_audit_log_parent_internal(
        &mut self,
        parent: BACnetDeviceObjectReference,
    ) -> Result<(), Error> {
        self.wrapped.set_audit_log_parent_internal(parent)
    }

    fn audit_log_notification_sink_internal(
        &mut self,
    ) -> Option<&mut dyn AuditLogNotificationSink> {
        self.wrapped.audit_log_notification_sink_internal()
    }

    fn file_configuration_internal(&self) -> Option<&dyn FileConfiguration> {
        self.wrapped.file_configuration_internal()
    }

    fn file_configuration_internal_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        self.wrapped.file_configuration_internal_mut()
    }

    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        self.wrapped.file_storage_internal()
    }

    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        self.wrapped.file_storage_internal_mut()
    }

    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        self.wrapped.log_record_identities_internal()
    }

    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.wrapped.add_trend_record(record)
    }
}
