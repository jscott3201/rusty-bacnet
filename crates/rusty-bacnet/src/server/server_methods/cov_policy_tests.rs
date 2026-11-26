use super::*;

use std::collections::BTreeMap;
use std::ffi::CStr;

const STUB: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/rusty_bacnet.pyi"));

/// A `cov_policy` naming every key once, each with a value that differs from
/// the default and from the other keys.
const EVERY_KEY: &CStr = cr"{
    'max_subscriptions_global': 101,
    'max_subscriptions_per_peer': 102,
    'reserved_capacity': 103,
    'reserved_peers': [b'\x01', b'\x02\x03'],
    'reserved_recipients': [(None, b'\x04'), (7, b'\x05\x06')],
    'allow_indefinite_subscriptions': False,
    'max_indefinite_per_peer': 104,
    'max_notifications_per_event': 105,
    'max_notification_bytes_per_event': 106,
    'max_confirmed_in_flight_per_peer': 107,
}";

/// Evaluate `source` as a Python dict and read it as `cov_policy`.
fn read(source: &CStr) -> PyResult<CovPolicy> {
    Python::initialize();
    Python::attach(|py| {
        let dict = py.eval(source, None, None)?.cast_into::<PyDict>()?;
        cov_policy(Some(&dict))
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

/// The literal names every field, so a new `CovPolicy` field stops this test
/// compiling until [`EVERY_KEY`] sets it too; a key the binding doesn't read
/// is then refused as unknown.
#[test]
fn every_cov_policy_key_reaches_its_field() {
    assert_eq!(
        read(EVERY_KEY).unwrap(),
        CovPolicy {
            max_subscriptions_global: 101,
            max_subscriptions_per_peer: 102,
            reserved_capacity: 103,
            reserved_peers: vec![MacAddr::from_slice(&[1]), MacAddr::from_slice(&[2, 3])],
            reserved_recipients: vec![
                CovRecipient::Direct(MacAddr::from_slice(&[4])),
                CovRecipient::Routed(NpduAddress {
                    network: 7,
                    mac_address: MacAddr::from_slice(&[5, 6]),
                }),
            ],
            allow_indefinite_subscriptions: false,
            max_indefinite_per_peer: 104,
            max_notifications_per_event: 105,
            max_notification_bytes_per_event: 106,
            max_confirmed_in_flight_per_peer: 107,
        }
    );
}

/// An 18-octet reserved MAC, the longest source the network layer delivers,
/// reads in every form (#1199); one octet more raises ValueError, as
/// `invalid_cov_policy_raises_the_constructor_exception_types` shows.
#[test]
fn eighteen_octet_reserved_macs_are_accepted() {
    let policy = read(
        c"{'reserved_peers': [bytes(18)], 'reserved_recipients': [(None, bytes(18)), (65534, bytes(18))]}",
    )
    .unwrap();
    let mac = MacAddr::from_slice(&[0; 18]);
    assert_eq!(policy.reserved_peers, vec![mac.clone()]);
    assert_eq!(
        policy.reserved_recipients,
        vec![
            CovRecipient::Direct(mac.clone()),
            CovRecipient::Routed(NpduAddress {
                network: 65534,
                mac_address: mac,
            }),
        ]
    );
}

#[test]
fn omitted_or_empty_cov_policy_keeps_the_rust_defaults() {
    assert_eq!(cov_policy(None).unwrap(), CovPolicy::default());
    assert_eq!(read(c"{}").unwrap(), CovPolicy::default());
    assert_eq!(
        read(c"{'max_subscriptions_per_peer': 2}").unwrap(),
        CovPolicy {
            max_subscriptions_per_peer: 2,
            ..CovPolicy::default()
        }
    );
}

#[test]
fn invalid_cov_policy_raises_the_constructor_exception_types() {
    for (source, expected) in [
        (
            c"{'max_subscriptions': 1}",
            "TypeError: cov_policy got an unexpected key 'max_subscriptions'",
        ),
        (c"{1: 1}", "TypeError: cov_policy keys must be str"),
        (
            c"{'max_subscriptions_global': 1.5}",
            "TypeError: cov_policy['max_subscriptions_global']: ",
        ),
        (
            c"{'max_subscriptions_global': '10'}",
            "TypeError: cov_policy['max_subscriptions_global']: ",
        ),
        (
            c"{'allow_indefinite_subscriptions': 1}",
            "TypeError: cov_policy['allow_indefinite_subscriptions']: ",
        ),
        (
            cr"{'reserved_peers': b'\x01'}",
            "TypeError: cov_policy['reserved_peers']: ",
        ),
        (
            c"{'reserved_peers': ['ab']}",
            "TypeError: cov_policy['reserved_peers']: ",
        ),
        (
            cr"{'reserved_recipients': [b'\x01']}",
            "TypeError: cov_policy['reserved_recipients']: ",
        ),
        (
            c"{'max_subscriptions_per_peer': -1}",
            "OverflowError: cov_policy['max_subscriptions_per_peer']: ",
        ),
        (
            c"{'max_subscriptions_global': 2**200}",
            "OverflowError: cov_policy['max_subscriptions_global']: ",
        ),
        (
            cr"{'reserved_recipients': [(70000, b'\x01')]}",
            "OverflowError: cov_policy['reserved_recipients']: ",
        ),
        (
            c"{'max_notifications_per_event': 0}",
            "ValueError: COV policy max_notifications_per_event must be positive",
        ),
        (
            c"{'reserved_peers': [b'']}",
            "ValueError: COV policy reserved_peers entries need a MAC of 1..=18 octets",
        ),
        (
            c"{'reserved_peers': [bytes(19)]}",
            "ValueError: COV policy reserved_peers entries need a MAC of 1..=18 octets",
        ),
        (
            c"{'reserved_recipients': [(None, bytes(19))]}",
            "ValueError: COV policy reserved_recipients entries need a MAC of 1..=18 octets",
        ),
        (
            c"{'reserved_recipients': [(65534, bytes(19))]}",
            "ValueError: COV policy reserved_recipients entries need a MAC of 1..=18 octets",
        ),
        (
            cr"{'reserved_recipients': [(65535, b'\x01')]}",
            "ValueError: COV policy reserved_recipients networks must be 1..=65534",
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
fn stub_typed_dict_lists_every_cov_policy_key() {
    let members = stub_class_members("class CovPolicy(TypedDict, total=False):");
    assert_eq!(members.keys().cloned().collect::<Vec<_>>(), every_key());
    for (name, annotation) in &members {
        let expected = match name.as_str() {
            "allow_indefinite_subscriptions" => "bool",
            "reserved_peers" => "list[bytes]",
            "reserved_recipients" => "list[tuple[int | None, bytes]]",
            _ => "int",
        };
        assert_eq!(annotation, expected, "{name}");
    }
    assert!(
        STUB.contains("\n        cov_policy: CovPolicy | None = None,\n"),
        "BACnetServer(cov_policy=...) missing from rusty_bacnet.pyi"
    );
}
