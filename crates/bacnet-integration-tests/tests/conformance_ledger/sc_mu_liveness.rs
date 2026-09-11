//! MU ordering is bounded local evidence, not a broader status promotion.
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
