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
use super::fixtures::{clocks, oid};
use super::rows::Row;
use bacnet_objects::access_control::AccessEventReport;
use bacnet_objects::channel::MemberDatatype;
use bacnet_objects::command::WriteFailure;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::traits::LocalInputValue;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{AccessEvent, LifeSafetyOperation, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};
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
        "command_runner",
        include_str!("../../../src/traits/capabilities/command_runner.rs"),
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
