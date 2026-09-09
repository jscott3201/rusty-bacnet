//! Real CLI process regressions for explicit SC trust and local TLS preflight.
#![cfg(feature = "sc-tls")]

#[allow(dead_code)]
mod support;

use rcgen::ExtendedKeyUsagePurpose;
use rcgen::{CertificateParams, KeyPair};
use support::sc::{cli, read, Site};
use support::{bounded, command, failure, Files, Process};

#[tokio::test]
async fn missing_ca_rejected_before_dial() {
    let files = Files::new();
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["client".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert = files.write("operational.pem", cert.pem());
    let key = files.write("operational.key", key.serialize_pem());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let mut cmd = command();
    cmd.args(["--sc", "--json", "--sc-url", &url, "--sc-cert"])
        .arg(cert)
        .arg("--sc-key")
        .arg(key)
        .args([
            "--sc-vmac",
            "02:00:00:00:00:02",
            "--sc-device-uuid",
            "02020202-0202-0202-0202-020202020202",
            "read",
            "02:00:00:00:00:01",
            "dev:123",
            "object-name",
        ]);
    let output = Process::spawn(&mut cmd).output();
    tokio::pin!(output);
    let (output, dialed) = bounded(async {
        tokio::select! {
            output = &mut output => (output, false),
            accepted = listener.accept() => {
                drop(accepted.unwrap());
                (output.await, true)
            }
        }
    })
    .await;
    assert!(!dialed, "missing explicit CA reached TCP dial");
    failure(&output, "--sc-ca");
}

#[tokio::test]
async fn invalid_local_files_never_reach_live_listener() {
    let site = Site::new();
    let leaf = site.leaf("cli", ExtendedKeyUsagePurpose::ClientAuth, None);
    let files = Files::new();
    let ca = files.write("site CA é.pem", site.ca.pem());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let invalid_der = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
    let malformed = "-----BEGIN CERTIFICATE-----\n!not-base64!\n-----END CERTIFICATE-----\n";
    let invalid_ca_paths = [
        std::path::PathBuf::new(),
        files.0.join("absent.pem"),
        files.0.clone(), // a directory is not a readable PEM file
        files.write("empty.pem", ""),
        files.write("text.pem", "not PEM"),
        files.write("malformed.pem", malformed),
        files.write("truncated.pem", "-----BEGIN CERTIFICATE-----\nAQID\n"),
        files.write("invalid-der.pem", invalid_der),
        files.write("key-only.pem", leaf.key.serialize_pem()),
        files.write("mixed.pem", format!("{}{invalid_der}", site.ca.pem())),
    ];
    for path in invalid_ca_paths {
        let mut cmd = cli(&files, &url, &leaf);
        cmd.arg("--sc-ca").arg(&path);
        let output = Process::spawn(read(&mut cmd)).output().await;
        failure(&output, "--sc-ca");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "invalid CA reached TCP: {}",
            path.display()
        );
    }
    for (name, contents, diagnostic) in [
        ("operational é.pem", "".to_owned(), "TLS config error"),
        ("operational é.pem", malformed.to_owned(), "cert PEM"),
        (
            "operational é.pem",
            invalid_der.to_owned(),
            "TLS config error",
        ),
        ("operational é.key", "".to_owned(), "key PEM"),
        ("operational é.key", "not a key".to_owned(), "key PEM"),
        (
            "operational é.key",
            KeyPair::generate().unwrap().serialize_pem(),
            "TLS config error",
        ),
    ] {
        let mut cmd = cli(&files, &url, &leaf);
        cmd.arg("--sc-ca").arg(&ca);
        files.write(name, contents);
        let output = Process::spawn(read(&mut cmd)).output().await;
        failure(&output, diagnostic);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    // Positive control: the SAME executable, listener and valid local inputs
    // really do dial. Close immediately so TLS fails, rather than waiting out a
    // timeout or treating a listening-but-unserviced peer as success.
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let mut cmd = cli(&files, &url, &leaf);
    cmd.arg("--sc-ca").arg(ca);
    let (output, ()) = tokio::join!(
        Process::spawn(read(&mut cmd)).output(),
        bounded(async {
            drop(listener.accept().await.unwrap());
        })
    );
    failure(&output, "TLS handshake");
}

#[tokio::test]
async fn missing_operational_credentials_and_identities_still_fail_locally() {
    // Every file is absent: identity/presence validation must win over file I/O.
    let base = [
        "--sc",
        "--json",
        "--sc-ca",
        "/absent/site.pem",
        "--sc-url",
        "wss://127.0.0.1:1",
    ];
    for (extra, message) in [
        (vec![], "--sc-cert is required"),
        (vec!["--sc-cert", "/absent/cert"], "--sc-key is required"),
        (
            vec!["--sc-cert", "/absent/cert", "--sc-key", "/absent/key"],
            "--sc-vmac is required",
        ),
        (
            vec![
                "--sc-cert",
                "/absent/cert",
                "--sc-key",
                "/absent/key",
                "--sc-vmac",
                "020000000002",
            ],
            "--sc-device-uuid is required",
        ),
    ] {
        let mut cmd = command();
        cmd.args(base).args(extra);
        failure(&Process::spawn(read(&mut cmd)).output().await, message);
    }
}
