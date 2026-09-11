//! Node rejection evidence remains bounded local policy, not a status promotion.
use super::*;

#[test]
fn mu_liveness_evidence_preserves_scope_and_blocked_write_limitation() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    assert_eq!(
        row["status"],
        "implementation-present-needs-state-machine-audit"
    );
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|row| row["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    assert_eq!(data["reviewed_at"], "2026-08-13");
    assert_eq!(data["repo_sha"], "f485021f5cd7058ac406d57d3d317936cbe7b361");
    for (field, anchors) in [
        ("positive_tests", &[
            "crates/bacnet-transport/src/sc/mu_liveness_tests.rs::mu_liveness_valid_npdu_data_options_and_heartbeat_request_restore_activity",
            "crates/bacnet-transport/src/sc_tls/mu_liveness_tests.rs::mu_liveness_tls_wire_rejection_and_healthy_recovery",
            "crates/rusty-bacnet/tests/test_sc_mu_liveness.py::MuLivenessTests::test_mu_rejection_then_healthy_native_read_property",
        ][..]),
        ("negative_tests", &[
            "crates/bacnet-transport/src/sc/mu_liveness_tests.rs::mu_liveness_rejected_burst_preserves_pending_probe_and_matching_ack",
            "crates/bacnet-transport/src/sc/mu_liveness_tests.rs::mu_liveness_rejected_traffic_keeps_original_idle_timeout_and_reconnects",
            "crates/bacnet-transport/src/sc/mu_liveness_tests.rs::mu_liveness_source_and_control_admission_still_precede_mu",
        ][..]),
    ] {
        for anchor in anchors {
            assert!(row[field].as_array().unwrap().iter().any(|v| v == anchor), "{anchor}");
            let parts: Vec<_> = anchor.split("::").collect();
            let source = read_repo_file(parts[0]);
            let name = parts.last().unwrap();
            assert!(source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")), "{anchor}");
        }
    }
    let policy = row["mu_rejection_liveness"].as_str().unwrap();
    for phrase in [
        "local admission policy",
        "AB.3.1.4",
        "AB.3.1.2/.3",
        "AB.6.3",
        "universal invalid-frame accounting",
        "receive loop progresses",
        "Blocked NAK sends",
        "blocked-send/backpressure",
        "Post-handle_received",
        "open/partial",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let section = STANDARD_LEDGER
        .split_once("## MU-rejection liveness accounting\n")
        .unwrap()
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let normalized = section.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "Base Standard 135-2020",
        "AB.3.1.4",
        "remaining parts",
        "existing receive-drop",
        "AB.6.3",
        "receive loop progress",
        "blocked-send/backpressure",
        "not fixed here",
        "Address-Resolution-ACK",
        "#519 remains open/partial",
        "full Annex AB",
    ] {
        assert!(normalized.contains(phrase), "{phrase}");
    }
    assert!(read_repo_file("CHANGELOG.md").contains("#mu-rejection-liveness-accounting"));
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}

#[test]
fn rejection_nak_budget_evidence_preserves_freshness_and_cancellation_limits() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    let policy = row["rejection_nak_budget"].as_str().unwrap();
    for phrase in [
        "owner-approved local policy",
        "remaining accepted-activity heartbeat budget",
        "Silent/nonrejection",
        "immediate errors",
        "Fresh-only recovery",
        "unused failover",
        "caller contract",
        "already-admitted application sends",
        "not rolled back",
        "not immediate OS closure",
        "not OS backpressure",
        "available state locks",
        "Other write paths",
        "open/partial",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let mut anchors = 0;
    for field in ["positive_tests", "negative_tests"] {
        for anchor in row[field].as_array().unwrap() {
            let anchor = anchor.as_str().unwrap();
            if !anchor.contains("rejection_deadline") {
                continue;
            }
            let parts: Vec<_> = anchor.split("::").collect();
            let source = read_repo_file(parts[0]);
            let name = parts.last().unwrap();
            assert!(
                source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")),
                "{anchor}"
            );
            anchors += 1;
        }
    }
    assert_eq!(anchors, 19); // Original 17 plus fourth/fifth-path fresh-recovery tests.
    for path in [
        "crates/bacnet-transport/src/sc/rejection.rs",
        "crates/bacnet-transport/src/sc/recovery.rs",
        "crates/bacnet-transport/src/sc_tls/rejection_deadline_tests.rs",
    ] {
        assert!(row["code_anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|anchor| anchor == path));
    }
    let section = STANDARD_LEDGER
        .split_once("## Rejection NAK budget and fresh-only recovery\n")
        .unwrap()
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let normalized = section.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "Base Standard 135-2020",
        "AB.3.1.4/.5",
        "AB.6.1/.2",
        "AB.6.3",
        "remaining-parts",
        "existing receive-drop",
        "three blocked-NAK gaps",
        "not immediate OS closure",
        "not OS backpressure",
        "not a hard real-time",
        "60-second expiry",
        "0.3.33",
        "0.29.0",
        "1.53.1",
        "#519 remains open/partial",
    ] {
        // Remove emphasis solely for comparing words across Markdown markup.
        assert!(normalized.replace("**", "").contains(phrase), "{phrase}");
    }
    assert!(
        read_repo_file("CHANGELOG.md").contains("#rejection-nak-budget-and-fresh-only-recovery")
    );
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}

#[test]
fn empty_npdu_evidence_preserves_zero_only_scope_and_existing_lifecycle_owners() {
    let data = ledger();
    let rows = rows_by_id(&data);
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|r| r["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    assert_eq!(data["reviewed_at"], "2026-08-13");
    assert_eq!(data["repo_sha"], "f485021f5cd7058ac406d57d3d317936cbe7b361");
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    assert_eq!(
        row["status"],
        "implementation-present-needs-state-machine-audit"
    );
    let policy = row["empty_npdu_admission"].as_str().unwrap();
    for phrase in [
        "AB.2.5/.1",
        "AB.3.1.5",
        "7/149",
        "marker zero",
        "source then MU",
        "Pre-registration OTHER",
        "One-byte compatibility",
        "codec/raw-send syntax",
        "fourth node path",
        "fresh-only recovery",
        "not a new global NAK deadline",
        "two independent installed-native seams",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let mut count = 0;
    for field in ["positive_tests", "negative_tests"] {
        for anchor in row[field].as_array().unwrap() {
            let anchor = anchor.as_str().unwrap();
            if !anchor.contains("empty_npdu") {
                continue;
            }
            let parts: Vec<_> = anchor.split("::").collect();
            let source = read_repo_file(parts[0]);
            let name = parts.last().unwrap();
            assert!(
                source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")),
                "{anchor}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 13);
    assert!(STANDARD_LEDGER.contains("## Empty Encapsulated-NPDU admission\n"));
    assert!(read_repo_file("CHANGELOG.md").contains("#empty-encapsulated-npdu-admission"));
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}

#[test]
fn unknown_function_evidence_preserves_node_only_scope_and_fifth_budget_path() {
    let data = ledger();
    let rows = rows_by_id(&data);
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|r| r["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    assert_eq!(
        row["status"],
        "implementation-present-needs-state-machine-audit"
    );
    let policy = row["unknown_function_admission"].as_str().unwrap();
    for phrase in [
        "established NODE only",
        "0x0D..0xFF",
        "AB.3.1.5",
        "7/143",
        "marker zero",
        "All explicit destinations",
        "owner-local policy",
        "Known 0x00..0x0C",
        "Result-for-Unknown",
        "handshake silence",
        "Fifth node path",
        "fresh-only recovery",
        "raw fake hub to NODE",
        "not native hub fallback",
        "not immediate OS closure",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let mut count = 0;
    for field in ["positive_tests", "negative_tests"] {
        for anchor in row[field].as_array().unwrap() {
            let anchor = anchor.as_str().unwrap();
            if !anchor.contains("unknown_function") {
                continue;
            }
            let parts: Vec<_> = anchor.split("::").collect();
            let source = read_repo_file(parts[0]);
            let name = parts.last().unwrap();
            assert!(
                source.contains(&format!("fn {name}(")) || source.contains(&format!("def {name}(")),
                "{anchor}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 11);
    assert!(STANDARD_LEDGER.contains("## Node unknown-function admission\n"));
    assert!(read_repo_file("CHANGELOG.md").contains("#node-unknown-function-admission"));
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}
