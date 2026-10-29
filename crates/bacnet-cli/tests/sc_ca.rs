//! Real CLI process regressions for explicit SC trust and local TLS preflight.
#![cfg(feature = "sc-tls")]

#[allow(dead_code)]
mod support;

use rcgen::ExtendedKeyUsagePurpose;
use rcgen::{CertificateParams, KeyPair};
use support::sc::{Site, cli, read};
use support::{Files, Process, bounded, command, failure};

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
    // The pre-dial credential lint (#968) explains what the TLS
    // configuration would otherwise refuse with a terser error.
    for (name, contents, diagnostic) in [
        (
            "operational é.pem",
            "".to_owned(),
            "SC credential check could not complete: input identity (empty_certificates)",
        ),
        ("operational é.pem", malformed.to_owned(), "cert PEM"),
        (
            "operational é.pem",
            invalid_der.to_owned(),
            "SC credential check refused the dial before connecting: Operational certificate [0]",
        ),
        ("operational é.key", "".to_owned(), "key PEM"),
        ("operational é.key", "not a key".to_owned(), "key PEM"),
        (
            "operational é.key",
            KeyPair::generate().unwrap().serialize_pem(),
            "Key file: The private key does not match the identity leaf",
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

/// The pre-dial lint (#968): an expired or not-yet-valid leaf is a hard
/// failure, refused with its reason before any TCP connection. The dates are
/// decades from any plausible clock, so the verdict does not depend on it.
#[tokio::test]
async fn expired_and_future_leaves_are_refused_before_any_dial() {
    let site = Site::new();
    let files = Files::new();
    let ca = files.write("site.pem", site.ca.pem());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    for dates in [(2000, 2001), (4090, 4091)] {
        let leaf = site.leaf("cli", ExtendedKeyUsagePurpose::ClientAuth, Some(dates));
        let mut cmd = cli(&files, &url, &leaf);
        cmd.arg("--sc-ca").arg(&ca);
        let output = Process::spawn(read(&mut cmd)).output().await;
        failure(
            &output,
            "SC credential check refused the dial before connecting: Operational certificate [0]: \
             The identity leaf has expired, is not valid yet",
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "a {dates:?} leaf reached TCP"
        );
    }
}

/// A CA file past the lint's inspection limits (here 65 CAs) is one the
/// TLS configuration accepts: the lint warns that it did not assess it and
/// the command still dials (#968).
#[tokio::test]
async fn a_ca_file_past_the_lint_limits_still_dials() {
    let site = Site::new();
    let leaf = site.leaf("cli", ExtendedKeyUsagePurpose::ClientAuth, None);
    let files = Files::new();
    let mut bundle = site.ca.pem();
    for _ in 0..64 {
        bundle.push_str(&Site::new().ca.pem());
    }
    let ca = files.write("big bundle.pem", bundle);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let mut cmd = cli(&files, &url, &leaf);
    cmd.arg("--sc-ca").arg(ca);
    let (output, ()) = tokio::join!(
        Process::spawn(read(&mut cmd)).output(),
        bounded(async {
            drop(listener.accept().await.unwrap());
        })
    );
    failure(&output, "TLS handshake");
    failure(
        &output,
        "not assessed; the files exceed the inspection limits",
    );
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
