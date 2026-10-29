//! Native CLI / Rust mTLS hub and BACnet server acceptance tests.
#![cfg(feature = "sc-tls")]
#[allow(dead_code)]
mod support;

use futures_util::FutureExt;
use rcgen::ExtendedKeyUsagePurpose::ClientAuth;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use support::sc::{Leaf, Site, cli, read, with_fixture};
use support::{Files, Process, bounded, failure};

async fn read_value(files: &Files, url: &str, leaf: &Leaf, ca: &std::path::Path, after: bool) {
    let mut cmd = cli(files, url, leaf);
    if after {
        read(&mut cmd).arg("--sc-ca").arg(ca).arg("-v");
    } else {
        cmd.arg("--sc-ca").arg(ca);
        read(&mut cmd);
    }
    let output = Process::spawn(&mut cmd).output().await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE KEY"));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(
        json,
        serde_json::json!({
            "object": "DEVICE:123", "property": "OBJECT_NAME", "value": "\"CLI site device\"",
            "decoded": "CLI site device"
        })
    );
}

#[tokio::test]
async fn explicit_site_ca_read_property_and_untrusted_ca_rejection() {
    with_fixture(async |fixture| {
        let site = Site::new();
        fixture.start(&site).await;
        let files = Files::new();
        let ca = files.write("site CA café.pem", site.ca.pem());
        let leaf = site.leaf("cli", ClientAuth, None);
        // Both global positions, argv paths with spaces/non-ASCII, and real
        // ReadProperty JSON, with no OS trust-store installation.
        read_value(&files, &fixture.url, &leaf, &ca, false).await;
        let wrong = files.write("other site.pem", Site::new().ca.pem());
        let mut cmd = cli(&files, &fixture.url, &leaf);
        cmd.arg("--sc-ca").arg(wrong)
            // These are child-local: explicit trust must not fall back to them.
            .env("SSL_CERT_FILE", &ca).env("SSL_CERT_DIR", &files.0);
        let output = Process::spawn(read(&mut cmd)).output().await;
        failure(&output, "TLS handshake");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("UnknownIssuer"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        read_value(&files, &fixture.url, &leaf, &ca, true).await;
        // Multiple CA certificates are accepted, not just the first block.
        let bundle = files.write(
            "bundle.pem",
            format!("{}{}", Site::new().ca.pem(), site.ca.pem()),
        );
        read_value(&files, &fixture.url, &leaf, &bundle, false).await;
        // Preserve streaming CA error order: do not parse a later PEM block
        // before validating the earlier DER entry. Neither is a usable CA.
        let bad_der = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
        let bad_pem = "-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----\n";
        for (contents, expected) in [
            (
                format!("{bad_der}{bad_pem}"),
                "unusable certificate in --sc-ca PEM",
            ),
            (format!("{bad_pem}{bad_der}"), "failed to parse --sc-ca PEM"),
        ] {
            let path = files.write("ordered-invalid-ca.pem", contents);
            let mut cmd = cli(&files, &fixture.url, &leaf);
            cmd.arg("--sc-ca").arg(path);
            failure(&Process::spawn(read(&mut cmd)).output().await, expected);
        }
        read_value(&files, &fixture.url, &leaf, &ca, false).await;
    })
    .await;
}

/// A BACnet/SC target is the device's VMAC as hex octets (#963), not the
/// IPv4-and-port spelling of its six octets that `read` in the support
/// module still uses.
#[tokio::test]
async fn read_property_with_a_vmac_target() {
    with_fixture(async |fixture| {
        let site = Site::new();
        fixture.start(&site).await;
        let files = Files::new();
        let ca = files.write("site.pem", site.ca.pem());
        let leaf = site.leaf("cli", ClientAuth, None);
        for vmac in ["02:00:00:00:00:01", "020000000001"] {
            let mut cmd = cli(&files, &fixture.url, &leaf);
            cmd.arg("--sc-ca")
                .arg(&ca)
                .args(["read", vmac, "dev:123", "object-name"]);
            let output = Process::spawn(&mut cmd).output().await;
            assert!(
                output.status.success(),
                "{vmac}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                json,
                serde_json::json!({
                    "object": "DEVICE:123", "property": "OBJECT_NAME", "value": "\"CLI site device\"",
                    "decoded": "CLI site device"
                }),
                "{vmac}"
            );
        }
    })
    .await;
}

/// An expired or future leaf no longer reaches the hub: the pre-dial lint
/// refuses it (`sc_ca.rs`). A leaf from another site's CA is only a lint
/// warning, logged on stderr, so the hub still gets to refuse it.
#[tokio::test]
async fn hub_rejects_an_untrusted_operational_certificate() {
    with_fixture(async |fixture| {
        let site = Site::new();
        fixture.start(&site).await;
        let files = Files::new();
        let ca = files.write("site.pem", site.ca.pem());
        let good = site.leaf("cli", ClientAuth, None);
        let rogue = Site::new().leaf("rogue", ClientAuth, None);
        let mut cmd = cli(&files, &fixture.url, &rogue);
        cmd.arg("--sc-ca").arg(&ca);
        let output = Process::spawn(read(&mut cmd)).output().await;
        failure(&output, "WebSocket");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("timed out"));
        assert!(
            stderr.contains("signed the identity leaf directly"),
            "{stderr}"
        );
        read_value(&files, &fixture.url, &good, &ca, false).await;
    })
    .await;
}

#[tokio::test]
async fn peer_observes_tls_version_errors_not_timeouts() {
    let site = Site::new();
    let files = Files::new();
    let ca = files.write("site.pem", site.ca.pem());
    let leaf = site.leaf("cli", ClientAuth, None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    let acceptor = site.acceptor(&rustls::version::TLS12);
    let mut cmd = cli(&files, &url, &leaf);
    cmd.arg("--sc-ca").arg(&ca);
    let (output, error) = tokio::join!(
        Process::spawn(read(&mut cmd)).output(),
        bounded(async {
            let (tcp, _) = listener.accept().await.unwrap();
            match acceptor.accept(tcp).await {
                Ok(_) => panic!("invalid credentials/protocol accepted"),
                Err(error) => error,
            }
        })
    );
    failure(&output, "WebSocket");
    let peer_error = format!("{error:?}");
    assert!(peer_error.contains("PeerIncompatible"), "{peer_error}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("ProtocolVersion"));
}

#[tokio::test]
async fn hub_tls_verifier_requires_a_client_certificate() {
    // CLI omission is rejected pre-dial; a raw TLS peer independently verifies
    // that the test hub's verifier really requires client authentication.
    let site = Site::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let acceptor = site.acceptor(&rustls::version::TLS13);
    let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(site.roots())
        .with_no_client_auth();
    let (client, peer) = tokio::join!(
        bounded(async {
            use tokio::io::AsyncReadExt;
            let tcp = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let mut tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(
                    rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap(),
                    tcp,
                )
                .await?;
            // TLS 1.3 may deliver the server's client-auth alert after Finished.
            tls.read(&mut [0; 1]).await
        }),
        bounded(async {
            let (tcp, _) = listener.accept().await.unwrap();
            match acceptor.accept(tcp).await {
                Ok(_) => panic!("certificate-less peer admitted"),
                Err(error) => error,
            }
        })
    );
    assert!(format!("{:?}", client.err().unwrap()).contains("CertificateRequired"));
    assert!(format!("{peer:?}").contains("NoCertificatesPresented"));
}

#[tokio::test]
async fn panic_still_joins_hub_and_server() {
    let result = AssertUnwindSafe(with_fixture(async |fixture| {
        fixture.start(&Site::new()).await;
        panic!("injected fixture failure");
    }))
    .catch_unwind()
    .await;
    let panic = result.expect_err("injected panic must propagate after cleanup");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"injected fixture failure")
    );
}
