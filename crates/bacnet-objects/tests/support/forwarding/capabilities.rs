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
use super::fixtures::clocks;
use super::rows::Row;
use std::collections::BTreeSet;
use std::time::Duration;

const MODULE: &str = include_str!("../../../src/traits/capabilities/mod.rs");

/// Each capability module, by name, and its source.
const SOURCES: &[(&str, &str)] = &[(
    "timed",
    include_str!("../../../src/traits/capabilities/timed.rs"),
)];

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
