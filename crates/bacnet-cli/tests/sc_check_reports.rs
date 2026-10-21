//! Actual binary JSON/human consumers and independent certificate fixtures.
#![cfg(feature = "sc-tls")]
#[path = "support/sc_check.rs"]
#[allow(dead_code)]
mod fixture;
#[allow(dead_code)]
mod support;
use fixture::*;
use support::Process;

#[tokio::test]
async fn complete_json_preserves_inventory_and_native_paths_without_runtime_construction() {
    let mut p = params(400);
    p.subject_alt_names = vec![rcgen::SanType::DnsName("localhost".try_into().unwrap())];
    p.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![
        rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        rcgen::ExtendedKeyUsagePurpose::ServerAuth,
    ];
    let fixture = Fixture::new(p);
    for before in [true, false] {
        let mut cmd = support::command();
        if before {
            cmd.arg("--sc-cert")
                .arg(&fixture.cert)
                .arg("--sc-ca")
                .arg(&fixture.cert)
                .arg("--sc-key")
                .arg(&fixture.key);
        }
        cmd.args(["sc-check", "--at-unix", "1704067200", "-vvv"])
            .env("TOKIO_WORKER_THREADS", "0");
        if !before {
            cmd.arg("--sc-cert")
                .arg(&fixture.cert)
                .arg("--sc-ca")
                .arg(&fixture.cert)
                .arg("--sc-key")
                .arg(&fixture.key);
        }
        let output = Process::spawn(&mut cmd).output().await;
        let doc = document(&output, 0);
        assert_eq!(doc["outcome"], "completed");
        assert!(doc["error"].is_null());
        assert_eq!(doc["summary"]["errors"], 0);
        assert_eq!(doc["summary"]["warnings"], 0);
        assert_eq!(
            doc["unassessed"],
            serde_json::json!([
                "issuer_authorization",
                "certificate_path",
                "peer_name_matching",
                "revocation",
                "remote_acceptance"
            ])
        );
        let inspection = &doc["inspection"];
        assert_eq!(inspection["inspected_at_unix"].as_i64(), Some(NOW));
        assert_eq!(inspection["peer_trust_supplied"], true);
        assert_eq!(inspection["key_consistency"], "match");
        let entries = inspection["certificates"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for (entry, role) in entries.iter().zip(["identity", "peer_trust"]) {
            assert_eq!(entry["role"], role);
            assert_eq!(entry["index"], 0);
            let hash = entry["sha256"].as_str().unwrap();
            assert_eq!(hash.len(), 64);
            assert!(hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
            assert_eq!(entry["metadata"]["subject"], "CN=CLI offline fixture");
            assert_eq!(entry["metadata"]["issuer"], "CN=CLI offline fixture");
            assert_eq!(entry["metadata"]["not_before_unix"].as_i64(), Some(NOW));
            assert_eq!(
                entry["metadata"]["not_after_unix"].as_i64(),
                Some(NOW + 400 * 86400)
            );
            assert_eq!(entry["metadata"]["text_truncated"], false);
            assert_eq!(
                entry["metadata"]["public_key_algorithm_oid"],
                "1.2.840.10045.2.1"
            );
            assert_eq!(
                entry["metadata"]["signature_algorithm_oid"],
                "1.2.840.10045.4.3.2"
            );
            assert_eq!(entry["validity"], "within_interval");
            assert_eq!(entry["expiry"], "none");
            assert_eq!(
                entry["san"],
                serde_json::json!({"assessed":true,"present":true,"values":["DNS:localhost"]})
            );
            assert_eq!(
                entry["key_usage"]["values"],
                serde_json::json!(["digital_signature"])
            );
            assert_eq!(
                entry["extended_key_usage"]["values"],
                serde_json::json!(["server_auth", "client_auth"])
            );
            assert_eq!(entry["findings"], serde_json::json!([]));
        }
        assert_eq!(entries[0]["sha256"], entries[1]["sha256"]);
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&fixture.key_pem));
        assert_eq!(std::fs::read_to_string(&fixture.cert).unwrap(), fixture.pem);
        assert_eq!(
            std::fs::read_to_string(&fixture.key).unwrap(),
            fixture.key_pem
        );
    }
}

#[tokio::test]
async fn exact_dates_drive_status_while_five_and_twenty_five_day_notices_remain_success() {
    for (days, expiry) in [(5, "alarm"), (25, "warning"), (400, "none")] {
        let fixture = Fixture::new(params(days));
        let doc = document(&fixture.run(NOW).await, 0);
        assert_eq!(doc["inspection"]["certificates"][0]["expiry"], expiry);
        assert_eq!(doc["summary"]["errors"], 0);
    }
    let fixture = Fixture::new(params(400));
    let end = NOW + 400 * 86400;
    for (remaining, expiry) in [
        (7 * 86400, "alarm"),
        (7 * 86400 + 1, "warning"),
        (30 * 86400, "warning"),
        (30 * 86400 + 1, "none"),
    ] {
        let doc = document(&fixture.run(end - remaining).await, 0);
        assert_eq!(doc["inspection"]["certificates"][0]["expiry"], expiry);
    }
    for (now, code, validity) in [
        (NOW - 1, 1, "not_yet_valid"),
        (NOW, 0, "within_interval"),
        (end, 0, "within_interval"),
        (end + 1, 1, "expired"),
        (i64::MIN, 1, "not_yet_valid"),
        (i64::MAX, 1, "expired"),
    ] {
        let doc = document(&fixture.run(now).await, code);
        assert_eq!(doc["inspection"]["inspected_at_unix"].as_i64(), Some(now));
        assert_eq!(doc["inspection"]["certificates"][0]["validity"], validity);
        if code == 1 {
            assert!(has(
                &doc,
                "identity",
                Some(0),
                &format!("validity.{validity}"),
                "error"
            ));
        }
    }
}

#[tokio::test]
async fn inventory_dates_are_warnings_and_only_the_identity_leaf_is_a_date_error() {
    let healthy = Fixture::new(params(400));
    let mut p = params(1);
    p.not_before -= std::time::Duration::from_secs(2 * 86400);
    p.not_after -= std::time::Duration::from_secs(2 * 86400);
    let expired = Fixture::new(p);
    let chain = healthy
        .files
        .write("chain.pem", format!("{}{}", healthy.pem, expired.pem));
    let mut cmd = support::command();
    cmd.arg("sc-check")
        .arg("--sc-cert")
        .arg(chain)
        .arg("--sc-ca")
        .arg(&expired.cert)
        .args(["--at-unix", "1704067200"]);
    let doc = document(&Process::spawn(&mut cmd).output().await, 0);
    assert!(has(
        &doc,
        "identity",
        Some(1),
        "validity.expired",
        "warning"
    ));
    assert!(has(
        &doc,
        "peer_trust",
        Some(0),
        "validity.expired",
        "warning"
    ));
    assert_eq!(doc["summary"]["errors"], 0);
    let mut p = params(0);
    p.not_after -= std::time::Duration::from_secs(1);
    let invalid = Fixture::new(p);
    let doc = document(&invalid.run(NOW).await, 1);
    assert!(has(
        &doc,
        "identity",
        Some(0),
        "validity.invalid_interval",
        "error"
    ));
}

#[tokio::test]
async fn human_pipe_output_and_json_precedence_preserve_the_same_decision() {
    let fixture = Fixture::new(params(5));
    let output = Process::spawn(fixture.command(NOW + 1).args(["--format", "table"]))
        .output()
        .await;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("SC offline inspection at 2024-01-01T00:00:01Z"));
    for expected in [
        "0 errors, 1 warnings",
        "identity[0] | within_interval | alarm",
        "4 days 23:59:59 remaining",
        "SHA-256:",
        "Subject:",
        "Issuer:",
        "Not before:",
        "Not after:",
        "SAN: absent",
        "KU: absent",
        "Key consistency: not_assessed",
        "Key Unix read bits: not assessed",
        "expiry.alarm",
        "revocation",
        "remote acceptance",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    let doc = document(
        &Process::spawn(fixture.command(NOW).args(["--format", "table", "--json"]))
            .output()
            .await,
        0,
    );
    assert_eq!(doc["summary"]["warnings"], 1);
    let extreme = Process::spawn(fixture.command(i64::MAX).args(["--format", "table"]))
        .output()
        .await;
    assert_eq!(extreme.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&extreme.stdout)
        .contains("Unix 9223372036854775807 (UTC calendar display unavailable)"));
}

#[tokio::test]
async fn malformed_der_is_completed_with_every_item_and_unknown_key_is_never_a_match() {
    let fixture = Fixture::new(params(400));
    std::fs::write(&fixture.cert, MALFORMED_DER.repeat(64)).unwrap();
    let doc = document(
        &Process::spawn(fixture.command(NOW).arg("--sc-key").arg(&fixture.key))
            .output()
            .await,
        1,
    );
    let entries = doc["inspection"]["certificates"].as_array().unwrap();
    assert_eq!(entries.len(), 64);
    assert_eq!(entries[63]["index"], 63);
    assert_eq!(entries[0]["sha256"], ABC_SHA256);
    assert!(entries[0]["metadata"].is_null());
    assert_eq!(entries[0]["san"]["assessed"], false);
    assert_eq!(doc["inspection"]["key_consistency"], "unknown");
    assert!(has(&doc, "private_key", None, "key.unknown", "warning"));
    assert!(has(
        &doc,
        "identity",
        Some(63),
        "certificate.malformed",
        "error"
    ));
    assert_eq!(doc["summary"]["errors"], 64);
    let human = Process::spawn(fixture.command(NOW).args(["--format", "table"]))
        .output()
        .await;
    assert_eq!(human.status.code(), Some(1));
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains(ABC_SHA256));
    assert!(text.contains("Metadata: unavailable"));
    assert!(text.contains("SAN: not assessed; presence unknown"));
}

#[tokio::test]
async fn supplied_keys_distinguish_match_mismatch_unusable_and_omission() {
    let fixture = Fixture::new(params(400));
    let omitted = document(&fixture.run(NOW).await, 0);
    assert_eq!(omitted["inspection"]["key_consistency"], "not_assessed");
    assert_eq!(omitted["inspection"]["peer_trust_supplied"], false);
    let wrong = fixture.files.write(
        "wrong.key",
        rcgen::KeyPair::generate().unwrap().serialize_pem(),
    );
    let unusable = fixture.files.write(
        "unusable.key",
        "-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----\n",
    );
    for (key, expected, code) in [
        (&fixture.key, "match", 0),
        (&wrong, "mismatch", 1),
        (&unusable, "unusable", 1),
    ] {
        let doc = document(
            &Process::spawn(fixture.command(NOW).arg("--sc-key").arg(key))
                .output()
                .await,
            code,
        );
        assert_eq!(doc["inspection"]["key_consistency"], expected);
    }
}

/// A leaf from an issuing CA that the trust file lacks is flagged as not
/// issued directly by a configured CA (a warning, exit 0); with that CA in the
/// file, the document names its index (#968).
#[tokio::test]
async fn a_leaf_from_an_intermediate_outside_the_trust_file_is_not_direct() {
    let ca = |name: &str| {
        let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.distinguished_name.push(rcgen::DnType::CommonName, name);
        (p, rcgen::KeyPair::generate().unwrap())
    };
    let (root, issuing) = (ca("CLI root"), ca("CLI issuing"));
    let root_pem = root.0.self_signed(&root.1).unwrap().pem();
    let by_root = rcgen::Issuer::from_params(&root.0, &root.1);
    let issuing_pem = issuing.0.signed_by(&issuing.1, &by_root).unwrap().pem();
    let key = rcgen::KeyPair::generate().unwrap();
    let by_issuing = rcgen::Issuer::from_params(&issuing.0, &issuing.1);
    let leaf = params(400).signed_by(&key, &by_issuing).unwrap().pem();
    let fixture = Fixture::new(params(400));
    let cert = fixture.files.write("leaf.pem", leaf);
    let key = fixture.files.write("leaf.key", key.serialize_pem());
    let run = |trust: &str| {
        let trust = fixture.files.write("trust.pem", trust);
        let mut cmd = support::command();
        cmd.arg("sc-check")
            .arg("--sc-cert")
            .arg(&cert)
            .arg("--sc-key")
            .arg(&key)
            .arg("--sc-ca")
            .arg(trust)
            .args(["--at-unix", &NOW.to_string(), "--json"]);
        cmd
    };
    let doc = document(&Process::spawn(&mut run(&root_pem)).output().await, 0);
    assert_eq!(doc["inspection"]["direct_issuer"], "not_direct");
    assert!(doc["inspection"]["direct_issuer_index"].is_null());
    assert!(has(
        &doc,
        "identity",
        Some(0),
        "issuer.not_direct",
        "warning"
    ));
    assert_eq!(doc["summary"]["errors"], 0);
    let both = format!("{root_pem}{issuing_pem}");
    let doc = document(&Process::spawn(&mut run(&both)).output().await, 0);
    assert_eq!(doc["inspection"]["direct_issuer"], "direct");
    assert_eq!(doc["inspection"]["direct_issuer_index"], 1);
    assert!(!has(
        &doc,
        "identity",
        Some(0),
        "issuer.not_direct",
        "warning"
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn unix_read_bits_are_observed_without_mutation() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new(params(400));
    for mode in [0o600, 0o640, 0o604, 0o644] {
        std::fs::set_permissions(&fixture.key, std::fs::Permissions::from_mode(mode)).unwrap();
        let doc = document(
            &Process::spawn(fixture.command(NOW).arg("--sc-key").arg(&fixture.key))
                .output()
                .await,
            0,
        );
        assert_eq!(
            doc["inspection"]["key_file_permissions"],
            serde_json::json!({
                "group_readable": mode & 0o040 != 0, "world_readable": mode & 0o004 != 0
            })
        );
        assert_eq!(
            has(
                &doc,
                "private_key",
                None,
                "key_permissions.group_readable",
                "warning"
            ),
            mode & 0o040 != 0
        );
        assert_eq!(
            has(
                &doc,
                "private_key",
                None,
                "key_permissions.world_readable",
                "warning"
            ),
            mode & 0o004 != 0
        );
        assert_eq!(
            std::fs::metadata(&fixture.key)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            mode
        );
    }
}

// APFS rejects invalid UTF-8 filenames. Linux additionally exercises an existing
// file; all Unix hosts exercise byte-preserving argument parsing and I/O errors.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_existing_non_utf8_path_is_read() {
    use std::os::unix::ffi::OsStringExt;
    let mut fixture = Fixture::new(params(400));
    let native = fixture
        .files
        .0
        .join(std::ffi::OsString::from_vec(b"identity-\xff.pem".to_vec()));
    std::fs::rename(&fixture.cert, &native).unwrap();
    fixture.cert = native;
    document(&fixture.run(NOW).await, 0);
}

#[cfg(not(unix))]
#[tokio::test]
async fn non_unix_permissions_remain_explicitly_not_assessed() {
    let fixture = Fixture::new(params(400));
    let doc = document(
        &Process::spawn(fixture.command(NOW).arg("--sc-key").arg(&fixture.key))
            .output()
            .await,
        0,
    );
    assert!(doc["inspection"]["key_file_permissions"].is_null());
    assert!(has(
        &doc,
        "private_key",
        None,
        "key_permissions.not_assessed",
        "notice"
    ));
}
