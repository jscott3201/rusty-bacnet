//! An object that answers every `BACnetObject` method itself, none the way
//! the trait default would, and logs each call it receives with its
//! arguments.

use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::fixtures::{METADATA, address, noon, oid, property_error};
use bacnet_objects::access_control::{AccessEventReport, CommittedAccessEvent};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::audit::{
    AuditLogObject, AuditLogPersistence, AuditLogSnapshot, AuditReporterAuthority,
    AuditReporterObject, ObjectAuditPolicy,
};
use bacnet_objects::command::{CommandRun, RunPlan, WriteFailure};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::device::{DeviceAuthority, DeviceConfig, DeviceObject};
use bacnet_objects::durable::DurableWrites;
use bacnet_objects::event::{
    EnrollmentSummaryCapability, EventStateChange, EventTransitionCommit,
    EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::file::{FileConfiguration, FileObject, FileStorage};
use bacnet_objects::log_reporting::BufferReadyReport;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::staging::StagingWritePlan;
use bacnet_objects::traits::{
    AccessEvents, AuditLog, AuditPolicy, BACnetObject, CalendarState, Clocks, CommandRunner,
    CovReportedProperty, CovReporting, EnrollmentEval, EventReporting, LifeSafety,
    LifeSafetyOperationEffect, LifeSafetyOperationOutcome, LocalInput, LocalInputValue, LogBuffer,
    ReliabilityEval, ScheduleRunner, StagingSource, Timed,
};
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetObjectPropertyReference};
use bacnet_types::enums::{
    AuditLevel, EngineeringUnits, ErrorClass, ErrorCode, EventState, EventType,
    LifeSafetyOperation, ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, Time};

#[path = "access_batch.rs"]
mod access_batch;
#[path = "audit.rs"]
mod audit;
#[path = "eval.rs"]
mod eval;
#[path = "log.rs"]
mod log;
#[path = "schedule.rs"]
mod schedule;

pub const CUSTOM: P = P::from_raw(5000);
const REPORTED: [CovReportedProperty; 1] = [CovReportedProperty::Trigger(CUSTOM)];

/// The calls a probe received, oldest first, each as `name(arguments)`.
pub type CallLog = Arc<Mutex<Vec<String>>>;

/// Empty the log, returning what it held.
pub fn take(log: &CallLog) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

struct NoPersistence;

impl AuditLogPersistence for NoPersistence {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(None)
    }
    fn commit(&self, _: &AuditLogSnapshot) -> Result<(), Error> {
        Ok(())
    }
}

/// Answers every method of the trait, none the way its default would. The
/// capability hooks lend out objects the probe owns, so an adapter has to
/// hand out the probe's own and not a copy.
pub struct Probe {
    oid: ObjectIdentifier,
    log: CallLog,
    device: DeviceObject,
    value: AnalogValueObject,
    reporter: AuditReporterObject,
    audit_log: AuditLogObject,
    file: FileObject,
    trend_log: TrendLogObject,
}

impl Probe {
    pub fn new(object_type: ObjectType, log: CallLog) -> Self {
        let mut audit_log =
            AuditLogObject::new(3, "Probe log", 4, Arc::new(NoPersistence)).unwrap();
        audit_log.set_member_of(Some(BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: oid(ObjectType::AUDIT_LOG, 9),
        }));
        let mut value =
            AnalogValueObject::new(7, "Probe value", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        value.set_audit_policy(ObjectAuditPolicy {
            level: Some(AuditLevel::NONE),
            ..ObjectAuditPolicy::default()
        });
        Self {
            oid: oid(object_type, 1),
            log,
            device: DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(77, 0)
                    .expect("valid Device identity"),
                ..DeviceConfig::default()
            })
            .unwrap(),
            value,
            reporter: AuditReporterObject::new(17, "Probe reporter").unwrap(),
            audit_log,
            file: FileObject::new(5, "Probe file", "test").unwrap(),
            trend_log: TrendLogObject::new(6, "Probe trend", 4).unwrap(),
        }
    }

    /// Log one call: `arguments` is a tuple, `()` for none.
    fn called(&self, method: &str, arguments: impl Debug) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{method}{arguments:?}"));
    }
}

impl BACnetObject for Probe {
    fn device_authority_internal(&mut self) -> Option<DeviceAuthority<'_>> {
        self.called("device_authority_internal", ());
        self.device.device_authority_internal()
    }
    fn audit_policy(&self) -> Option<&dyn AuditPolicy> {
        self.called("audit_policy", ());
        Some(self)
    }
    fn audit_policy_mut(&mut self) -> Option<&mut dyn AuditPolicy> {
        self.called("audit_policy_mut", ());
        Some(self)
    }
    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        self.called("audit_reporter", ());
        Some(&self.reporter)
    }
    fn audit_reporter_mut(&mut self) -> Option<AuditReporterAuthority<'_>> {
        self.called("audit_reporter_mut", ());
        self.reporter.audit_reporter_mut()
    }
    fn object_identifier(&self) -> ObjectIdentifier {
        self.called("object_identifier", ());
        self.oid
    }
    fn object_name(&self) -> &str {
        self.called("object_name", ());
        "Probe"
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        self.called("read_property", (property, index));
        if property == CUSTOM {
            Ok(PropertyValue::Unsigned(7))
        } else {
            Err(property_error(ErrorCode::UNKNOWN_PROPERTY))
        }
    }
    fn write_property(
        &mut self,
        property: P,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.called("write_property", (property, index, value, priority));
        Ok(())
    }
    fn write_property_from(
        &mut self,
        property: P,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        let arguments = (property, index, value, priority, origin);
        self.called("write_property_from", arguments);
        // Unlike the default, which answers as `write_property` does.
        Err(property_error(ErrorCode::VALUE_OUT_OF_RANGE))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.called("property_metadata", ());
        Cow::Borrowed(&METADATA)
    }
    fn timed(&mut self) -> Option<&mut dyn Timed> {
        self.called("timed", ());
        Some(self)
    }
    fn staging_source(&mut self) -> Option<&mut dyn StagingSource> {
        self.called("staging_source", ());
        Some(self)
    }
    fn command_runner(&self) -> Option<&dyn CommandRunner> {
        self.called("command_runner", ());
        Some(self)
    }
    fn command_runner_mut(&mut self) -> Option<&mut dyn CommandRunner> {
        self.called("command_runner_mut", ());
        Some(self)
    }
    fn local_input(&mut self) -> Option<&mut dyn LocalInput> {
        self.called("local_input", ());
        Some(self)
    }
    fn access_events(&mut self) -> Option<&mut dyn AccessEvents> {
        self.called("access_events", ());
        Some(self)
    }
    fn life_safety(&mut self) -> Option<&mut dyn LifeSafety> {
        self.called("life_safety", ());
        Some(self)
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.called("event_reporting", ());
        Some(self)
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        self.called("event_reporting_mut", ());
        Some(self)
    }
    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        self.called("enrollment_eval", ());
        Some(self)
    }
    fn enrollment_eval_mut(&mut self) -> Option<&mut dyn EnrollmentEval> {
        self.called("enrollment_eval_mut", ());
        Some(self)
    }
    fn reliability_eval(&mut self) -> Option<&mut dyn ReliabilityEval> {
        self.called("reliability_eval", ());
        Some(self)
    }
    fn schedule_runner(&mut self) -> Option<&mut dyn ScheduleRunner> {
        self.called("schedule_runner", ());
        Some(self)
    }
    fn calendar_state(&self) -> Option<&dyn CalendarState> {
        self.called("calendar_state", ());
        Some(self)
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        self.called("cov", ());
        Some(self)
    }
    fn audit_log(&self) -> Option<&dyn AuditLog> {
        self.called("audit_log", ());
        Some(self)
    }
    fn audit_log_mut(&mut self) -> Option<&mut dyn AuditLog> {
        self.called("audit_log_mut", ());
        Some(self)
    }
    fn durable_writes(&mut self) -> Option<&mut dyn DurableWrites> {
        self.called("durable_writes", ());
        self.audit_log.durable_writes()
    }
    fn file_configuration(&self) -> Option<&dyn FileConfiguration> {
        self.called("file_configuration", ());
        Some(&self.file)
    }
    fn file_configuration_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        self.called("file_configuration_mut", ());
        Some(&mut self.file)
    }
    fn file_storage(&self) -> Option<&dyn FileStorage> {
        self.called("file_storage", ());
        Some(&self.file)
    }
    fn file_storage_mut(&mut self) -> Option<&mut dyn FileStorage> {
        self.called("file_storage_mut", ());
        Some(&mut self.file)
    }
    fn log_buffer(&self) -> Option<&dyn LogBuffer> {
        self.called("log_buffer", ());
        Some(self)
    }
    fn log_buffer_mut(&mut self) -> Option<&mut dyn LogBuffer> {
        self.called("log_buffer_mut", ());
        Some(self)
    }
}

/// The probe's time capability: every method answers unlike its default
/// and logs its call under `Timed::`.
impl Timed for Probe {
    fn bind_clocks(&mut self, clocks: &Clocks) {
        let bound = (
            clocks.clock.as_deref().map(address),
            clocks.monotonic.as_deref().map(address),
            clocks.deadline_waker.as_deref().map(address),
        );
        self.called("Timed::bind_clocks", bound);
    }
    fn advance_monotonic_time(&mut self, now: Duration) -> bool {
        self.called("Timed::advance_monotonic_time", (now,));
        true
    }
    fn next_monotonic_deadline(&self) -> Option<Duration> {
        self.called("Timed::next_monotonic_deadline", ());
        Some(Duration::from_secs(99))
    }
    fn set_tracking_cov_increment(&mut self, finest: Option<f64>) {
        self.called("Timed::set_tracking_cov_increment", (finest,));
    }
    fn start_output(&mut self) {
        self.called("Timed::start_output", ());
    }
    fn retire(&mut self, retirement: bacnet_objects::traits::Retirement) {
        self.called("Timed::retire", (retirement,));
    }
    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        self.called("Timed::cov_snapshot", ());
        Some(Box::new(
            AuditReporterObject::new(18, "Probe snapshot").unwrap(),
        ))
    }
    fn take_due_sample(&mut self, now: Duration) -> Option<BACnetObjectPropertyReference> {
        self.called("Timed::take_due_sample", (now,));
        Some(BACnetObjectPropertyReference {
            object_identifier: oid(ObjectType::ANALOG_VALUE, 41),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: Some(2),
        })
    }
}

/// The probe's Staging capability, logged under `StagingSource::`.
impl StagingSource for Probe {
    fn take_staging_write_plan(&mut self) -> Option<StagingWritePlan> {
        self.called("StagingSource::take_staging_write_plan", ());
        Some(StagingWritePlan {
            source: self.oid,
            generation: 7,
            priority: 9,
            writes: Vec::new(),
        })
    }
    fn staging_generation(&self) -> u64 {
        self.called("StagingSource::staging_generation", ());
        7
    }
    fn complete_staging_write_plan(&mut self, generation: u64, success: bool) -> bool {
        self.called(
            "StagingSource::complete_staging_write_plan",
            (generation, success),
        );
        true
    }
    fn take_owed_relinquishes(&mut self) -> Vec<bacnet_objects::schedule::ScheduleWrite> {
        self.called("StagingSource::take_owed_relinquishes", ());
        vec![bacnet_objects::schedule::ScheduleWrite::new(
            PropertyValue::Null,
            10,
            Vec::new(),
        )]
    }
    fn take_relinquish_writes(&mut self) -> Vec<bacnet_objects::schedule::ScheduleWrite> {
        self.called("StagingSource::take_relinquish_writes", ());
        vec![bacnet_objects::schedule::ScheduleWrite::new(
            PropertyValue::Null,
            9,
            Vec::new(),
        )]
    }
}

/// The probe's command-run capability, logged under `CommandRunner::`.
impl CommandRunner for Probe {
    fn take_command_run(&mut self) -> Option<CommandRun> {
        self.called("CommandRunner::take_command_run", ());
        Some(CommandRun {
            source: self.oid,
            generation: 11,
            plan: RunPlan::Actions(Vec::new()),
            chain: std::sync::Arc::from([]),
        })
    }
    fn command_generation(&self) -> u64 {
        self.called("CommandRunner::command_generation", ());
        11
    }
    fn record_command_write(&mut self, generation: u64, command: usize, success: bool) -> bool {
        self.called(
            "CommandRunner::record_command_write",
            (generation, command, success),
        );
        true
    }
    fn remember_member_datatype(
        &mut self,
        slot: usize,
        reference: &bacnet_types::constructed::BACnetDeviceObjectPropertyReference,
        datatype: Option<bacnet_objects::channel::MemberDatatype>,
    ) {
        self.called(
            "CommandRunner::remember_member_datatype",
            (slot, reference.clone(), datatype),
        );
    }
    fn complete_command_run(&mut self, generation: u64, outcome: Result<(), WriteFailure>) -> bool {
        self.called("CommandRunner::complete_command_run", (generation, outcome));
        true
    }
}

/// The probe's local input capability, logged under `LocalInput::`.
impl LocalInput for Probe {
    fn apply_local_input(&mut self, input: LocalInputValue) -> Result<(), Error> {
        self.called("LocalInput::apply_local_input", (input,));
        Err(property_error(ErrorCode::VALUE_OUT_OF_RANGE))
    }
}

/// The probe's access-event capability, logged under `AccessEvents::`.
impl AccessEvents for Probe {
    fn record_access_event_batch(
        &mut self,
        reports: &[AccessEventReport],
    ) -> Result<Vec<CommittedAccessEvent>, Error> {
        access_batch::record(self, reports)
    }
}

/// The probe's Life Safety capability, logged under `LifeSafety::`: every
/// method answers unlike its default.
impl LifeSafety for Probe {
    fn apply_life_safety_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        self.called("LifeSafety::apply_life_safety_operation", (operation,));
        Ok(LifeSafetyOperationOutcome {
            effect: LifeSafetyOperationEffect::Applied,
            changed_properties: vec![P::SILENCED],
        })
    }
    fn set_life_safety_operation_expected(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.called(
            "LifeSafety::set_life_safety_operation_expected",
            (operation,),
        );
        Ok(())
    }
}

/// The probe's COV reporting, logged under `CovReporting::`: every method
/// answers unlike its default.
impl CovReporting for Probe {
    fn supports_subscribe_cov(&self) -> bool {
        self.called("CovReporting::supports_subscribe_cov", ());
        false
    }
    fn supports_cov_property(&self, property: P) -> bool {
        self.called("CovReporting::supports_cov_property", (property,));
        // Unlike the default, which takes every property.
        property == CUSTOM
    }
    fn cov_increment(&self) -> Option<f64> {
        self.called("CovReporting::cov_increment", ());
        Some(1.25)
    }
    fn cov_reported_properties(&self) -> Option<&'static [CovReportedProperty]> {
        self.called("CovReporting::cov_reported_properties", ());
        Some(&REPORTED)
    }
}

/// What the probe's log-record hooks answer: an error of the wrapped
/// object's own, unlike the default's OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
fn own_record_error() -> Error {
    Error::protocol(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
}

/// The probe's event reporting, logged under `EventReporting::`: every
/// method answers unlike its default.
impl EventReporting for Probe {
    fn evaluate_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.called("EventReporting::evaluate_intrinsic_reporting", ());
        Some(TransitionOutcome {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            event_type: EventType::OUT_OF_RANGE,
            distribute: true,
        })
    }
    fn tick_intrinsic_reporting(&mut self) -> Option<TransitionOutcome> {
        self.called("EventReporting::tick_intrinsic_reporting", ());
        Some(TransitionOutcome {
            change: EventStateChange {
                from: EventState::HIGH_LIMIT,
                to: EventState::NORMAL,
            },
            event_type: EventType::OUT_OF_RANGE,
            distribute: false,
        })
    }
    fn commit_event_transition(
        &mut self,
        commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        self.called("EventReporting::commit_event_transition", (commit,));
        Ok(())
    }
    fn acknowledge_alarm(
        &mut self,
        event_state: EventState,
        timestamp: &BACnetTimeStamp,
    ) -> Result<Option<EventStateChange>, Error> {
        let arguments = (event_state, timestamp);
        self.called("EventReporting::acknowledge_alarm", arguments);
        Ok(Some(EventStateChange {
            from: event_state,
            to: EventState::NORMAL,
        }))
    }
    fn enrollment_summary_capability(&self) -> Option<EnrollmentSummaryCapability> {
        self.called("EventReporting::enrollment_summary_capability", ());
        Some(EnrollmentSummaryCapability {
            event_type: EventType::CHANGE_OF_STATE,
            last_transition: None,
        })
    }
    fn buffer_ready_report(&self) -> Option<BufferReadyReport> {
        self.called("EventReporting::buffer_ready_report", ());
        Some(BufferReadyReport {
            previous_notification: 4,
            current_notification: 9,
        })
    }
    fn event_algorithm_inhibit_reference(&self) -> Option<BACnetObjectPropertyReference> {
        self.called("EventReporting::event_algorithm_inhibit_reference", ());
        Some(BACnetObjectPropertyReference {
            object_identifier: oid(ObjectType::BINARY_VALUE, 43),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: None,
        })
    }
    fn follow_event_algorithm_inhibit(&mut self, inhibit: bool) -> bool {
        self.called("EventReporting::follow_event_algorithm_inhibit", (inhibit,));
        true
    }
}
