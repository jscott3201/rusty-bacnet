//! Rows for the methods of each capability trait in
//! `bacnet-objects/src/traits/capabilities/` (#2179), named
//! `Trait::method` and called through the `BACnetObject` accessor that
//! lends the capability. The probe implements every capability, so an
//! adapter that keeps an accessor's `None` default, or lends a capability
//! of its own that drops a call or an argument, reads differently from the
//! probe.
//!
//! The capability traits are read from their source, as `BACnetObject` is.
//! A capability module added to `capabilities/mod.rs` fails
//! [`assert_rows_cover_the_capabilities`] until it is listed in `SOURCES`,
//! and a method added to a capability trait until it has a row here.

use super::declared;
use super::fixtures::{address, clocks, day, noon, oid, schedule_write};
use super::probe::CUSTOM;
use super::rows::{Row, commit, event_log_record, multiple_record, record};
use bacnet_objects::access_control::AccessEventReport;
use bacnet_objects::channel::MemberDatatype;
use bacnet_objects::command::WriteFailure;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentReliabilityCommit,
};
use bacnet_objects::schedule::ScheduleTargetOutcome;
use bacnet_objects::traits::LocalInputValue;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::{
    AccessEvent, AuditLevel, EventState, LifeSafetyOperation, ObjectType, PropertyIdentifier as P,
    Reliability,
};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use std::collections::BTreeSet;
use std::time::Duration;

const MODULE: &str = include_str!("../../../src/traits/capabilities/mod.rs");

/// Each capability module, by name, and its source.
const SOURCES: &[(&str, &str)] = &[
    (
        "access_events",
        include_str!("../../../src/traits/capabilities/access_events.rs"),
    ),
    (
        "audit_log",
        include_str!("../../../src/traits/capabilities/audit_log.rs"),
    ),
    (
        "audit_policy",
        include_str!("../../../src/traits/capabilities/audit_policy.rs"),
    ),
    (
        "calendar_state",
        include_str!("../../../src/traits/capabilities/calendar_state.rs"),
    ),
    (
        "command_runner",
        include_str!("../../../src/traits/capabilities/command_runner.rs"),
    ),
    (
        "cov_reporting",
        include_str!("../../../src/traits/capabilities/cov_reporting.rs"),
    ),
    (
        "enrollment_eval",
        include_str!("../../../src/traits/capabilities/enrollment_eval.rs"),
    ),
    (
        "event_reporting",
        include_str!("../../../src/traits/capabilities/event_reporting.rs"),
    ),
    (
        "life_safety",
        include_str!("../../../src/traits/capabilities/life_safety.rs"),
    ),
    (
        "local_input",
        include_str!("../../../src/traits/capabilities/local_input.rs"),
    ),
    (
        "log_buffer",
        include_str!("../../../src/traits/capabilities/log_buffer.rs"),
    ),
    (
        "reliability_eval",
        include_str!("../../../src/traits/capabilities/reliability_eval.rs"),
    ),
    (
        "schedule_runner",
        include_str!("../../../src/traits/capabilities/schedule_runner.rs"),
    ),
    (
        "staging_source",
        include_str!("../../../src/traits/capabilities/staging_source.rs"),
    ),
    (
        "timed",
        include_str!("../../../src/traits/capabilities/timed.rs"),
    ),
];

/// One row per capability method. A row through an accessor that takes
/// `&self` is a `Row::Query`, which a read-only view runs as well.
pub const CAPABILITY_ROWS: &[(&str, Row)] = &[
    (
        "Timed::bind_clocks",
        Row::Command(|o| format!("{:?}", o.timed().map(|t| t.bind_clocks(&clocks())))),
    ),
    (
        "Timed::advance_monotonic_time",
        Row::Command(|o| {
            let now = Duration::from_secs(33);
            format!("{:?}", o.timed().map(|t| t.advance_monotonic_time(now)))
        }),
    ),
    (
        "Timed::next_monotonic_deadline",
        Row::Command(|o| format!("{:?}", o.timed().map(|t| t.next_monotonic_deadline()))),
    ),
    (
        "Timed::set_tracking_cov_increment",
        Row::Command(|o| {
            let finest = Some(0.25);
            format!(
                "{:?}",
                o.timed().map(|t| t.set_tracking_cov_increment(finest))
            )
        }),
    ),
    (
        "Timed::start_output",
        Row::Command(|o| format!("{:?}", o.timed().map(|t| t.start_output()))),
    ),
    (
        "Timed::retire",
        Row::Command(|o| format!("{:?}", o.timed().map(|t| t.retire()))),
    ),
    (
        "Timed::cov_snapshot",
        Row::Command(|o| {
            let name = |t: &mut dyn bacnet_objects::traits::Timed| {
                t.cov_snapshot()
                    .map(|snapshot| snapshot.object_name().to_owned())
            };
            format!("{:?}", o.timed().map(name))
        }),
    ),
    (
        "Timed::take_due_sample",
        Row::Command(|o| {
            let now = Duration::from_secs(42);
            format!("{:?}", o.timed().map(|t| t.take_due_sample(now)))
        }),
    ),
    (
        "StagingSource::take_staging_write_plan",
        Row::Command(|o| {
            let plan = o.staging_source().map(|s| s.take_staging_write_plan());
            format!("{plan:?}")
        }),
    ),
    (
        "StagingSource::staging_generation",
        Row::Command(|o| format!("{:?}", o.staging_source().map(|s| s.staging_generation()))),
    ),
    (
        "StagingSource::complete_staging_write_plan",
        Row::Command(|o| {
            let completed = o
                .staging_source()
                .map(|s| s.complete_staging_write_plan(7, true));
            format!("{completed:?}")
        }),
    ),
    (
        "StagingSource::take_owed_relinquishes",
        Row::Command(|o| {
            let writes = o.staging_source().map(|s| s.take_owed_relinquishes());
            format!("{writes:?}")
        }),
    ),
    (
        "StagingSource::take_relinquish_writes",
        Row::Command(|o| {
            let writes = o.staging_source().map(|s| s.take_relinquish_writes());
            format!("{writes:?}")
        }),
    ),
    (
        "CommandRunner::take_command_run",
        Row::Command(|o| format!("{:?}", o.command_runner_mut().map(|r| r.take_command_run()))),
    ),
    // Through the shared accessor, the one a host's read guard lends, so a
    // read-only view runs it as well.
    (
        "CommandRunner::command_generation",
        Row::Query(|o| format!("{:?}", o.command_runner().map(|r| r.command_generation()))),
    ),
    (
        "CommandRunner::record_command_write",
        Row::Command(|o| {
            let recorded = o
                .command_runner_mut()
                .map(|r| r.record_command_write(11, 2, false));
            format!("{recorded:?}")
        }),
    ),
    (
        "CommandRunner::remember_member_datatype",
        Row::Command(|o| {
            let reference = BACnetDeviceObjectPropertyReference::new_local(
                oid(ObjectType::BINARY_OUTPUT, 4),
                P::PRESENT_VALUE.to_raw(),
            );
            let datatype = Some(MemberDatatype::Enumerated);
            let remembered = o
                .command_runner_mut()
                .map(|r| r.remember_member_datatype(3, &reference, datatype));
            format!("{remembered:?}")
        }),
    ),
    (
        "CommandRunner::complete_command_run",
        Row::Command(|o| {
            let outcome = Err(WriteFailure::Communication);
            let completed = o
                .command_runner_mut()
                .map(|r| r.complete_command_run(11, outcome));
            format!("{completed:?}")
        }),
    ),
    (
        "LocalInput::apply_local_input",
        Row::Command(|o| {
            let origin = CommandOrigin::Local {
                owner_device: oid(ObjectType::DEVICE, 9),
                initiating_object: None,
            };
            let input = LocalInputValue::present_value_from(PropertyValue::Real(22.5), origin);
            format!("{:?}", o.local_input().map(|i| i.apply_local_input(input)))
        }),
    ),
    (
        "AccessEvents::record_access_event_batch",
        Row::Command(|o| {
            let reports = [
                AccessEventReport {
                    time: Some(BACnetTimeStamp::SequenceNumber(12)),
                    ..AccessEventReport::new(AccessEvent::GRANTED, 41)
                },
                AccessEventReport {
                    time: Some(BACnetTimeStamp::SequenceNumber(13)),
                    ..AccessEventReport::new(AccessEvent::DENIED_OTHER, 42)
                },
            ];
            let committed = o.access_events().map(|events| {
                events.record_access_event_batch(&reports).map(|events| {
                    events
                        .into_iter()
                        .map(|event| {
                            let tag = event.snapshot.read_property(P::ACCESS_EVENT_TAG, None);
                            let name = event.snapshot.object_name().to_owned();
                            (event.report, name, tag.unwrap(), event.clock_frame)
                        })
                        .collect::<Vec<_>>()
                })
            });
            format!("{committed:?}")
        }),
    ),
    // Through the one shared accessor, so a read-only view runs them too.
    (
        "CovReporting::supports_subscribe_cov",
        Row::Query(|o| format!("{:?}", o.cov().map(|c| c.supports_subscribe_cov()))),
    ),
    (
        "CovReporting::supports_cov_property",
        Row::Query(|o| {
            let supported = o
                .cov()
                .map(|c| [CUSTOM, P::DESCRIPTION].map(|p| c.supports_cov_property(p)));
            format!("{supported:?}")
        }),
    ),
    (
        "CovReporting::cov_increment",
        Row::Query(|o| format!("{:?}", o.cov().map(|c| c.cov_increment()))),
    ),
    (
        "CovReporting::cov_reported_properties",
        Row::Query(|o| format!("{:?}", o.cov().map(|c| c.cov_reported_properties()))),
    ),
    (
        "EventReporting::evaluate_intrinsic_reporting",
        Row::Command(|o| {
            let outcome = o
                .event_reporting_mut()
                .map(|r| r.evaluate_intrinsic_reporting());
            format!("{outcome:?}")
        }),
    ),
    (
        "EventReporting::tick_intrinsic_reporting",
        Row::Command(|o| {
            let outcome = o
                .event_reporting_mut()
                .map(|r| r.tick_intrinsic_reporting());
            format!("{outcome:?}")
        }),
    ),
    (
        "EventReporting::commit_event_transition",
        Row::Command(|o| {
            let committed = o
                .event_reporting_mut()
                .map(|r| r.commit_event_transition(commit()));
            format!("{committed:?}")
        }),
    ),
    (
        "EventReporting::acknowledge_alarm",
        Row::Command(|o| {
            let timestamp = BACnetTimeStamp::SequenceNumber(42);
            let acknowledged = o
                .event_reporting_mut()
                .map(|r| r.acknowledge_alarm(EventState::HIGH_LIMIT, &timestamp));
            format!("{acknowledged:?}")
        }),
    ),
    // The two queries go through the shared accessor, the one a host's read
    // guard lends, so a read-only view runs them as well.
    (
        "EventReporting::enrollment_summary_capability",
        Row::Query(|o| {
            let capability = o
                .event_reporting()
                .map(|r| r.enrollment_summary_capability());
            format!("{capability:?}")
        }),
    ),
    (
        "EventReporting::buffer_ready_report",
        Row::Query(|o| {
            let report = o.event_reporting().map(|r| r.buffer_ready_report());
            format!("{report:?}")
        }),
    ),
    (
        "EventReporting::event_algorithm_inhibit_reference",
        Row::Query(|o| {
            let reference = o
                .event_reporting()
                .map(|r| r.event_algorithm_inhibit_reference());
            format!("{reference:?}")
        }),
    ),
    (
        "EventReporting::follow_event_algorithm_inhibit",
        Row::Command(|o| {
            let changed = o
                .event_reporting_mut()
                .map(|r| r.follow_event_algorithm_inhibit(true));
            format!("{changed:?}")
        }),
    ),
    (
        "LifeSafety::apply_life_safety_operation",
        Row::Command(|o| {
            let operation = LifeSafetyOperation::SILENCE;
            let outcome = o
                .life_safety()
                .map(|l| l.apply_life_safety_operation(operation));
            format!("{outcome:?}")
        }),
    ),
    (
        "LifeSafety::set_life_safety_operation_expected",
        Row::Command(|o| {
            let operation = LifeSafetyOperation::SILENCE;
            let set = o
                .life_safety()
                .map(|l| l.set_life_safety_operation_expected(operation));
            format!("{set:?}")
        }),
    ),
    // The two snapshots go through the shared accessor, the one a host's
    // read guard lends, so a read-only view runs them as well.
    (
        "EnrollmentEval::eval_state",
        Row::Query(|o| format!("{:?}", o.enrollment_eval().map(|e| e.eval_state()))),
    ),
    (
        "EnrollmentEval::set_eval_state",
        Row::Command(|o| {
            let state = EventEnrollmentEvalState {
                last_offnormal_value: Some(3),
                ..EventEnrollmentEvalState::default()
            };
            let set = o.enrollment_eval_mut().map(|e| e.set_eval_state(state));
            format!("{set:?}")
        }),
    ),
    (
        "EnrollmentEval::eval_source",
        Row::Query(|o| format!("{:?}", o.enrollment_eval().map(|e| e.eval_source()))),
    ),
    (
        "EnrollmentEval::set_eval_source",
        Row::Command(|o| {
            let source = (oid(ObjectType::ANALOG_INPUT, 2), P::PRESENT_VALUE, None);
            let set = o
                .enrollment_eval_mut()
                .map(|e| e.set_eval_source(Some(source)));
            format!("{set:?}")
        }),
    ),
    (
        "EnrollmentEval::commit_reliability",
        Row::Command(|o| {
            let commit = EventEnrollmentReliabilityCommit {
                reliability: Reliability::OVER_RANGE,
                transition: Some(commit()),
            };
            let committed = o
                .enrollment_eval_mut()
                .map(|e| e.commit_reliability(commit));
            format!("{committed:?}")
        }),
    ),
    (
        "ReliabilityEval::evaluate_reliability",
        Row::Command(|o| {
            let evaluated = o.reliability_eval().map(|r| r.evaluate_reliability());
            format!("{evaluated:?}")
        }),
    ),
    (
        "ReliabilityEval::reliability_evaluation_inhibited",
        Row::Command(|o| {
            let inhibited = o
                .reliability_eval()
                .map(|r| r.reliability_evaluation_inhibited());
            format!("{inhibited:?}")
        }),
    ),
    (
        "ReliabilityEval::set_reliability",
        Row::Command(|o| {
            let set = o
                .reliability_eval()
                .map(|r| r.set_reliability(Reliability::OVER_RANGE));
            format!("{set:?}")
        }),
    ),
    (
        "ScheduleRunner::tick_schedule",
        Row::Command(|o| {
            let calendars = |calendar: ObjectIdentifier| calendar.instance_number() == 4;
            let write = o
                .schedule_runner()
                .map(|r| r.tick_schedule(day(), noon(), &calendars));
            format!("{write:?}")
        }),
    ),
    (
        "ScheduleRunner::take_owed_schedule_writes",
        Row::Command(|o| {
            let owed = o.schedule_runner().map(|r| r.take_owed_schedule_writes());
            format!("{owed:?}")
        }),
    ),
    (
        "ScheduleRunner::complete_schedule_write",
        Row::Command(|o| {
            let outcomes = [ScheduleTargetOutcome::DatatypeRefused];
            let changed = o
                .schedule_runner()
                .map(|r| r.complete_schedule_write(&schedule_write(2), &outcomes));
            format!("{changed:?}")
        }),
    ),
    (
        "ScheduleRunner::retry_refusals_naming",
        Row::Command(|o| {
            let target = oid(ObjectType::ANALOG_VALUE, 9);
            let retry = o.schedule_runner().map(|r| r.retry_refusals_naming(target));
            format!("{retry:?}")
        }),
    ),
    (
        "ScheduleRunner::take_relinquish_writes",
        Row::Command(|o| {
            let writes = o.schedule_runner().map(|r| r.take_relinquish_writes());
            format!("{writes:?}")
        }),
    ),
    // The two reads go through the shared accessor, the one ReadRange's
    // read guard lends, so a read-only view runs them as well.
    (
        "LogBuffer::record_identities",
        Row::Query(|o| format!("{:?}", o.log_buffer().map(|l| l.record_identities()))),
    ),
    (
        "LogBuffer::buffer_records",
        Row::Query(|o| {
            let records = o.log_buffer().map(|l| l.buffer_records().map(address));
            format!("{records:?}")
        }),
    ),
    (
        "LogBuffer::add_trend_record",
        Row::Command(|o| {
            let added = o.log_buffer_mut().map(|l| l.add_trend_record(record()));
            format!("{added:?}")
        }),
    ),
    (
        "LogBuffer::add_trend_multiple_record",
        Row::Command(|o| {
            let record = multiple_record();
            let added = o
                .log_buffer_mut()
                .map(|l| l.add_trend_multiple_record(record));
            format!("{added:?}")
        }),
    ),
    (
        "LogBuffer::add_event_log_record",
        Row::Command(|o| {
            let record = event_log_record();
            let added = o.log_buffer_mut().map(|l| l.add_event_log_record(record));
            format!("{added:?}")
        }),
    ),
    (
        "LogBuffer::refresh_window",
        Row::Command(|o| format!("{:?}", o.log_buffer_mut().map(|l| l.refresh_window()))),
    ),
    // The two reads go through the shared accessor, the one AuditLogQuery's
    // and the forwarder's read guards lend, so a read-only view runs them.
    (
        "AuditLog::storage",
        Row::Query(|o| format!("{:?}", o.audit_log().map(|l| address(l.storage())))),
    ),
    (
        "AuditLog::forwarding",
        Row::Query(|o| {
            let forwarding = o.audit_log().map(|l| l.forwarding().map(|f| address(&*f)));
            format!("{forwarding:?}")
        }),
    ),
    (
        "AuditLog::set_parent",
        Row::Command(|o| {
            let parent = BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: oid(ObjectType::DEVICE, 9),
            };
            format!("{:?}", o.audit_log_mut().map(|l| l.set_parent(parent)))
        }),
    ),
    (
        "AuditLog::notification_sink",
        Row::Command(|o| {
            let sink = o
                .audit_log_mut()
                .map(|l| l.notification_sink().map(|sink| address(sink)));
            format!("{sink:?}")
        }),
    ),
    // Through the shared accessor, the one the Reporter's read guard lends,
    // so a read-only view runs it as well.
    (
        "AuditPolicy::object_policy",
        Row::Query(|o| format!("{:?}", o.audit_policy().map(|p| p.object_policy()))),
    ),
    (
        "AuditPolicy::policy_authority",
        Row::Command(|o| {
            // The probe's own policy level is NONE, so raising it prepares a
            // write.
            let level = PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw());
            let prepared = o.audit_policy_mut().map(|p| {
                p.policy_authority().map(|policy| {
                    let write = policy.prepare(P::AUDIT_LEVEL, None, &level, None);
                    write.map(|write| write.is_some())
                })
            });
            format!("{prepared:?}")
        }),
    ),
    // Through the shared accessor, the one a host's read guard lends, so a
    // read-only view runs it as well.
    (
        "CalendarState::state_on",
        Row::Query(|o| format!("{:?}", o.calendar_state().map(|c| c.state_on(day())))),
    ),
];

/// Every capability trait's methods, as `Trait::method`, read from its
/// source; a trait's methods follow its `pub trait` line at four spaces.
fn capability_methods() -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for (module, source) in SOURCES {
        let mut traits = 0;
        for (start, _) in source.match_indices("\npub trait ") {
            traits += 1;
            let body = &source[start + 1..];
            let body = &body[..body.find("\n}\n").expect("trait closes")];
            let name = body["pub trait ".len()..]
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .expect("trait name");
            for (offset, marker) in body.match_indices("\n    fn ") {
                let method = declared(&body[offset + marker.len()..]);
                methods.insert(format!("{name}::{}", method.name));
            }
            assert!(
                super::TRAIT_SOURCE.contains(&format!("Option<&mut dyn {name}>"))
                    || super::TRAIT_SOURCE.contains(&format!("Option<&dyn {name}>")),
                "capability {name} has no accessor on BACnetObject"
            );
        }
        assert!(
            traits > 0,
            "capabilities/{module}.rs declares no capability"
        );
    }
    methods
}

/// Every capability module is read, and each capability method has one
/// row.
pub fn assert_rows_cover_the_capabilities() {
    let declared_modules: BTreeSet<_> = MODULE
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let read: BTreeSet<_> = SOURCES.iter().map(|(module, _)| *module).collect();
    assert_eq!(
        declared_modules, read,
        "list every module of bacnet-objects/src/traits/capabilities in SOURCES in \
         bacnet-objects/tests/support/forwarding/capabilities.rs"
    );
    let named: Vec<_> = CAPABILITY_ROWS.iter().map(|row| row.0.to_owned()).collect();
    let rows: BTreeSet<_> = named.iter().cloned().collect();
    assert_eq!(rows.len(), named.len(), "a capability method has two rows");
    let methods = capability_methods();
    let missing: Vec<_> = methods.difference(&rows).collect();
    let unknown: Vec<_> = rows.difference(&methods).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "CAPABILITY_ROWS in bacnet-objects/tests/support/forwarding/capabilities.rs does \
         not match the capability traits: no row for {missing:?}, no such method for \
         {unknown:?}. Give a new method a probe answer and a row, and forward its \
         accessor in SourceReporter and, if it takes &self, in DeviceReadView"
    );
}
