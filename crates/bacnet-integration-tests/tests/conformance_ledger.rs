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
    assert_eq!(data["reviewed_at"], "2026-08-12");
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
    for doc in [SUPPORT_SUMMARY, PICS_DRAFT, BIBBS_DRAFT] {
        assert!(doc.contains("DRAFT internal support evidence"));
        assert!(doc.contains("docs/conformance/bacnet-135-2020.json"));
    }
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
