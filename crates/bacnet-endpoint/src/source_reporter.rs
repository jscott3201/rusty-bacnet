//! Endpoint-private ownership projection. There is no source flag in bacnet-objects.

use bacnet_objects::database::AuditOwnership;
use std::borrow::Cow;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bacnet_objects::access_control::AccessControlInput;
use bacnet_objects::audit::{
    AuditLogForwarding, AuditLogNotificationSink, AuditLogStorage, AuditReporterObject,
};
use bacnet_objects::clock::ClockReader;
use bacnet_objects::command::{CommandRun, WriteFailure};
use bacnet_objects::durable::DurableWrites;
use bacnet_objects::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentMonitoredSource, EventEnrollmentReliabilityCommit,
};
use bacnet_objects::file::{FileConfiguration, FileStorage};
use bacnet_objects::log_buffer::{LogBufferRecords, LogRecordIdentity};
use bacnet_objects::log_reporting::BufferReadyReport;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::schedule::{ScheduleTargetOutcome, ScheduleWrite};
use bacnet_objects::staging::StagingWritePlan;
use bacnet_objects::traits::{
    BACnetObject, CovReportedProperty, DeadlineWaker, LifeSafetyOperationOutcome, MonotonicClock,
    ReliabilityEvaluation,
};
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter, EventTransitionBits};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord,
    BACnetObjectPropertyReference, BACnetObjectSelector,
};
use bacnet_types::enums::{
    AuditLevel, ErrorClass, ErrorCode, EventState, LifeSafetyOperation, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, Time};

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
// `trait_tests` runs each method the trait declares and fails on one that does
// not reach the wrapped object unchanged.
impl BACnetObject for SourceReporter {
    fn device_authority_internal(&mut self) -> Option<bacnet_objects::device::DeviceAuthority<'_>> {
        self.wrapped.device_authority_internal()
    }

    fn audit_reporter_authority_internal(
        &mut self,
    ) -> Option<bacnet_objects::audit::AuditReporterAuthority<'_>> {
        self.wrapped.audit_reporter_authority_internal()
    }

    fn audit_policy_authority_internal(
        &mut self,
    ) -> Option<bacnet_objects::audit::AuditPolicyAuthority<'_>> {
        self.wrapped.audit_policy_authority_internal()
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
        maximum_send_delay: Option<bacnet_objects::audit::AuditSendDelay>,
    ) -> Result<(), Error> {
        if self.active() && (selectors.is_some() || maximum_send_delay.is_some()) {
            return Err(Error::Encoding(
                "source Audit does not support Monitored_Objects or delayed notifications".into(),
            ));
        }
        self.wrapped.configure_audit_reporter_internal(
            level,
            operations,
            confirmed,
            selectors,
            priorities,
            maximum_send_delay,
        )
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

    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &bacnet_objects::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        if self.active() && property == PropertyIdentifier::AUDIT_SOURCE_REPORTER {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            });
        }
        self.wrapped
            .write_property_from(property, array_index, value, priority, origin)
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

    fn bind_deadline_waker_internal(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.wrapped.bind_deadline_waker_internal(waker)
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.wrapped.advance_monotonic_time_internal(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.wrapped.next_monotonic_deadline_internal()
    }

    fn set_tracking_cov_increment_internal(&mut self, finest: Option<f64>) {
        self.wrapped.set_tracking_cov_increment_internal(finest)
    }

    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        self.wrapped.cov_snapshot_internal()
    }

    fn lighting_blink_count_internal(&self) -> u64 {
        self.wrapped.lighting_blink_count_internal()
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

    fn is_createable(&self) -> bool {
        self.wrapped.is_createable()
    }

    fn creation_only_properties(&self) -> &'static [PropertyIdentifier] {
        self.wrapped.creation_only_properties()
    }

    fn initialize_property(
        &mut self,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.wrapped.initialize_property(property, value)
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

    fn supports_subscribe_cov_property(&self) -> bool {
        self.wrapped.supports_subscribe_cov_property()
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

    fn take_command_run_internal(&mut self) -> Option<CommandRun> {
        self.wrapped.take_command_run_internal()
    }

    fn command_generation_internal(&self) -> Option<u64> {
        self.wrapped.command_generation_internal()
    }

    fn record_command_write_internal(
        &mut self,
        generation: u64,
        command: usize,
        success: bool,
    ) -> bool {
        self.wrapped
            .record_command_write_internal(generation, command, success)
    }

    fn remember_member_datatype_internal(
        &mut self,
        slot: usize,
        reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
        datatype: Option<bacnet_objects::channel::MemberDatatype>,
    ) {
        self.wrapped
            .remember_member_datatype_internal(slot, reference, datatype);
    }

    fn complete_command_run_internal(
        &mut self,
        generation: u64,
        outcome: Result<(), WriteFailure>,
    ) -> bool {
        self.wrapped
            .complete_command_run_internal(generation, outcome)
    }

    fn enrollment_summary_capability_internal(&self) -> Option<EnrollmentSummaryCapability> {
        self.wrapped.enrollment_summary_capability_internal()
    }

    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        self.wrapped.supports_cov_property(property)
    }

    fn cov_increment(&self) -> Option<f64> {
        self.wrapped.cov_increment()
    }

    fn cov_reported_properties(&self) -> &'static [CovReportedProperty] {
        self.wrapped.cov_reported_properties()
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
        today: SpecificDate,
        time: Time,
        calendar_active: &dyn Fn(ObjectIdentifier) -> bool,
    ) -> Option<ScheduleWrite> {
        self.wrapped.tick_schedule(today, time, calendar_active)
    }

    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        self.wrapped.take_owed_schedule_writes()
    }

    fn complete_schedule_write(
        &mut self,
        write: &ScheduleWrite,
        outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        self.wrapped.complete_schedule_write(write, outcomes)
    }

    fn retry_refusals_naming(&self, target: ObjectIdentifier) -> Option<ScheduleWrite> {
        self.wrapped.retry_refusals_naming(target)
    }

    fn calendar_state_internal(&self, day: SpecificDate) -> Option<bool> {
        self.wrapped.calendar_state_internal(day)
    }

    fn acknowledge_alarm(&mut self, transition_bit: EventTransitionBits) -> Result<(), Error> {
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
        transition_bit: EventTransitionBits,
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

    fn set_reliability_internal(&mut self, reliability: Reliability) -> Result<(), Error> {
        self.wrapped.set_reliability_internal(reliability)
    }

    fn set_present_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.wrapped.set_present_value_internal(value)
    }

    fn set_present_value_from_internal(
        &mut self,
        value: PropertyValue,
        origin: &bacnet_objects::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.wrapped.set_present_value_from_internal(value, origin)
    }

    fn set_tracking_value_internal(&mut self, value: PropertyValue) -> Result<(), Error> {
        self.wrapped.set_tracking_value_internal(value)
    }

    fn report_access_input_internal(&mut self, input: AccessControlInput) -> Result<(), Error> {
        self.wrapped.report_access_input_internal(input)
    }

    fn set_controlled_variable_value_internal(
        &mut self,
        value: PropertyValue,
    ) -> Result<(), Error> {
        self.wrapped.set_controlled_variable_value_internal(value)
    }

    fn add_averaging_sample_internal(
        &mut self,
        sample: Option<PropertyValue>,
    ) -> Result<(), Error> {
        self.wrapped.add_averaging_sample_internal(sample)
    }

    fn take_due_averaging_sample_internal(
        &mut self,
        now: Duration,
    ) -> Option<BACnetObjectPropertyReference> {
        self.wrapped.take_due_averaging_sample_internal(now)
    }

    fn input_reference_internal(&self) -> Option<Option<&BACnetObjectPropertyReference>> {
        self.wrapped.input_reference_internal()
    }

    fn set_input_usable_internal(&mut self, usable: bool) -> bool {
        self.wrapped.set_input_usable_internal(usable)
    }

    fn take_input_reading_internal(
        &mut self,
        reading: Option<bacnet_objects::accumulator::InputReading>,
    ) -> bool {
        self.wrapped.take_input_reading_internal(reading)
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

    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        self.wrapped.durable_writes_internal()
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

    fn log_buffer_internal(&self) -> Option<&dyn LogBufferRecords> {
        self.wrapped.log_buffer_internal()
    }

    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.wrapped.add_trend_record(record)
    }

    fn add_trend_multiple_record(&mut self, record: BACnetLogMultipleRecord) -> Result<(), Error> {
        self.wrapped.add_trend_multiple_record(record)
    }

    fn add_event_log_record(&mut self, record: BACnetEventLogRecord) -> Result<(), Error> {
        self.wrapped.add_event_log_record(record)
    }

    fn refresh_log_window_internal(&mut self) -> bool {
        self.wrapped.refresh_log_window_internal()
    }

    fn logs_received_event_notifications_internal(&self) -> bool {
        self.wrapped.logs_received_event_notifications_internal()
    }

    fn buffer_ready_report_internal(&self) -> Option<BufferReadyReport> {
        self.wrapped.buffer_ready_report_internal()
    }

    fn event_algorithm_inhibit_reference_internal(&self) -> Option<BACnetObjectPropertyReference> {
        self.wrapped.event_algorithm_inhibit_reference_internal()
    }

    fn follow_event_algorithm_inhibit_internal(&mut self, inhibit: bool) -> bool {
        self.wrapped
            .follow_event_algorithm_inhibit_internal(inhibit)
    }
}

#[cfg(test)]
#[path = "source_reporter_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "source_reporter_trait_tests.rs"]
mod trait_tests;
