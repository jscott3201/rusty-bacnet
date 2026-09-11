//! Keep the local zero-only capacity policy distinct from identity closeout.
use super::*;

#[test]
fn zero_limit_policy_has_wire_lifecycle_and_native_evidence_without_promotion() {
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
    for anchor in [
        "crates/bacnet-transport/src/sc_frame/connect_test_support.rs",
        "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs",
    ] {
        assert!(row["code_anchors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == anchor));
    }
    for (field, anchors) in [
        ("positive_tests", &[
            "crates/bacnet-transport/src/sc_frame/connect.rs::tests::positive_limits_remain_independent_without_a_serviceability_floor",
            "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_positive_limits_commit_without_normalization",
            "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs::positive_limits_mtls_request_commits_exact_peer_capacities",
            "crates/rusty-bacnet/tests/test_sc_zero_limits.py::ZeroLimitsTests::test_native_nodes_zero_limits_wait_silently_then_recover",
        ][..]),
        ("negative_tests", &[
            "crates/bacnet-transport/src/sc_frame/connect.rs::tests::zero_limits_rejection_is_receive_admission_not_codec_policy",
            "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_zero_limits_is_transactional_in_every_state",
            "crates/bacnet-transport/src/sc/connect_validation_tests.rs::connect_accept_zero_limits_only_and_flood_keep_absolute_deadline",
            "crates/bacnet-transport/src/sc/reconnect_validation_tests.rs::zero_limits_accept_failover_and_failed_primary_probe_preserve_active_identity_and_limits",
            "crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs::zero_limits_mtls_collision_at_capacity_preserves_live_peers",
            "crates/bacnet-transport/src/sc_hub/deadline_commit_tests.rs::zero_limits_flood_cannot_extend_blocked_nak_connect_deadline",
            "crates/rusty-bacnet/tests/test_sc_zero_limits.py::ZeroLimitsTests::test_native_nodes_zero_limits_expire_without_connecting",
            "crates/rusty-bacnet/tests/test_sc_zero_limits.py::ZeroLimitsTests::test_native_hub_zero_limits_preserve_known_uuid_owner_and_repeat",
        ][..]),
    ] {
        for anchor in anchors {
            assert!(row[field].as_array().unwrap().iter().any(|test| test == anchor), "{anchor}");
            let parts: Vec<_> = anchor.split("::").collect();
            let source = read_repo_file(parts[0]);
            assert!(source.contains(&format!("fn {}(", parts.last().unwrap()))
                || source.contains(&format!("def {}(", parts.last().unwrap())), "{anchor}");
        }
    }
    let policy = row["zero_limit_admission"].as_str().unwrap();
    for phrase in [
        "zero-only local",
        "not a universal minimum-capacity",
        "7/80",
        "silently discarded under AB.2",
        "original connect deadline",
        "All positive values",
        "open/partial",
        "Closed #517",
    ] {
        assert!(policy.contains(phrase), "{phrase}");
    }
}

#[test]
fn zero_limit_docs_keep_capacity_policy_separate_from_immutable_identity_closeout() {
    let section = STANDARD_LEDGER
        .split_once("## Received zero-capacity admission\n")
        .unwrap()
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let normalized = section.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "Zero-only local policy",
        "not a universal minimum-capacity conformance claim",
        "silently discarded",
        "original connect deadline is not extended",
        "1/1",
        "65535/65535",
        "300/1476",
        "positive floors",
        "relationship checks",
        "5705",
        "1497",
        "1476",
        "Generic codecs",
        "post-start public mutation",
        "#519 remains open/partial",
        "AB.2.10–11",
        "AB.3.1.2/.4/.5",
        "AB.6.2",
    ] {
        assert!(normalized.contains(phrase), "{phrase}");
    }
    let closeout = sc_identity_closeout();
    assert!(!closeout.contains("#519"));
    assert!(closeout.contains("bde599405c38e2ceb62e23ee628a0f15d1ac9fe2"));
    assert_eq!(closeout.matches("\n| A").count(), 6);
    for path in [
        "README.md",
        "CHANGELOG.md",
        "docs/rust-api.md",
        "docs/python-api.md",
    ] {
        let body = read_repo_file(path);
        assert!(body.contains("received-zero-capacity-admission"), "{path}");
        let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
        for phrase in [
            "zero-only local policy",
            "7/80",
            "silently discard",
            "universal minimum-capacity conformance",
        ] {
            assert!(normalized.contains(phrase), "{path}: {phrase}");
        }
    }
}
