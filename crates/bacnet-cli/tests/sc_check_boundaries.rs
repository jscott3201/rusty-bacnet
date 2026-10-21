//! Bounded input, extension interpretation and output-failure consumer evidence.
#![cfg(feature = "sc-tls")]
#[path = "support/sc_check.rs"]
#[allow(dead_code)]
mod fixture;
#[allow(dead_code)]
mod support;
use fixture::*;
use support::Process;

#[tokio::test]
async fn execution_errors_preserve_role_kind_and_nullability_without_paths_or_secrets() {
    let fixture = Fixture::new(params(400));
    let missing = fixture.files.0.join("secret-input-path.pem");
    let malformed = fixture.files.write(
        "malformed.pem",
        "-----BEGIN CERTIFICATE-----\nsecret!\n-----END CERTIFICATE-----\n",
    );
    let empty = fixture.files.write("empty.pem", "");
    let huge = fixture.files.write("huge.pem", vec![b' '; (1 << 20) + 1]);
    // 131073 decoded bytes, all zero, expressed independently as base64 quartets.
    let large_cert = fixture.files.write(
        "large-cert.pem",
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            "AAAA".repeat(43691)
        ),
    );
    let many = fixture.files.write("many.pem", MALFORMED_DER.repeat(65));
    for (flag, path, role, category, kind, index) in [
        ("--sc-ca", &missing, "peer_trust", "io", None, None),
        (
            "--sc-ca",
            &empty,
            "peer_trust",
            "input",
            Some("empty_certificates"),
            None,
        ),
        (
            "--sc-key",
            &empty,
            "private_key",
            "input",
            Some("missing_private_key"),
            None,
        ),
        (
            "--sc-ca",
            &malformed,
            "peer_trust",
            "input",
            Some("malformed_pem"),
            Some(0),
        ),
        (
            "--sc-ca",
            &huge,
            "peer_trust",
            "input",
            Some("input_too_large"),
            None,
        ),
        (
            "--sc-ca",
            &large_cert,
            "peer_trust",
            "input",
            Some("certificate_too_large"),
            Some(0),
        ),
        (
            "--sc-ca",
            &many,
            "peer_trust",
            "input",
            Some("too_many_certificates"),
            Some(63),
        ),
    ] {
        let result = Process::spawn(fixture.command(NOW).arg(flag).arg(path))
            .output()
            .await;
        let doc = document(&result, 3);
        assert_eq!(doc["outcome"], "not_completed");
        assert!(doc["inspection"].is_null() && doc["summary"].is_null());
        assert_eq!(doc["diagnostics"], serde_json::json!([]));
        assert_eq!(doc["error"]["category"], category);
        assert_eq!(doc["error"]["role"], role);
        assert_eq!(doc["error"]["kind"].as_str(), kind);
        assert_eq!(doc["error"]["index"].as_u64(), index);
        if category == "io" {
            assert!(doc["error"]["os_code"].is_i64());
        } else {
            assert!(doc["error"]["os_code"].is_null());
        }
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(!text.contains("secret") && !text.contains("BEGIN"));
        assert!(!text.contains(path.to_str().unwrap()));
    }
    let human = Process::spawn(
        fixture
            .command(NOW)
            .arg("--sc-ca")
            .arg(&missing)
            .args(["--format", "table"]),
    )
    .output()
    .await;
    assert_eq!(human.status.code(), Some(3));
    assert!(human.stdout.is_empty());
    let text = String::from_utf8(human.stderr).unwrap();
    assert!(text.contains("io peer_trust") && text.contains("OS code"));
    assert!(!text.contains("secret-input-path"));
}

#[cfg(unix)]
#[tokio::test]
async fn native_non_utf8_missing_path_returns_a_structured_io_error() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new(params(400));
    let native = fixture
        .files
        .0
        .join(std::ffi::OsString::from_vec(b"missing-\xff.pem".to_vec()));
    let doc = document(
        &Process::spawn(fixture.command(NOW).arg("--sc-ca").arg(native))
            .output()
            .await,
        3,
    );
    assert_eq!(doc["error"]["category"], "io");
    assert_eq!(doc["error"]["role"], "peer_trust");
    assert!(doc["error"]["os_code"].is_i64());
}

fn extension(oid: &[u64], bytes: Vec<u8>) -> rcgen::CustomExtension {
    rcgen::CustomExtension::from_oid_content(oid, bytes)
}

#[tokio::test]
async fn absent_and_interpretation_defects_remain_inventory_or_warnings() {
    let absent = document(&Fixture::new(params(400)).run(NOW).await, 0);
    for area in ["san", "key_usage", "extended_key_usage"] {
        assert_eq!(
            absent["inspection"]["certificates"][0][area],
            serde_json::json!({"assessed":true,"present":false,"values":[]})
        );
    }
    for (oid, area, value) in [
        (vec![2, 5, 29, 17], "san", vec![0x30, 3, 0x82, 1, b'a']),
        (vec![2, 5, 29, 15], "key_usage", vec![3, 2, 7, 0x80]),
        (
            vec![2, 5, 29, 37],
            "extended_key_usage",
            vec![0x30, 10, 6, 8, 0x2b, 6, 1, 5, 5, 7, 3, 1],
        ),
    ] {
        for kind in ["malformed", "duplicate"] {
            let mut p = params(400);
            p.custom_extensions = if kind == "malformed" {
                vec![extension(&oid, vec![0xff])]
            } else {
                vec![
                    extension(&oid, value.clone()),
                    extension(&oid, value.clone()),
                ]
            };
            let doc = document(&Fixture::new(p).run(NOW).await, 0);
            assert!(has(
                &doc,
                "identity",
                Some(0),
                &format!("{area}.{kind}"),
                "warning"
            ));
            assert_eq!(doc["inspection"]["certificates"][0][area]["present"], true);
        }
    }
    let mut p = params(400);
    p.custom_extensions = vec![
        extension(&[2, 5, 29, 17], vec![0x30, 2, 0xa3, 0]),
        extension(&[2, 5, 29, 37], vec![0x30, 4, 6, 2, 0x2a, 3]),
    ];
    let doc = document(&Fixture::new(p).run(NOW).await, 0);
    assert!(has(&doc, "identity", Some(0), "san.unsupported", "warning"));
    assert!(has(
        &doc,
        "identity",
        Some(0),
        "extended_key_usage.unsupported",
        "warning"
    ));
    assert_eq!(
        doc["inspection"]["certificates"][0]["extended_key_usage"]["values"],
        serde_json::json!(["1.2.3"])
    );
}

#[tokio::test]
async fn truncation_and_hostile_text_survive_projection_without_terminal_controls() {
    let mut p = params(400);
    p.distinguished_name
        .push(rcgen::DnType::CommonName, "x\n\x1b\u{202e}".repeat(1000));
    p.custom_extensions.push(extension(
        &[2, 5, 29, 17],
        tlv(0x30, &tlv(0x86, &vec![b'a'; 1000]).repeat(20)),
    ));
    let fixture = Fixture::new(p);
    let doc = document(&fixture.run(NOW).await, 0);
    assert!(has(
        &doc,
        "identity",
        Some(0),
        "metadata.text_truncated",
        "warning"
    ));
    assert!(has(&doc, "identity", Some(0), "san.truncated", "warning"));
    let certificate = &doc["inspection"]["certificates"][0];
    assert_eq!(certificate["metadata"]["text_truncated"], true);
    assert_eq!(certificate["san"]["assessed"], false);
    assert_eq!(certificate["san"]["values"], serde_json::json!([]));
    let subject = certificate["metadata"]["subject"].as_str().unwrap();
    assert!(!subject.contains(['\n', '\x1b', '\u{202e}']));
    let human = Process::spawn(fixture.command(NOW).args(["--format", "table"]))
        .output()
        .await;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains(subject) && text.contains("Metadata text: truncated"));
    assert!(!text.contains(['\x1b', '\u{202e}']));

    let mut p = params(400);
    p.custom_extensions.push(extension(
        &[2, 5, 29, 17],
        tlv(0x30, &tlv(0x86, &vec![b'a'; 300]).repeat(20)),
    ));
    let doc = document(&Fixture::new(p).run(NOW).await, 0);
    let values = doc["inspection"]["certificates"][0]["san"]["values"]
        .as_array()
        .unwrap();
    assert_eq!(values.len(), 16);
    assert!(values.iter().all(|v| v.as_str().unwrap().len() <= 256));
    assert!(has(&doc, "identity", Some(0), "san.truncated", "warning"));
}

#[tokio::test]
async fn excessive_extensions_complete_with_warning_and_unknown_presence() {
    let mut p = params(400);
    p.custom_extensions = (0..65)
        .map(|n| extension(&[1, 2, 3, n], vec![5, 0]))
        .collect();
    let fixture = Fixture::new(p);
    let doc = document(&fixture.run(NOW).await, 0);
    assert!(has(
        &doc,
        "identity",
        Some(0),
        "certificate.truncated",
        "warning"
    ));
    for area in ["san", "key_usage", "extended_key_usage"] {
        assert_eq!(
            doc["inspection"]["certificates"][0][area]["assessed"],
            false
        );
        assert!(has(
            &doc,
            "identity",
            Some(0),
            &format!("{area}.not_assessed"),
            "notice"
        ));
    }
    let human = Process::spawn(fixture.command(NOW).args(["--format", "table"]))
        .output()
        .await;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout).unwrap();
    assert_eq!(text.matches("not assessed; presence unknown").count(), 3);
}

#[tokio::test]
async fn omitted_clock_uses_one_current_sample_and_diagnostics_have_stable_order() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let fixture = Fixture::new(params(400));
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let output = Process::spawn(
        support::command()
            .arg("sc-check")
            .arg("--sc-cert")
            .arg(&fixture.cert),
    )
    .output()
    .await;
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let doc = document(&output, 1); // This fixed historical leaf is now expired.
    let sample = doc["inspection"]["inspected_at_unix"].as_i64().unwrap();
    assert!((before..=after).contains(&sample));
    let roles: Vec<_> = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["identity", "peer_trust", "private_key", "private_key"]
    );
}

#[tokio::test]
async fn a_closed_stdout_pipe_returns_three_and_reaps_without_panic() {
    use std::{
        fs::File,
        process::{Child, Stdio},
        time::Duration,
    };
    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            self.0.wait().unwrap();
        }
    }
    let fixture = Fixture::new(params(400));
    // More output than typical pipe buffers guarantees a write after closure.
    std::fs::write(&fixture.cert, MALFORMED_DER.repeat(64)).unwrap();
    let stderr = fixture.files.0.join("pipe-stderr");
    let mut child = ChildGuard(
        fixture
            .command(NOW)
            .stdout(Stdio::piped())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    drop(child.0.stdout.take());
    let status = support::bounded(async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert_eq!(status.code(), Some(3));
    let text = std::fs::read_to_string(stderr).unwrap();
    assert!(text.contains("could not write or flush sc-check output"));
    assert!(!text.contains("panicked"));
}
