//! Forwarding checks for the adapters that wrap a `BACnetObject` and hand
//! its methods to the object inside: SourceReporter in bacnet-endpoint
//! forwards every method, and DeviceReadView in bacnet-server every `&self`
//! query. Their unit tests include this directory with `#[path]`, so it
//! adds nothing to a published crate's API.
//!
//! The trait's methods are read from its source. A method added there fails
//! `assert_rows_cover_the_trait` until it has a probe answer in `probe.rs`
//! and a row in `rows.rs`. The probe answers every row unlike the trait
//! default and logs each call with its arguments, so an adapter that keeps a
//! default, drops an argument or swaps the answer reads differently from the
//! probe. Values the probe and the rows share, such as identifiers, dates
//! and clocks, are in `fixtures.rs`.

pub mod defaults;
pub mod fixtures;
pub mod probe;
pub mod rows;

use bacnet_types::enums::ObjectType;
use defaults::Defaults;
use probe::{CallLog, Probe, take};
use rows::{COMMANDS, QUERIES, rows};
use std::collections::BTreeSet;

const TRAIT_SOURCE: &str = include_str!("../../../src/traits.rs");

/// One `BACnetObject` method as the trait declares it.
pub struct TraitMethod {
    pub name: &'static str,
    /// Takes `&self`: a query a read-only view can forward.
    pub shared: bool,
    /// Has no default body.
    pub required: bool,
}

/// Every method of `BACnetObject`, from the trait's source. rustfmt puts each
/// item at four spaces and the trait's closing brace at column 0. A line at
/// that depth that is not a method, an attribute, a comment or the tail of a
/// signature stops the reading rather than being skipped, so nothing the
/// trait declares is missed.
pub fn trait_methods() -> Vec<TraitMethod> {
    let start = TRAIT_SOURCE
        .find("pub trait BACnetObject")
        .expect("trait declared");
    let body = &TRAIT_SOURCE[start..];
    let body = &body[..body.find("\n}\n").expect("trait closes")];
    for line in body.lines().skip(1) {
        let Some(item) = line.strip_prefix("    ") else {
            continue;
        };
        assert!(
            item.is_empty()
                || item.starts_with(' ')
                || ["fn ", "//", "#[", ")", "}"]
                    .iter()
                    .any(|start| item.starts_with(start)),
            "the BACnetObject reader does not understand `{}`",
            item.trim()
        );
    }
    let methods: Vec<_> = body
        .match_indices("\n    fn ")
        .map(|(offset, marker)| declared(&body[offset + marker.len()..]))
        .collect();
    let method = |name| {
        methods
            .iter()
            .find(|method| method.name == name)
            .unwrap_or_else(|| panic!("BACnetObject::{name} not read"))
    };
    // The reading is sound: one shared required query, one mutating required
    // method, one shared provided query, one multi-line mutating signature.
    assert!(method("object_identifier").shared && method("object_identifier").required);
    assert!(!method("write_property").shared && method("write_property").required);
    assert!(method("is_list_property").shared && !method("is_list_property").required);
    assert!(!method("audit_policy_authority_internal").shared);
    methods
}

/// One method from the text after its `fn `.
fn declared(signature: &'static str) -> TraitMethod {
    let name = signature
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .expect("method name");
    let open = signature.find('(').expect("parameter list");
    let receiver = signature[open + 1..]
        .split([',', ')'])
        .next()
        .expect("first parameter")
        .trim();
    assert!(receiver.ends_with("self"), "{name} takes no receiver");
    // The declaration ends at the first `;` (no body) or `{` (a default
    // body) outside its parentheses and brackets.
    let mut depth = 0;
    let end = signature
        .chars()
        .find(|&c| {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ => {}
            }
            depth == 0 && matches!(c, ';' | '{')
        })
        .expect("declaration ends");
    TraitMethod {
        name,
        shared: receiver.starts_with('&') && !receiver.contains("mut"),
        required: end == ';',
    }
}

/// Each method has one row, in `QUERIES` if it takes `&self` and in
/// `COMMANDS` otherwise.
pub fn assert_rows_cover_the_trait() {
    let methods = trait_methods();
    let tables: [(&str, bool, Vec<&str>); 2] = [
        ("QUERIES", true, QUERIES.iter().map(|row| row.0).collect()),
        (
            "COMMANDS",
            false,
            COMMANDS.iter().map(|row| row.0).collect(),
        ),
    ];
    for (table, shared, rows) in tables {
        let named: BTreeSet<_> = rows.iter().copied().collect();
        assert_eq!(named.len(), rows.len(), "{table} has two rows for a method");
        let declared: BTreeSet<_> = methods
            .iter()
            .filter(|method| method.shared == shared)
            .map(|method| method.name)
            .collect();
        let missing: Vec<_> = declared.difference(&named).collect();
        let unknown: Vec<_> = named.difference(&declared).collect();
        assert!(
            missing.is_empty() && unknown.is_empty(),
            "{table} in bacnet-objects/tests/support/forwarding/rows.rs does not match \
             BACnetObject: no row for {missing:?}, no such method for {unknown:?}. Give a \
             new method a probe answer and a row, then forward it in SourceReporter and, \
             if it takes &self, in DeviceReadView"
        );
    }
}

/// Every row reaches the probe's own method, and every provided method
/// answers unlike its trait default (a unit answer aside), so an adapter
/// that keeps a default cannot match the probe by accident.
pub fn assert_probe_answers_every_row() {
    let required: BTreeSet<_> = trait_methods()
        .into_iter()
        .filter(|method| method.required)
        .map(|method| method.name)
        .collect();
    for (name, row) in rows() {
        let log = CallLog::default();
        let answer = row.call(&mut Probe::new(ObjectType::ANALOG_VALUE, log.clone()));
        let prefix = format!("{name}(");
        assert!(
            take(&log).iter().any(|call| call.starts_with(&prefix)),
            "{name}: the probe must answer its row itself"
        );
        if !required.contains(name) && answer != "()" {
            let probe = Probe::new(ObjectType::ANALOG_VALUE, CallLog::default());
            assert_ne!(
                row.call(&mut Defaults(probe)),
                answer,
                "{name}: the probe must answer unlike the trait default"
            );
        }
    }
}
