use super::*;

use std::collections::BTreeMap;
use std::ffi::CStr;

const STUB: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/rusty_bacnet.pyi"));

/// A `time_sync_policy` naming every key once, each with a value that
/// differs from the default.
const EVERY_KEY: &CStr = cr"{
    'enabled': False,
    'source_restriction': [(None, b'\x01\x02'), (7, b'\x2a')],
    'max_step_ms': 1500,
    'per_source_rate': (0.5, 2),
    'global_rate': (4, 8),
    'coalesce_window_ms': 250,
    'global_coalesce_window_ms': 125,
    'max_sources': 16,
}";

/// Evaluate `source` as a Python dict and read it as `time_sync_policy`.
fn read(source: &CStr) -> PyResult<TimeSyncPolicy> {
    Python::initialize();
    Python::attach(|py| {
        let dict = py.eval(source, None, None)?.cast_into::<PyDict>()?;
        time_sync_policy(Some(&dict))
    })
}

/// The keys of [`EVERY_KEY`], in sorted order.
fn every_key() -> Vec<String> {
    Python::initialize();
    Python::attach(|py| {
        let dict = py
            .eval(EVERY_KEY, None, None)
            .unwrap()
            .cast_into::<PyDict>()
            .unwrap();
        let mut keys: Vec<String> = dict.keys().extract().unwrap();
        keys.sort();
        keys
    })
}

/// The literal names every field, so a new `TimeSyncPolicy` field stops this
/// test compiling until [`EVERY_KEY`] sets it too.
#[test]
fn every_time_sync_policy_key_reaches_its_field() {
    assert_eq!(
        read(EVERY_KEY).unwrap(),
        TimeSyncPolicy {
            enabled: false,
            source_restriction: Some(
                TimeSyncSourceRestriction::new(vec![
                    TimeSyncSource::Direct(vec![1, 2]),
                    TimeSyncSource::Routed {
                        network: 7,
                        address: vec![0x2a],
                    },
                ])
                .unwrap()
            ),
            max_step: Some(Duration::from_millis(1500)),
            per_source_rate: Some(TimeSyncRateLimit {
                max_per_second: 0.5,
                burst_capacity: 2,
            }),
            global_rate: Some(TimeSyncRateLimit {
                max_per_second: 4.0,
                burst_capacity: 8,
            }),
            coalesce_window: Duration::from_millis(250),
            global_coalesce_window: Duration::from_millis(125),
            max_sources: 16,
        }
    );
}

/// Omitted keys keep the unrestricted default, and an explicit empty
/// allowlist is kept as one that denies every source.
#[test]
fn omitted_keys_keep_the_default_and_an_empty_allowlist_is_kept() {
    assert_eq!(time_sync_policy(None).unwrap(), TimeSyncPolicy::default());
    assert_eq!(read(c"{}").unwrap(), TimeSyncPolicy::default());
    assert_eq!(
        read(c"{'source_restriction': None, 'max_step_ms': None, 'global_rate': None}").unwrap(),
        TimeSyncPolicy::default()
    );
    assert_eq!(
        read(c"{'source_restriction': []}").unwrap(),
        TimeSyncPolicy {
            source_restriction: Some(TimeSyncSourceRestriction::new(vec![]).unwrap()),
            ..TimeSyncPolicy::default()
        }
    );
}

/// The bounds the Rust policy holds entries to: 18 address octets, networks
/// 1 and 65534, 256 entries, and `max_sources` 1 and 65536.
#[test]
fn the_rust_bounds_are_accepted_at_their_edges() {
    let policy = read(
        c"{'source_restriction': [(None, bytes(18)), (1, b'\x01'), (65534, bytes(18))] * 85 + [(None, b'\x01')],
           'max_sources': 65536}",
    )
    .unwrap();
    assert_eq!(policy.max_sources, 65536);
    assert!(read(c"{'max_sources': 1}").is_ok());
}

#[test]
fn invalid_time_sync_policy_raises_the_constructor_exception_types() {
    for (source, expected) in [
        (
            c"{'max_step': 1}",
            "TypeError: time_sync_policy got an unexpected key 'max_step'",
        ),
        (c"{1: 1}", "TypeError: time_sync_policy keys must be str"),
        (
            c"{'enabled': 1}",
            "TypeError: time_sync_policy['enabled']: ",
        ),
        (
            cr"{'source_restriction': [b'\x01']}",
            "TypeError: time_sync_policy['source_restriction']: ",
        ),
        (
            c"{'source_restriction': [(None, 'ab')]}",
            "TypeError: time_sync_policy['source_restriction']: ",
        ),
        (
            c"{'max_step_ms': 1.5}",
            "TypeError: time_sync_policy['max_step_ms']: ",
        ),
        (
            c"{'per_source_rate': 1}",
            "TypeError: time_sync_policy['per_source_rate']: ",
        ),
        (
            c"{'coalesce_window_ms': None}",
            "TypeError: time_sync_policy['coalesce_window_ms']: ",
        ),
        (
            c"{'max_step_ms': -1}",
            "OverflowError: time_sync_policy['max_step_ms']: ",
        ),
        (
            cr"{'source_restriction': [(70000, b'\x01')]}",
            "OverflowError: time_sync_policy['source_restriction']: ",
        ),
        (
            c"{'global_rate': (1.0, -1)}",
            "OverflowError: time_sync_policy['global_rate']: ",
        ),
        (
            c"{'source_restriction': [(None, b'')]}",
            "ValueError: time sync: source address must contain 1..=18 octets",
        ),
        (
            c"{'source_restriction': [(None, bytes(19))]}",
            "ValueError: time sync: source address must contain 1..=18 octets",
        ),
        (
            c"{'source_restriction': [(65534, bytes(19))]}",
            "ValueError: time sync: source address must contain 1..=18 octets",
        ),
        (
            cr"{'source_restriction': [(0, b'\x01')]}",
            "ValueError: time sync: source network must be 1..=65534",
        ),
        (
            cr"{'source_restriction': [(65535, b'\x01')]}",
            "ValueError: time sync: source network must be 1..=65534",
        ),
        (
            cr"{'source_restriction': [(None, b'\x01')] * 257}",
            "ValueError: time sync: source restriction allows at most 256 entries",
        ),
        (
            c"{'per_source_rate': (0.0, 1)}",
            "ValueError: time sync: rate must be positive and finite",
        ),
        (
            c"{'global_rate': (float('inf'), 1)}",
            "ValueError: time sync: rate must be positive and finite",
        ),
        (
            c"{'global_rate': (1.0, 0)}",
            "ValueError: time sync: rate must be positive and finite",
        ),
        (
            c"{'max_sources': 0}",
            "ValueError: time sync: max_sources must be 1..=65536",
        ),
        (
            c"{'max_sources': 65537}",
            "ValueError: time sync: max_sources must be 1..=65536",
        ),
    ] {
        let error = read(source).unwrap_err().to_string();
        assert!(error.starts_with(expected), "{source:?}: {error}");
    }
}

/// The `name: annotation` members of a top-level stub class, read up to the
/// first line that is neither indented nor blank.
fn stub_class_members(class_header: &str) -> BTreeMap<String, String> {
    let (_, body) = STUB
        .split_once(&format!("\n{class_header}\n"))
        .unwrap_or_else(|| panic!("`{class_header}` missing from rusty_bacnet.pyi"));
    body.lines()
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .filter_map(|line| line.trim().split_once(": "))
        .filter(|(name, _)| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(|(name, annotation)| (name.to_owned(), annotation.to_owned()))
        .collect()
}

#[test]
fn stub_typed_dict_lists_every_time_sync_policy_key() {
    let members = stub_class_members("class TimeSyncPolicy(TypedDict, total=False):");
    assert_eq!(members.keys().cloned().collect::<Vec<_>>(), every_key());
    for (name, annotation) in &members {
        let expected = match name.as_str() {
            "enabled" => "bool",
            "source_restriction" => "list[tuple[int | None, bytes]] | None",
            "max_step_ms" => "int | None",
            "per_source_rate" | "global_rate" => "tuple[float, int] | None",
            _ => "int",
        };
        assert_eq!(annotation, expected, "{name}");
    }
    assert!(
        STUB.contains("\n        time_sync_policy: TimeSyncPolicy | None = None,\n"),
        "BACnetServer(time_sync_policy=...) missing from rusty_bacnet.pyi"
    );
}
