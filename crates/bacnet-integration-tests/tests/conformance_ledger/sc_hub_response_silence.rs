//! Bind the narrow accepting-hub supplement to actual tests, not a promotion.
use super::*;

#[test]
fn hub_response_silence_has_scoped_policy_and_executable_anchors() {
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
            .filter(|r| r["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    assert_eq!(data["reviewed_at"], "2026-08-13");
    assert_eq!(data["repo_sha"], "f485021f5cd7058ac406d57d3d317936cbe7b361");
    for file in [
        "crates/bacnet-transport/src/sc_hub/handler.rs",
        "crates/bacnet-transport/src/sc_hub/response_silence_tests.rs",
        "crates/bacnet-transport/src/sc_hub/response_silence_lifecycle_tests.rs",
    ] {
        assert!(row["code_anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == file));
        assert!(!read_repo_file(file).is_empty());
    }
    for (field, anchors) in [
        ("positive_tests", &[
            "crates/bacnet-transport/src/sc_hub/response_silence_tests.rs::unsolicited_response_matrix_preserves_lease_activity_and_probe_then_recovers",
            "crates/rusty-bacnet/tests/test_sc_hub_response_silence.py::HubResponseSilenceTests::test_unsolicited_responses_preserve_native_hub_and_registration",
        ][..]),
        ("negative_tests", &[
            "crates/bacnet-transport/src/sc_hub/response_silence_tests.rs::unsolicited_connect_accept_is_silent_before_and_after_registration",
            "crates/bacnet-transport/src/sc_hub/response_silence_tests.rs::unsolicited_disconnect_ack_is_silent_before_and_after_registration",
            "crates/bacnet-transport/src/sc_hub/response_silence_tests.rs::unsolicited_responses_do_not_defer_idle_probe_or_its_original_timeout",
            "crates/bacnet-transport/src/sc_hub/response_silence_lifecycle_tests.rs::unsolicited_responses_mtls_keep_absolute_connect_deadline_and_release_admission",
            "crates/bacnet-transport/src/sc_hub/response_silence_lifecycle_tests.rs::unsolicited_responses_at_capacity_preserve_owners_then_allow_real_replacement",
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
    let policy = row["unsolicited_response_admission"].as_str().unwrap();
    for phrase in [
        "only unsolicited",
        "0x07",
        "0x09",
        "AB.2",
        "AB.6.2.3",
        "scoped local liveness policy",
        "Result relay",
        "matching Heartbeat-ACK",
        "Address-Resolution-ACK",
        "excluded",
        "open/partial",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let section = STANDARD_LEDGER
        .split_once("## Accepting hub unsolicited-response silence\n")
        .unwrap()
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let normalized = section.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "Base Standard 135-2020",
        "AB.2.11",
        "AB.2.13",
        "AB.6.2.3",
        "local liveness policy",
        "not a claim that AB.6.3",
        "no Disconnect-ACK waiter",
        "Address-Resolution-ACK is not included",
        "Result relay",
        "matching Heartbeat-ACK",
        "#519 remains open/partial",
        "not full Annex AB",
    ] {
        assert!(normalized.contains(phrase), "{phrase}");
    }
    assert!(read_repo_file("CHANGELOG.md").contains("#accepting-hub-unsolicited-response-silence"));
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}

#[test]
fn hub_unknown_transit_evidence_keeps_family_scope_and_existing_lifecycle() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let row = rows["BACNET-AB-SC-CONNECTION-STATE"];
    assert_eq!(rows.len(), 68);
    assert_eq!(
        rows.values()
            .filter(|r| r["status"] == "supported-with-clause-evidence")
            .count(),
        19
    );
    assert_eq!(
        row["status"],
        "implementation-present-needs-state-machine-audit"
    );
    assert_eq!(data["reviewed_at"], "2026-08-13");
    assert_eq!(data["repo_sha"], "f485021f5cd7058ac406d57d3d317936cbe7b361");
    let policy = row["hub_unknown_transit"].as_str().unwrap();
    for phrase in [
        "AB.5.1",
        "AB.5.3/.1/.2/.3",
        "registered Unknown 0x0D..0xFF",
        "encoded BVLC limits only",
        "same socket only",
        "no source echo",
        "7/143",
        "ResultForUnknown",
        "NODE production/fatal policy unchanged",
        "owner-local policy",
        "Pending ACK timeout was already independent",
        "idle-probe deferral",
        "no new global NAK deadline",
        "not rollback",
        "General known-function forwarding",
        "open/partial",
        "No full Annex AB claim",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
    let mut count = 0;
    for field in ["positive_tests", "negative_tests"] {
        for anchor in row[field].as_array().unwrap() {
            let anchor = anchor.as_str().unwrap();
            if !anchor.contains("unknown_transit") {
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
    assert!(STANDARD_LEDGER.contains("## Hub Unknown transit and Result return\n"));
    assert!(read_repo_file("CHANGELOG.md").contains("#hub-unknown-transit-and-result-return"));
    assert_eq!(sc_identity_closeout().matches("\n| A").count(), 6);
}
