//! Conformance ledger schema and public-claim guard tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

const LEDGER_JSON: &str = include_str!("../../../docs/conformance/bacnet-135-2020.json");
const SUPPORT_SUMMARY: &str = include_str!("../../../docs/conformance/support-summary.md");
const PICS_DRAFT: &str = include_str!("../../../docs/conformance/pics-draft.md");
const BIBBS_DRAFT: &str = include_str!("../../../docs/conformance/bibbs-draft.md");
const STANDARD_LEDGER: &str = include_str!("../../../docs/conformance/standard-135-2020-ledger.md");

const REQUIRED_IDS: &[&str] = &[
    "BACNET-J-BVLC-FUNCTION-CODES",
    "BACNET-J-ORIGINAL-UNICAST-NPDU",
    "BACNET-J-ORIGINAL-BROADCAST-NPDU",
    "BACNET-J-FORWARDED-NPDU",
    "BACNET-J-BBMD-BDT",
    "BACNET-J-FOREIGN-DEVICE-FDT",
    "BACNET-J-NAT-TRAVERSAL",
    "BACNET-J-IP-MULTICAST",
    "BACNET-AB-SC-FRAME",
    "BACNET-AB-SC-BVLC-RESULT",
    "BACNET-AB-SC-HUB-CONNECTOR",
    "BACNET-AB-SC-WEBSOCKET-TLS",
    "BACNET-AB-SC-HEARTBEAT",
    "BACNET-5-TSM-CLIENT",
    "BACNET-5-TSM-SERVER",
    "BACNET-5-SEGMENTATION-WINDOW",
    "BACNET-6-NPDU-CONTROL",
    "BACNET-6-ROUTER-MESSAGES",
    "BACNET-A-PICS",
    "BACNET-K-BIBBS",
    "BACNET-9-MSTP-FRAMES",
    "BACNET-U-IPV6-BVLL",
    "BACNET-7-ETHERNET-LLC",
    "BACNET-8-ARCNET",
    "BACNET-10-PTP",
    "BACNET-11-LONTALK",
    "BACNET-13-COV-SUBSCRIPTIONS",
    "BACNET-O-ZIGBEE",
];

const ALLOWED_STATUSES: &[&str] = &[
    "in-progress",
    "implementation-present-needs-conformance-tests",
    "implementation-present-needs-negative-tests",
    "implementation-present-needs-security-tests",
    "implementation-present-needs-timeout-tests",
    "implementation-present-needs-state-machine-audit",
    "implementation-present-needs-window-tests",
    "implementation-present-needs-source-review",
    "implementation-present-needs-platform-tests",
    "supported-with-clause-evidence",
    "deferred-pending-owner-decision",
    "unsupported-by-design",
    "unknown-pending-source-review",
];

struct ClaimRule {
    files: &'static [&'static str],
    needle: &'static str,
    required_ids: &'static [&'static str],
}

const CLAIM_RULES: &[ClaimRule] = &[
    ClaimRule {
        files: &[
            "README.md",
            "docs/rust-api.md",
            "docs/python-api.md",
            "docs/CLI.md",
        ],
        needle: "BACnet/IP",
        required_ids: &["BACNET-J-BVLC-FUNCTION-CODES"],
    },
    ClaimRule {
        files: &[
            "README.md",
            "docs/rust-api.md",
            "docs/python-api.md",
            "docs/CLI.md",
        ],
        needle: "BACnet/IPv6",
        required_ids: &["BACNET-U-IPV6-BVLL"],
    },
    ClaimRule {
        files: &[
            "README.md",
            "docs/rust-api.md",
            "docs/python-api.md",
            "docs/CLI.md",
        ],
        needle: "BACnet/SC",
        required_ids: &["BACNET-AB-SC-FRAME", "BACNET-AB-SC-WEBSOCKET-TLS"],
    },
    ClaimRule {
        files: &["README.md", "docs/rust-api.md", "docs/architecture.md"],
        needle: "MS/TP",
        required_ids: &["BACNET-9-MSTP-FRAMES"],
    },
    ClaimRule {
        files: &["README.md", "docs/rust-api.md", "docs/architecture.md"],
        needle: "Ethernet",
        required_ids: &["BACNET-7-ETHERNET-LLC"],
    },
    ClaimRule {
        files: &["README.md", "docs/architecture.md", "docs/rust-api.md"],
        needle: "object",
        required_ids: &["BACNET-12-OBJECT-MODEL"],
    },
    ClaimRule {
        files: &["README.md", "docs/architecture.md"],
        needle: "BTL Test Plan",
        required_ids: &["BACNET-A-PICS"],
    },
];

const FORBIDDEN_PUBLIC_CLAIMS: &[(&str, &str)] = &[
    ("README.md", "A complete BACnet protocol stack"),
    ("README.md", "Full BACnet/IP stack"),
    ("README.md", "All standard BACnet objects"),
    ("README.md", "full API parity"),
    (
        "README.md",
        "Omitting `ca_cert` leaves the hub in server-auth-only example mode",
    ),
    (
        "Benchmarks.md",
        "All tests ran on localhost with zero errors unless noted.",
    ),
    ("Benchmarks.md", "Zero errors across all tests"),
    ("Benchmarks.md", "production-ready"),
    ("Benchmarks.md", "zero latency degradation"),
    (
        "docs/architecture.md",
        "BTL Test Plan 26.1 compliance harness",
    ),
];

fn ledger() -> Value {
    serde_json::from_str(LEDGER_JSON).expect("ledger JSON should parse")
}

fn rows_by_id(data: &Value) -> BTreeMap<String, &Value> {
    data["rows"]
        .as_array()
        .expect("rows should be an array")
        .iter()
        .map(|row| {
            (
                row["id"]
                    .as_str()
                    .expect("row id should be a string")
                    .to_owned(),
                row,
            )
        })
        .collect()
}

fn assert_array_field(row: &Value, field: &str) {
    assert!(
        row[field].as_array().is_some(),
        "{} must have array field {field}",
        row["id"]
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_path(path: &str) -> PathBuf {
    repo_root().join(path)
}

fn read_repo_file(path: &str) -> String {
    fs::read_to_string(repo_path(path)).expect("repo file should be readable")
}

#[test]
fn ledger_schema_has_required_seed_rows_and_unique_ids() {
    let data = ledger();
    assert_eq!(data["standard"], "ANSI/ASHRAE Standard 135-2020");
    assert_eq!(data["reviewed_at"], "2026-08-13");
    assert!(
        data["repo_sha"].as_str().is_some_and(|sha| sha.len() == 40),
        "repo_sha must be a full git SHA"
    );

    let rows = data["rows"].as_array().expect("rows should be an array");
    let allowed_statuses: BTreeSet<&str> = ALLOWED_STATUSES.iter().copied().collect();
    let mut ids = BTreeSet::new();
    for row in rows {
        let id = row["id"].as_str().expect("row id should be a string");
        assert!(ids.insert(id), "duplicate ledger id {id}");
        assert!(row["standard_anchor"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(row["priority"]
            .as_str()
            .is_some_and(|p| matches!(p, "P0" | "P1" | "P2" | "P3")));
        assert!(row["requirement_summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        let status = row["status"].as_str().expect("status should be a string");
        assert!(
            allowed_statuses.contains(status),
            "unsupported status {status} for {id}"
        );
        assert_array_field(row, "code_anchors");
        assert_array_field(row, "positive_tests");
        assert_array_field(row, "negative_tests");
        assert_array_field(row, "benchmarks");
        assert_array_field(row, "public_claims");
    }

    let row_map = rows_by_id(&data);
    for required in REQUIRED_IDS {
        assert!(
            row_map.contains_key(*required),
            "missing seed row {required}"
        );
    }
}

#[test]
fn supported_rows_require_clause_evidence_and_positive_tests() {
    let data = ledger();
    let errors =
        supported_row_evidence_errors(data["rows"].as_array().expect("rows should be an array"));
    assert!(
        errors.is_empty(),
        "supported row evidence guard failed:\n{}",
        errors.join("\n")
    );
}

#[test]
fn supported_row_guard_rejects_supported_status_without_positive_tests() {
    let data = json!({
        "rows": [{
            "id": "BACNET-TEST-SUPPORTED-ROW",
            "standard_anchor": "Clause 1",
            "status": "supported-with-clause-evidence",
            "positive_tests": []
        }]
    });

    let errors =
        supported_row_evidence_errors(data["rows"].as_array().expect("rows should be an array"));
    assert!(errors
        .iter()
        .any(|e| e.contains("BACNET-TEST-SUPPORTED-ROW is supported but has no positive tests")));
}

#[test]
fn public_claim_guard_current_docs() {
    let data = ledger();
    let row_map = rows_by_id(&data);
    let docs = CLAIM_RULES
        .iter()
        .flat_map(|rule| rule.files.iter().copied())
        .collect::<BTreeSet<_>>();
    let contents = docs
        .into_iter()
        .map(|path| (path, read_repo_file(path)))
        .collect::<Vec<_>>();

    let errors = claim_guard_errors(&contents, CLAIM_RULES, &row_map);
    assert!(
        errors.is_empty(),
        "public claim guard failed:\n{}",
        errors.join("\n")
    );
}

#[test]
fn public_docs_avoid_unqualified_support_claims() {
    for (path, forbidden) in FORBIDDEN_PUBLIC_CLAIMS {
        let body = read_repo_file(path);
        assert!(
            !body.contains(forbidden),
            "{path} contains unqualified support claim {forbidden:?}"
        );
    }
}

#[test]
fn current_ledger_does_not_cite_retired_one_way_sc_benchmarks() {
    let data = ledger();
    let retired = [
        "benchmarks/benches/sc_latency.rs",
        "benchmarks/benches/sc_throughput.rs",
    ];
    for row in data["rows"].as_array().expect("rows should be an array") {
        for anchor in row["benchmarks"]
            .as_array()
            .expect("benchmarks should be an array")
        {
            let anchor = anchor
                .as_str()
                .expect("benchmark anchor should be a string");
            assert!(
                !retired.contains(&anchor),
                "{} cites retired benchmark {anchor}; historical results are not current anchors",
                row["id"]
            );
        }
    }
}

#[test]
fn sc_credential_evidence_does_not_promote_the_full_security_profile() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-WEBSOCKET-TLS"];
    assert_eq!(row["status"], "implementation-present-needs-security-tests");
    let tests = row["negative_tests"]
        .as_array()
        .expect("negative tests should be an array");
    for anchor in [
        "crates/bacnet-transport/tests/sc_hub_tls.rs::typed_config_rejects_empty_ca_before_startup",
        "crates/bacnet-transport/src/sc_tls/tls_config_tests.rs::node_tls_factory_requires_nonempty_ca_and_identity",
        "crates/rusty-bacnet/tests/test_sc_hub_mtls.py::HubMtlsTests::test_invalid_files_fail_before_bind",
        "crates/rusty-bacnet/tests/test_sc_hub_mtls.py::NodeMtlsTests::test_invalid_local_files_do_not_dial_or_drain",
        "crates/bacnet-cli/tests/sc_ca.rs::missing_ca_rejected_before_dial",
    ] {
        assert!(
            tests.iter().any(|test| test.as_str() == Some(anchor)),
            "SC credential acceptance must retain preflight evidence: {anchor}"
        );
    }
}

#[test]
fn sc_identity_evidence_keeps_caller_storage_and_raw_transport_limits_explicit() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-WEBSOCKET-TLS"];
    assert_eq!(row["status"], "implementation-present-needs-security-tests");
    for anchor in [
        "crates/rusty-bacnet/tests/test_sc_hub_mtls.py::NodeIdentityMtlsTests::test_uuid_owned_wire_bytes_across_stop_start_and_recreation",
        "crates/rusty-bacnet/tests/test_sc_hub_mtls.py::NodeIdentityMtlsTests::test_distinct_nodes_and_same_uuid_replacement_leave_other_node_usable",
        "benchmarks/tests/sc_mtls/node_identity.rs::sc_server_uuid_wire_bytes_survive_reconnect_and_fresh_builds",
        "crates/bacnet-transport/tests/sc_hub_tls.rs::local_hub_identity_wire_bytes_survive_fresh_start_on_every_api",
        "crates/rusty-bacnet/tests/test_sc_hub_mtls.py::NodeIdentityMtlsTests::test_hub_owned_identity_survives_stop_start_and_fresh_object",
        "benchmarks/tests/sc_binary/handshake.rs::hub_identity_is_explicit_and_stable_across_binary_restart",
    ] {
        assert!(row["positive_tests"].as_array().unwrap().iter().any(|test| test == anchor));
    }
    // The machine-readable tranche notes retain their slice-time issue status.
    assert!(row["notes"].as_str().unwrap().contains("#517 remains open"));
    for body in [row["notes"].as_str().unwrap(), STANDARD_LEDGER] {
        assert!(body.contains("changed UUIDs cannot be detected without application history"));
        assert!(body.contains("before transport-owned I/O or startup state changes"));
        assert!(body.contains("same owned WebSocket"));
        assert!(body.contains("not lifetime immutability"));
        assert!(body.contains("later application mutation through public connection()"));
        assert!(body.contains("Wire admission, peer replacement"));
        assert!(body.contains("hosting port VMAC and hosting device UUID"));
        assert!(body.contains("one shared Rust check"));
        assert!(body.contains("TEST-ONLY hub UUID"));
        assert!(
            !body.contains("the Python two-node sketch is illustrative, not validated end-to-end")
        );
    }
}

#[test]
fn sc_hub_identity_evidence_retains_pre_io_checks_and_no_status_promotion() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-WEBSOCKET-TLS"];
    for anchor in [
        "crates/bacnet-transport/tests/sc_hub_tls.rs::local_hub_identity_rejected_before_bind_on_every_start_api",
        "crates/rusty-bacnet/tests/test_sc_hub_identity.py::HubIdentityTests::test_uuid_required_length_zero_and_vmac_errors_precede_io",
        "crates/rusty-bacnet/tests/test_sc_hub_identity.py::HubIdentityTests::test_installed_hub_stub_matches_runtime_keyword_contract",
        "benchmarks/tests/sc_binary/preflight.rs::missing_empty_and_invalid_identity_precede_file_or_network_io",
    ] {
        assert!(row["negative_tests"].as_array().unwrap().iter().any(|test| test == anchor));
    }
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|row| row["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    assert_eq!(row["status"], "implementation-present-needs-security-tests");
}

#[test]
fn sc_raw_identity_evidence_retains_startup_only_boundary() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-WEBSOCKET-TLS"];
    for anchor in [
        "crates/bacnet-transport/src/sc/identity_tests.rs::explicit_zero_uuid_rejected_without_io_and_repaired_on_same_socket",
        "crates/bacnet-transport/src/sc/identity_tests.rs::zero_vmac_rejected_without_io_or_socket_consumption",
        "crates/bacnet-transport/src/sc/identity_tests.rs::broadcast_vmac_rejected_without_io_or_socket_consumption",
        "crates/bacnet-transport/src/sc/identity_tests.rs::reconnect_then_heartbeat_then_identity_error_precedence",
    ] {
        assert!(row["negative_tests"].as_array().unwrap().iter().any(|test| test == anchor));
    }
    let docs = read_repo_file("docs/rust-api.md");
    assert!(
        docs.contains("startup enforcement, not lifetime immutability")
            || docs.contains("startup\nenforcement, not lifetime immutability")
    );
    assert!(docs.contains("cannot undo caller-owned WebSocket creation"));
    assert!(docs.contains("There is no\nnew VMAC repair setter"));
}

#[test]
fn public_claim_guard_rejects_missing_ledger_row() {
    let data = json!({"rows": []});
    let row_map = rows_by_id(&data);
    let docs = [("README.md", "BACnet/SC transport support".to_owned())];
    let rules = [ClaimRule {
        files: &["README.md"],
        needle: "BACnet/SC",
        required_ids: &["BACNET-AB-SC-FRAME"],
    }];
    let errors = claim_guard_errors(&docs, &rules, &row_map);
    assert!(errors
        .iter()
        .any(|e| e.contains("missing ledger row BACNET-AB-SC-FRAME")));
}

#[test]
fn sc_peer_uuid_evidence_retains_silent_accept_policy_without_status_promotion() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    assert_eq!(
        row["status"],
        "implementation-present-needs-state-machine-audit"
    );
    for anchor in [
        "crates/bacnet-transport/src/sc/connect_validation_tests.rs",
        "crates/bacnet-transport/src/sc/reconnect_validation_tests.rs",
        "crates/bacnet-transport/src/sc_tls/connect_accept_tests.rs",
    ] {
        assert!(row["code_anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == anchor));
    }
    for anchor in [
        "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs::zero_uuid_mtls_request_never_reaches_admission",
        "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs::zero_uuid_mtls_collision_at_capacity_preserves_live_peers",
        "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs::zero_uuid_mtls_repeat_flood_preserves_activity_probe_and_registration",
        "crates/rusty-bacnet/tests/test_sc_peer_uuid.py::PeerUuidTests::test_nil_request_nak_close_repeat_and_surviving_native_read",
        "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_zero_uuid_is_transactional_in_every_state",
        "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_nil_only_and_flood_keep_absolute_deadline",
        "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_nil_wrong_id_is_discarded_but_valid_wrong_id_is_terminal",
        "crates/bacnet-transport/src/sc/reconnect_validation_tests.rs::nil_accept_failover_and_failed_primary_probe_preserve_active_identity_and_limits",
        "crates/bacnet-transport/src/sc/reconnect_validation_tests.rs::nil_accept_reconnect_probe_times_out_then_redials_without_reseeding",
        "crates/bacnet-transport/src/sc_tls/connect_accept_tests.rs::nil_accept_tls_expires_without_peer_identity_or_limits",
        "crates/rusty-bacnet/tests/test_sc_accept_uuid.py::AcceptUuidTests::test_native_nodes_nil_accept_expires_without_connecting",
    ] {
        assert!(row["negative_tests"].as_array().unwrap().iter().any(|test| test == anchor));
    }
    for anchor in [
        "crates/bacnet-transport/src/sc_frame/connect.rs::tests::nonzero_uuid_bits_remain_opaque",
        "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_invalid_matrix_waits_silently_for_valid_accept",
        "crates/bacnet-transport/src/sc_tls/connect_accept_tests.rs::nil_accept_tls_is_silent_until_later_valid_accept",
        "crates/rusty-bacnet/tests/test_sc_accept_uuid.py::AcceptUuidTests::test_native_nodes_wait_silently_then_accept_valid_uuid",
    ] {
        assert!(row["positive_tests"].as_array().unwrap().iter().any(|test| test == anchor));
    }
    for phrase in [
        "Local security policy",
        "Nonzero bits remain opaque",
        "Owner-approved scoped resolution",
        "Connect-Accept with a zero UUID is silently discarded",
        "prohibits replies to response messages",
        "local diagnostic, not a wire NAK",
        "without publishing Connected or resetting the absolute connect",
        "manual raw sending still permit nil syntax",
        "not a pre-dial check",
    ] {
        assert!(
            STANDARD_LEDGER.contains(phrase),
            "missing boundary: {phrase}"
        );
    }
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|row| row["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
}

#[test]
fn public_claim_guard_rejects_unknown_status_for_public_claim() {
    let data = json!({
        "rows": [{
            "id": "BACNET-9-MSTP-FRAMES",
            "standard_anchor": "Clause 9.3",
            "status": "unknown-pending-source-review"
        }]
    });
    let row_map = rows_by_id(&data);
    let docs = [("README.md", "MS/TP transport support".to_owned())];
    let rules = [ClaimRule {
        files: &["README.md"],
        needle: "MS/TP",
        required_ids: &["BACNET-9-MSTP-FRAMES"],
    }];
    let errors = claim_guard_errors(&docs, &rules, &row_map);
    assert!(errors
        .iter()
        .any(|e| e.contains("unknown-pending-source-review")));
}

fn sc_identity_closeout() -> &'static str {
    let heading = "### Device identity acceptance closeout\n";
    let section = STANDARD_LEDGER.split_once(heading).unwrap();
    section.1.split("\n## ").next().unwrap()
}

#[test]
fn sc_identity_closeout_maps_six_criteria_without_promoting_excluded_guarantees() {
    let body = sc_identity_closeout();
    let criteria: Vec<_> = body.split("\n| A").skip(1).collect();
    assert_eq!(criteria.len(), 6);
    for (index, evidence) in [
        "test_distinct_nodes_and_same_uuid_replacement_leave_other_node_usable",
        "test_sc_uuid_validation_precedes_file_and_socket_io",
        "test_uuid_owned_wire_bytes_across_stop_start_and_recreation",
        "documented provisioning boundary",
        "strict_hub_start_family_requires_mutual_tls13_and_preserves_uuid",
        "sc_client_builder_sends_configured_vmac_and_device_uuid",
    ]
    .iter()
    .enumerate()
    {
        let row = criteria[index].lines().next().unwrap();
        assert!(row.starts_with(&format!("{} |", index + 1)));
        assert!(row.contains(evidence) && row.contains("]("), "{evidence}");
    }
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    for boundary in [
        "proposed closeout",
        "caller-owned provisioning/storage",
        "non-colliding VMACs",
        "all nonzero 128-bit values",
        "deferred/excluded",
        "not fully implemented",
        "not literal all-public-API coverage",
        "post-start mutation",
        "cannot undo caller-owned WebSocket",
        "VMAC validation may follow dialing",
        "not application disk-storage qualification",
        "Runtime evidence is reused",
        "no fresh native/platform qualification",
        "full Annex AB/PICS/BTL",
        "bde599405c38e2ceb62e23ee628a0f15d1ac9fe2",
    ] {
        let present = normalized.contains(boundary);
        assert!(present, "missing boundary: {boundary}");
    }
    for section in ["4.1.1", "4.1.3", "4.1.7"] {
        let rfc = "https://www.rfc-editor.org/rfc/rfc4122.html";
        assert!(body.contains(&format!("{rfc}#section-{section}")));
    }
    assert!(!body.contains("#517 remains open"));
}

#[test]
fn sc_identity_closeout_links_and_symbol_anchors_resolve_offline() {
    let target = "conformance/standard-135-2020-ledger.md#device-identity-acceptance-closeout";
    for (doc, prefix) in [
        ("README.md", "docs/"),
        ("CHANGELOG.md", "docs/"),
        ("docs/rust-api.md", ""),
        ("docs/python-api.md", ""),
    ] {
        let link = format!("]({prefix}{target})");
        assert!(read_repo_file(doc).contains(&link), "{doc}");
    }
    let links = sc_identity_closeout()
        .split('[')
        .filter_map(|s| s.split_once("]("));
    for (label, link) in links {
        let target = link.split(')').next().unwrap();
        if target.starts_with("https://") {
            continue;
        }
        let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
        let source = read_repo_file(&format!("docs/conformance/{path}"));
        if !anchor.is_empty() {
            // Both provisioning links target level-four, plain-word headings.
            let heading = format!("#### {}", anchor.replace('-', " "));
            let lower = source.to_lowercase();
            assert!(lower.lines().any(|s| s == heading), "{target}");
        }
        if label.starts_with('`') {
            let symbol = label.trim_matches('`');
            let rust = source.contains(&format!("fn {symbol}("));
            let python = source.contains(&format!("def {symbol}("));
            assert!(rust || python, "missing {symbol} in {path}");
        }
    }
}

#[test]
fn public_claim_guard_rejects_claim_without_standard_anchor() {
    let data = json!({
        "rows": [{
            "id": "BACNET-J-BVLC-FUNCTION-CODES",
            "standard_anchor": "",
            "status": "implementation-present-needs-conformance-tests"
        }]
    });
    let row_map = rows_by_id(&data);
    let docs = [("README.md", "BACnet/IP transport support".to_owned())];
    let rules = [ClaimRule {
        files: &["README.md"],
        needle: "BACnet/IP",
        required_ids: &["BACNET-J-BVLC-FUNCTION-CODES"],
    }];
    let errors = claim_guard_errors(&docs, &rules, &row_map);
    assert!(errors.iter().any(|e| e.contains("has no Standard anchor")));
}

#[test]
fn generated_support_docs_are_current_with_ledger() {
    let data = ledger();
    let repo_sha = data["repo_sha"]
        .as_str()
        .expect("repo_sha should be a string");
    for doc in [SUPPORT_SUMMARY, PICS_DRAFT, BIBBS_DRAFT] {
        assert!(doc.contains("DRAFT internal support evidence"));
        assert!(doc.contains("docs/conformance/bacnet-135-2020.json"));
    }
    assert!(
        STANDARD_LEDGER.contains(&format!(
            "Implementation evidence SHA reviewed: `{repo_sha}`"
        )),
        "standard ledger evidence SHA differs from the machine-readable ledger"
    );
    assert!(STANDARD_LEDGER.contains("## Clause 4 Architecture"));
    assert!(STANDARD_LEDGER.contains("## Annex AB BACnet/SC"));
    for id in REQUIRED_IDS {
        assert!(SUPPORT_SUMMARY.contains(id), "support summary missing {id}");
    }
    assert!(PICS_DRAFT.contains("BACNET-A-PICS"));
    assert!(PICS_DRAFT.contains("BACNET-L-PROFILES"));
    assert!(BIBBS_DRAFT.contains("BACNET-K-BIBBS"));
    assert_eq!(
        rows_by_id(&data).len(),
        data["rows"].as_array().unwrap().len()
    );
}

#[test]
fn generated_support_docs_match_generator_check() {
    let output = Command::new("python3")
        .arg(repo_path("scripts/generate-conformance-docs.py"))
        .arg("--check")
        .current_dir(repo_root())
        .output()
        .expect("conformance generator should run");
    assert!(
        output.status.success(),
        "conformance generated docs are stale\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn supported_row_evidence_errors(rows: &[Value]) -> Vec<String> {
    let mut errors = Vec::new();
    for row in rows {
        if row["status"] == "supported-with-clause-evidence" {
            let id = row["id"].as_str().unwrap_or("<missing id>");
            if row["standard_anchor"].as_str().is_none_or(|s| s.is_empty()) {
                errors.push(format!("{id} is supported but has no Standard anchor"));
            }
            if row["positive_tests"]
                .as_array()
                .is_none_or(|tests| tests.is_empty())
            {
                errors.push(format!("{id} is supported but has no positive tests"));
            }
        }
    }
    errors
}

fn claim_guard_errors(
    docs: &[(&str, String)],
    rules: &[ClaimRule],
    rows: &BTreeMap<String, &Value>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for rule in rules {
        let claim_present = docs
            .iter()
            .any(|(path, body)| rule.files.contains(path) && body.contains(rule.needle));
        if !claim_present {
            continue;
        }
        for required_id in rule.required_ids {
            let Some(row) = rows.get(*required_id) else {
                errors.push(format!(
                    "public claim {:?} is present but missing ledger row {required_id}",
                    rule.needle
                ));
                continue;
            };
            let anchor = row["standard_anchor"].as_str().unwrap_or_default();
            if anchor.is_empty() {
                errors.push(format!(
                    "{required_id} is linked to a public claim but has no Standard anchor"
                ));
            }
            let status = row["status"].as_str().unwrap_or_default();
            if status.is_empty() || status == "unknown-pending-source-review" {
                errors.push(format!(
                    "{required_id} is linked to a public claim but status is {status:?}"
                ));
            }
        }
    }
    errors
}
