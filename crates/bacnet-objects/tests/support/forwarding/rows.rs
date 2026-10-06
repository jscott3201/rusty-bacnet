//! One row per `BACnetObject` method: a call with fixed arguments, and its
//! answer rendered as text. Capabilities render as the address they borrow.

use super::fixtures::{address, clock, day, monotonic, noon, oid, schedule_write, waker};
use super::probe::{CUSTOM, CUSTOM_LIST};
use bacnet_objects::access_control::{AccessControlInput, DoorStateReport};
use bacnet_objects::accumulator::InputReading;
use bacnet_objects::command::WriteFailure;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentReliabilityCommit,
};
use bacnet_objects::schedule::ScheduleTargetOutcome;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetDeviceObjectReference, BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord,
    EventLogDatum, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{
    AuditLevel, DoorStatus, EventState, LifeSafetyOperation, ObjectType, PropertyIdentifier as P,
    Reliability,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, PropertyValue};
use std::time::Duration;

/// A row for a method that takes `&self`.
pub type Query = fn(&dyn BACnetObject) -> String;
/// A row for a method that takes `&mut self`.
pub type Command = fn(&mut dyn BACnetObject) -> String;

/// Either kind of row.
#[derive(Clone, Copy)]
pub enum Row {
    Query(Query),
    Command(Command),
}

impl Row {
    pub fn call(self, object: &mut dyn BACnetObject) -> String {
        match self {
            Self::Query(query) => query(object),
            Self::Command(command) => command(object),
        }
    }
}

/// Every row, the queries first.
pub fn rows() -> impl Iterator<Item = (&'static str, Row)> {
    let queries = QUERIES
        .iter()
        .map(|&(name, query)| (name, Row::Query(query)));
    let commands = COMMANDS
        .iter()
        .map(|&(name, command)| (name, Row::Command(command)));
    queries.chain(commands)
}

fn commit() -> EventTransitionCommit {
    EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::HIGH_LIMIT,
        },
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        message_text: Some("probe commit".into()),
    }
}

fn multiple_record() -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: record().date,
        time: noon(),
        log_data: LogData::Values(vec![LogValue::UnsignedValue(77), LogValue::NullValue]),
    }
}

fn record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        time: noon(),
        log_datum: LogDatum::UnsignedValue(77),
        status_flags: None,
    }
}

fn event_log_record() -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        time: noon(),
        log_datum: EventLogDatum::TimeChange(1.5),
    }
}

pub const QUERIES: &[(&str, Query)] = &[
    ("audit_object_policy_internal", |o| {
        format!("{:?}", o.audit_object_policy_internal())
    }),
    ("audit_reporter_internal", |o| {
        format!("{:?}", o.audit_reporter_internal().map(address))
    }),
    ("object_identifier", |o| {
        format!("{:?}", o.object_identifier())
    }),
    ("object_name", |o| o.object_name().to_owned()),
    ("read_property", |o| {
        format!("{:?}", o.read_property(CUSTOM, Some(3)))
    }),
    ("property_metadata", |o| {
        format!("{:?}", o.property_metadata())
    }),
    ("property_list", |o| format!("{:?}", o.property_list())),
    ("next_monotonic_deadline_internal", |o| {
        format!("{:?}", o.next_monotonic_deadline_internal())
    }),
    ("cov_snapshot_internal", |o| {
        format!(
            "{:?}",
            o.cov_snapshot_internal()
                .map(|snapshot| snapshot.object_name().to_owned())
        )
    }),
    ("lighting_blink_count_internal", |o| {
        o.lighting_blink_count_internal().to_string()
    }),
    ("is_writable_property", |o| {
        format!(
            "{:?}",
            [CUSTOM, P::DESCRIPTION].map(|p| o.is_writable_property(p))
        )
    }),
    ("is_array_property", |o| {
        format!(
            "{:?}",
            [CUSTOM, P::PRIORITY_ARRAY].map(|p| o.is_array_property(p))
        )
    }),
    ("is_list_property", |o| {
        format!(
            "{:?}",
            [CUSTOM_LIST, P::DATE_LIST].map(|p| o.is_list_property(p))
        )
    }),
    ("is_createable", |o| o.is_createable().to_string()),
    ("creation_only_properties", |o| {
        format!("{:?}", o.creation_only_properties())
    }),
    ("is_deleteable", |o| o.is_deleteable().to_string()),
    ("required_properties", |o| {
        format!("{:?}", o.required_properties())
    }),
    ("supports_cov", |o| o.supports_cov().to_string()),
    // Rendered beside `supports_cov`: the probe's false must differ both from
    // an all-default object (false, false) and from an adapter that kept the
    // default, which follows the forwarded `supports_cov` (true, true).
    ("supports_subscribe_cov_property", |o| {
        format!(
            "{:?}",
            [o.supports_cov(), o.supports_subscribe_cov_property()]
        )
    }),
    ("staging_generation_internal", |o| {
        format!("{:?}", o.staging_generation_internal())
    }),
    ("command_generation_internal", |o| {
        format!("{:?}", o.command_generation_internal())
    }),
    ("enrollment_summary_capability_internal", |o| {
        format!("{:?}", o.enrollment_summary_capability_internal())
    }),
    ("supports_cov_property", |o| {
        format!(
            "{:?}",
            [CUSTOM, P::DESCRIPTION].map(|p| o.supports_cov_property(p))
        )
    }),
    ("cov_increment", |o| format!("{:?}", o.cov_increment())),
    ("cov_reported_properties", |o| {
        format!("{:?}", o.cov_reported_properties())
    }),
    ("calendar_state_internal", |o| {
        format!("{:?}", o.calendar_state_internal(day()))
    }),
    ("retry_refusals_naming", |o| {
        format!(
            "{:?}",
            o.retry_refusals_naming(oid(ObjectType::ANALOG_VALUE, 9))
        )
    }),
    ("enrollment_eval_state_internal", |o| {
        format!("{:?}", o.enrollment_eval_state_internal())
    }),
    ("enrollment_eval_source_internal", |o| {
        format!("{:?}", o.enrollment_eval_source_internal())
    }),
    ("input_reference_internal", |o| {
        format!("{:?}", o.input_reference_internal())
    }),
    ("reliability_evaluation_inhibited_internal", |o| {
        o.reliability_evaluation_inhibited_internal().to_string()
    }),
    ("audit_log_storage_internal", |o| {
        format!("{:?}", o.audit_log_storage_internal().map(address))
    }),
    ("audit_log_forwarding_internal", |o| {
        format!(
            "{:?}",
            o.audit_log_forwarding_internal()
                .map(|forwarding| address(forwarding.as_ref()))
        )
    }),
    ("file_configuration_internal", |o| {
        format!("{:?}", o.file_configuration_internal().map(address))
    }),
    ("file_storage_internal", |o| {
        format!("{:?}", o.file_storage_internal().map(address))
    }),
    ("log_record_identities_internal", |o| {
        format!("{:?}", o.log_record_identities_internal())
    }),
    ("log_buffer_internal", |o| {
        format!("{:?}", o.log_buffer_internal().map(address))
    }),
    ("logs_received_event_notifications_internal", |o| {
        o.logs_received_event_notifications_internal().to_string()
    }),
    ("buffer_ready_report_internal", |o| {
        format!("{:?}", o.buffer_ready_report_internal())
    }),
    ("event_algorithm_inhibit_reference_internal", |o| {
        format!("{:?}", o.event_algorithm_inhibit_reference_internal())
    }),
];

pub const COMMANDS: &[(&str, Command)] = &[
    ("device_authority_internal", |o| {
        format!(
            "{:?}",
            o.device_authority_internal()
                .map(|device| device.object_identifier())
        )
    }),
    ("audit_policy_authority_internal", |o| {
        // The probe's policy level is NONE, so raising it prepares a write.
        let level = PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw());
        let prepared = o.audit_policy_authority_internal().map(|policy| {
            let write = policy.prepare(P::AUDIT_LEVEL, None, &level, None);
            write.map(|write| write.is_some())
        });
        format!("{prepared:?}")
    }),
    ("audit_reporter_authority_internal", |o| {
        format!(
            "{:?}",
            o.audit_reporter_authority_internal()
                .map(|reporter| reporter.object_identifier())
        )
    }),
    ("configure_audit_reporter_internal", |o| {
        format!(
            "{:?}",
            o.configure_audit_reporter_internal(
                AuditLevel::AUDIT_CONFIG,
                AuditOperationFlags::from_bits(1 << 1).unwrap(),
                true,
                None,
                BACnetPriorityFilter::from_bits(1 << 7),
                None,
            )
        )
    }),
    ("write_property", |o| {
        format!(
            "{:?}",
            o.write_property(CUSTOM, Some(3), PropertyValue::Unsigned(42), Some(7))
        )
    }),
    ("write_property_from", |o| {
        let origin = CommandOrigin::Local {
            owner_device: oid(ObjectType::DEVICE, 9),
            initiating_object: Some(oid(ObjectType::SCHEDULE, 4)),
        };
        format!(
            "{:?}",
            o.write_property_from(
                CUSTOM,
                Some(3),
                PropertyValue::Unsigned(42),
                Some(7),
                &origin
            )
        )
    }),
    ("initialize_property", |o| {
        let value = PropertyValue::CharacterString("probe".into());
        format!("{:?}", o.initialize_property(CUSTOM, value))
    }),
    ("bind_clock_internal", |o| {
        format!("{:?}", o.bind_clock_internal(Some(clock())))
    }),
    ("advance_time_internal", |o| {
        o.advance_time_internal(Duration::from_secs(5)).to_string()
    }),
    ("bind_monotonic_clock_internal", |o| {
        format!("{:?}", o.bind_monotonic_clock_internal(Some(monotonic())))
    }),
    ("bind_deadline_waker_internal", |o| {
        format!("{:?}", o.bind_deadline_waker_internal(Some(waker())))
    }),
    ("advance_monotonic_time_internal", |o| {
        o.advance_monotonic_time_internal(Duration::from_secs(33))
            .to_string()
    }),
    ("set_tracking_cov_increment_internal", |o| {
        format!("{:?}", o.set_tracking_cov_increment_internal(Some(0.25)))
    }),
    ("take_staging_write_plan_internal", |o| {
        format!("{:?}", o.take_staging_write_plan_internal())
    }),
    ("complete_staging_write_plan_internal", |o| {
        o.complete_staging_write_plan_internal(7, true).to_string()
    }),
    ("take_command_run_internal", |o| {
        format!("{:?}", o.take_command_run_internal())
    }),
    ("record_command_write_internal", |o| {
        o.record_command_write_internal(11, 2, false).to_string()
    }),
    ("remember_member_datatype_internal", |o| {
        let reference = bacnet_types::constructed::BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::BINARY_OUTPUT, 4),
            P::PRESENT_VALUE.to_raw(),
        );
        let datatype = Some(bacnet_objects::channel::MemberDatatype::Enumerated);
        format!(
            "{:?}",
            o.remember_member_datatype_internal(3, &reference, datatype)
        )
    }),
    ("complete_command_run_internal", |o| {
        o.complete_command_run_internal(11, Err(WriteFailure::Communication))
            .to_string()
    }),
    ("set_overridden", |o| {
        format!("{:?}", o.set_overridden(true))
    }),
    ("evaluate_intrinsic_reporting", |o| {
        format!("{:?}", o.evaluate_intrinsic_reporting())
    }),
    ("tick_intrinsic_reporting", |o| {
        format!("{:?}", o.tick_intrinsic_reporting())
    }),
    ("commit_event_transition_internal", |o| {
        format!("{:?}", o.commit_event_transition_internal(commit()))
    }),
    ("commit_event_enrollment_reliability_internal", |o| {
        let commit = EventEnrollmentReliabilityCommit {
            reliability: Reliability::OVER_RANGE,
            transition: Some(commit()),
        };
        format!(
            "{:?}",
            o.commit_event_enrollment_reliability_internal(commit)
        )
    }),
    ("tick_schedule", |o| {
        let calendars =
            |calendar: bacnet_types::primitives::ObjectIdentifier| calendar.instance_number() == 4;
        format!("{:?}", o.tick_schedule(day(), noon(), &calendars))
    }),
    ("take_owed_schedule_writes", |o| {
        format!("{:?}", o.take_owed_schedule_writes())
    }),
    ("complete_schedule_write", |o| {
        let outcomes = [ScheduleTargetOutcome::DatatypeRefused];
        o.complete_schedule_write(&schedule_write(2), &outcomes)
            .to_string()
    }),
    ("acknowledge_alarm", |o| {
        format!("{:?}", o.acknowledge_alarm(EventTransitionBits::TO_FAULT))
    }),
    ("acknowledge_alarm_correlated_internal", |o| {
        let timestamp = BACnetTimeStamp::SequenceNumber(42);
        format!(
            "{:?}",
            o.acknowledge_alarm_correlated_internal(EventState::HIGH_LIMIT, &timestamp)
        )
    }),
    ("acknowledge_alarm_correlated_detailed_internal", |o| {
        let timestamp = BACnetTimeStamp::SequenceNumber(42);
        format!(
            "{:?}",
            o.acknowledge_alarm_correlated_detailed_internal(EventState::HIGH_LIMIT, &timestamp)
        )
    }),
    ("apply_life_safety_operation", |o| {
        format!(
            "{:?}",
            o.apply_life_safety_operation(LifeSafetyOperation::SILENCE)
        )
    }),
    ("set_life_safety_operation_expected_internal", |o| {
        format!(
            "{:?}",
            o.set_life_safety_operation_expected_internal(LifeSafetyOperation::SILENCE)
        )
    }),
    ("set_event_state_internal", |o| {
        format!("{:?}", o.set_event_state_internal(EventState::OFFNORMAL))
    }),
    ("set_enrollment_eval_state_internal", |o| {
        format!(
            "{:?}",
            o.set_enrollment_eval_state_internal(EventEnrollmentEvalState::default())
        )
    }),
    ("set_enrollment_eval_source_internal", |o| {
        let source = (oid(ObjectType::ANALOG_INPUT, 2), P::PRESENT_VALUE, None);
        format!("{:?}", o.set_enrollment_eval_source_internal(Some(source)))
    }),
    ("set_acked_transitions_internal", |o| {
        format!(
            "{:?}",
            o.set_acked_transitions_internal(EventTransitionBits::TO_NORMAL, true)
        )
    }),
    ("evaluate_reliability_internal", |o| {
        format!("{:?}", o.evaluate_reliability_internal())
    }),
    ("set_reliability_internal", |o| {
        format!("{:?}", o.set_reliability_internal(Reliability::OVER_RANGE))
    }),
    ("set_present_value_internal", |o| {
        format!(
            "{:?}",
            o.set_present_value_internal(PropertyValue::Real(21.5))
        )
    }),
    ("set_present_value_from_internal", |o| {
        let origin = CommandOrigin::Local {
            owner_device: oid(ObjectType::DEVICE, 9),
            initiating_object: None,
        };
        format!(
            "{:?}",
            o.set_present_value_from_internal(PropertyValue::Real(22.5), &origin)
        )
    }),
    ("set_tracking_value_internal", |o| {
        format!(
            "{:?}",
            o.set_tracking_value_internal(PropertyValue::Unsigned(4))
        )
    }),
    ("report_access_input_internal", |o| {
        let report = DoorStateReport {
            door_status: Some(DoorStatus::OPENED),
            ..DoorStateReport::default()
        };
        format!(
            "{:?}",
            o.report_access_input_internal(AccessControlInput::DoorState(report))
        )
    }),
    ("set_controlled_variable_value_internal", |o| {
        format!(
            "{:?}",
            o.set_controlled_variable_value_internal(PropertyValue::Real(3.5))
        )
    }),
    ("add_averaging_sample_internal", |o| {
        format!(
            "{:?}",
            o.add_averaging_sample_internal(Some(PropertyValue::Real(3.0)))
        )
    }),
    ("take_due_averaging_sample_internal", |o| {
        format!(
            "{:?}",
            o.take_due_averaging_sample_internal(Duration::from_secs(42))
        )
    }),
    ("set_input_usable_internal", |o| {
        format!("{:?}", o.set_input_usable_internal(false))
    }),
    ("follow_event_algorithm_inhibit_internal", |o| {
        format!("{:?}", o.follow_event_algorithm_inhibit_internal(true))
    }),
    ("take_input_reading_internal", |o| {
        let reading = InputReading {
            value: -41,
            wraps_after: Some(99),
        };
        format!("{:?}", o.take_input_reading_internal(Some(reading)))
    }),
    ("set_audit_log_parent_internal", |o| {
        let parent = BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: oid(ObjectType::DEVICE, 9),
        };
        format!("{:?}", o.set_audit_log_parent_internal(parent))
    }),
    ("audit_log_notification_sink_internal", |o| {
        format!(
            "{:?}",
            o.audit_log_notification_sink_internal()
                .map(|sink| address(sink))
        )
    }),
    ("durable_writes_internal", |o| {
        format!(
            "{:?}",
            o.durable_writes_internal().map(|writes| address(writes))
        )
    }),
    ("file_configuration_internal_mut", |o| {
        format!(
            "{:?}",
            o.file_configuration_internal_mut()
                .map(|file| address(file))
        )
    }),
    ("file_storage_internal_mut", |o| {
        format!(
            "{:?}",
            o.file_storage_internal_mut().map(|file| address(file))
        )
    }),
    ("add_trend_record", |o| {
        format!("{:?}", o.add_trend_record(record()))
    }),
    ("add_trend_multiple_record", |o| {
        format!("{:?}", o.add_trend_multiple_record(multiple_record()))
    }),
    ("add_event_log_record", |o| {
        format!("{:?}", o.add_event_log_record(event_log_record()))
    }),
    ("refresh_log_window_internal", |o| {
        o.refresh_log_window_internal().to_string()
    }),
];
