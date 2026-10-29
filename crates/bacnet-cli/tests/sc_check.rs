//! Consumer contracts for the synchronous offline SC inspection command.

#[allow(dead_code)]
mod support;

use support::{Process, command};

#[tokio::test]
async fn discovery_succeeds_without_credentials_or_sc_feature() {
    let root = Process::spawn(command().arg("--help")).output().await;
    assert!(root.status.success());
    assert!(String::from_utf8_lossy(&root.stdout).contains("sc-check"));
    let subcommand = Process::spawn(command().args(["sc-check", "--help"]))
        .output()
        .await;
    assert!(subcommand.status.success());
    assert!(subcommand.stderr.is_empty());
    let help = String::from_utf8_lossy(&subcommand.stdout);
    for expected in ["offline", "--sc-cert", "--at-unix", "hub"] {
        assert!(help.contains(expected), "help omitted {expected}");
    }
}

#[tokio::test]
async fn usage_errors_precede_execution_and_do_not_emit_json() {
    for arguments in [
        vec!["sc-check"],
        vec![
            "sc-check",
            "--sc-cert",
            "missing",
            "--at-unix",
            "9223372036854775808",
        ],
        vec![
            "sc-check",
            "--sc-cert",
            "missing",
            "--at-unix",
            "-9223372036854775809",
        ],
        vec![
            "sc-check",
            "--sc-cert",
            "missing",
            "--format",
            "yaml",
            "--json",
        ],
    ] {
        let result = Process::spawn(command().args(arguments)).output().await;
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains("error:"));
    }
}

#[tokio::test]
async fn explicit_transport_globals_are_rejected_on_either_side_even_at_defaults() {
    for flags in [
        vec!["--interface", "0.0.0.0"],
        vec!["--port", "47808"],
        vec!["--broadcast", "255.255.255.255"],
        vec!["--timeout", "6000"],
        vec!["--min-interval-ms", "0"],
        vec!["--foreign-bbmd", "127.0.0.1"],
        vec!["--ttl", "300"],
        vec!["--renewal", "150", "--foreign-bbmd", "127.0.0.1"],
        vec!["--ipv6"],
        vec!["--ipv6-interface", "::"],
        vec!["--device-instance", "0"],
        vec!["--sc"],
        vec!["--sc-url", "wss://unused.invalid"],
        vec!["--sc-vmac", "020000000001"],
        vec!["--sc-device-uuid", "00000000000000000000000000000001"],
    ] {
        for before in [true, false] {
            let mut cmd = command();
            if before {
                cmd.args(&flags);
            }
            cmd.args(["sc-check", "--sc-cert", "missing", "--json"]);
            if !before {
                cmd.args(&flags);
            }
            let result = Process::spawn(&mut cmd).output().await;
            assert_eq!(result.status.code(), Some(2), "flag {}", flags[0]);
            assert!(result.stdout.is_empty());
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains("transport option"), "{stderr}");
        }
    }
}

#[cfg(not(feature = "sc-tls"))]
#[tokio::test]
async fn feature_off_is_discoverable_but_execution_reports_structured_unavailable() {
    for before in [true, false] {
        let mut cmd = command();
        if before {
            cmd.args(["--sc-cert", "not-opened", "--sc-key", "also-not-opened"]);
        }
        cmd.args(["sc-check", "--at-unix", "-9223372036854775808"]);
        if !before {
            cmd.args(["--sc-cert", "not-opened", "--sc-key", "also-not-opened"]);
        }
        // This existing Tokio setting would make runtime construction fail.
        // Offline dispatch must return before such construction is attempted.
        cmd.env("TOKIO_WORKER_THREADS", "0");
        let result = Process::spawn(&mut cmd).output().await;
        assert_eq!(result.status.code(), Some(3));
        assert!(result.stderr.is_empty());
        let doc: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(doc["outcome"], "not_completed");
        assert_eq!(doc["error"]["category"], "feature_unavailable");
        assert!(
            doc["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--features sc-tls")
        );
        for field in ["inspection", "summary"] {
            assert!(doc[field].is_null());
        }
        for field in ["role", "index", "kind", "os_code"] {
            assert!(doc["error"][field].is_null());
        }
        assert_eq!(doc["diagnostics"], serde_json::json!([]));
    }
    let result =
        Process::spawn(command().args(["sc-check", "--sc-cert", "missing", "--format", "table"]))
            .output()
            .await;
    assert_eq!(result.status.code(), Some(3));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--features sc-tls"));
}
