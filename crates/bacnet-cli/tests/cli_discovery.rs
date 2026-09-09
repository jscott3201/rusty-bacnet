//! Consumer-level discovery and feature-disabled compatibility checks.
#[allow(dead_code)]
mod support;
use support::{failure, run};

#[tokio::test]
async fn help_and_version_do_not_load_sc_files() {
    for option in ["--help", "--version"] {
        for args in [
            vec![option],
            vec![
                "--sc",
                "--sc-ca",
                "/nonexistent/site é.pem",
                "--sc-cert",
                "/nonexistent/client.pem",
                "--sc-key",
                "/nonexistent/client.key",
                option,
            ],
            vec!["--sc", "--sc-ca=", option],
        ] {
            let output = run(args).await;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
            let text = String::from_utf8(output.stdout).unwrap();
            if option == "--help" {
                assert!(text.contains("--sc-ca <FILE>"));
            } else {
                assert_eq!(text.trim(), concat!("bacnet ", env!("CARGO_PKG_VERSION")));
            }
        }
    }
    let output = run(["read", "--sc-ca", "/nonexistent/é.pem", "--help"]).await;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[cfg(not(feature = "sc-tls"))]
#[tokio::test]
async fn disabled_sc_preserves_rebuild_advice_without_loading_files() {
    for ca in [None, Some(""), Some("/nonexistent/site é.pem")] {
        let mut args = vec![
            "--sc",
            "--json",
            "read",
            "02:00:00:00:00:01",
            "dev:123",
            "on",
        ];
        if let Some(ca) = ca {
            args.extend(["--sc-ca", ca]);
        }
        let output = run(args).await;
        failure(&output, "cargo install bacnet-cli --features sc-tls");
        assert_eq!(output.status.code(), Some(1));
    }
}

#[cfg(not(feature = "pcap"))]
#[tokio::test]
async fn capture_without_client_preserves_its_feature_advice() {
    let output = run(["--sc", "--sc-ca", "/nonexistent/é.pem", "capture"]).await;
    failure(&output, "cargo install bacnet-cli --features pcap");
}

#[tokio::test]
async fn bip_ignores_unused_sc_ca_and_credentials() {
    // Unicast only to an owned loopback socket, never network device discovery.
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = socket.local_addr().unwrap().to_string();
    let output = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--json",
        "--sc-ca",
        "/nonexistent/site é.pem",
        "--sc-cert",
        "/nonexistent/cert",
        "--sc-key",
        "/nonexistent/key",
        "discover",
        "--wait",
        "0",
        "--target",
        &target,
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, serde_json::json!([]));
}

#[tokio::test]
async fn ipv6_does_not_require_sc_files() {
    let output = run([
        "--ipv6",
        "--ipv6-interface",
        "::1",
        "--port",
        "0",
        "--device-instance",
        "123",
        "--json",
        "--sc-ca",
        "/nonexistent/site.pem",
        "devices",
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, serde_json::json!([]));
}
