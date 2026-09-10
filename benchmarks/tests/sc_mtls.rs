//! Integration tests for BACnet/SC mTLS enforcement (ASHRAE 135-2020 Annex AB.3).

use std::sync::Arc;
use std::time::Duration;

use bacnet_benchmarks::sc_helpers::*;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::{ScConnectionState, ScTransport};
use bacnet_transport::sc_hub::ScHubTlsConfig;
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::error::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{TlsAcceptor, TlsConnector};

async fn assert_connect_fails(url: &str, tls_config: Arc<rustls::ClientConfig>, message: &str) {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        TlsWebSocket::connect(url, tls_config),
    )
    .await
    .expect("TLS/WebSocket connect attempt timed out");

    assert!(result.is_err(), "{message}");
}

/// mTLS connection succeeds when the client presents a valid certificate
/// signed by the CA that the hub trusts.
#[tokio::test]
async fn sc_mtls_connection_succeeds() {
    let certs = generate_test_certs();
    let hub_vmac = [0x10; 6];
    let client_vmac = [0x01; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    // Connect with mTLS client config (presents client cert).
    let tls_config = make_client_tls_config_mtls(&certs);
    let ws = TlsWebSocket::connect(&url, tls_config).await.unwrap();
    let mut transport = ScTransport::new(ws, client_vmac);
    let _rx = transport.start().await.unwrap();

    // Verify connected state.
    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    drop(c);

    transport.stop().await.unwrap();
    hub.stop().await;
}

/// mTLS hub rejects a client that does NOT present a client certificate.
/// The TLS handshake should fail because the server requires client auth.
#[tokio::test]
async fn sc_mtls_rejects_unauthenticated_client() {
    let certs = generate_test_certs();
    let hub_vmac = [0x10; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    // Connect WITHOUT a client certificate (standard non-mTLS config).
    let tls_config = make_client_tls_config(&certs);
    let result = TlsWebSocket::connect(&url, tls_config).await;

    // Should fail because the hub requires a client cert.
    assert!(
        result.is_err(),
        "Expected TLS handshake to fail without client cert"
    );

    hub.stop().await;
}

/// The mTLS convenience helpers (`start_sc_hub_mtls` / `make_sc_transport_mtls`)
/// produce a working end-to-end connection.
#[tokio::test]
async fn sc_mtls_helpers_roundtrip() {
    let certs = generate_test_certs();
    let hub_vmac = [0xF0; 6];
    let client_vmac = [0x02; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;
    let mut transport = make_sc_transport_mtls(&url, &certs, client_vmac).await;
    let _rx = transport.start().await.unwrap();

    let conn = transport.connection().unwrap();
    let c = conn.lock().await;
    assert_eq!(c.state, ScConnectionState::Connected);
    drop(c);

    transport.stop().await.unwrap();
    hub.stop().await;
}

/// The generated test configs negotiate TLS 1.3 for BACnet/SC.
#[tokio::test]
async fn sc_tls_negotiates_tls13() {
    let certs = generate_test_certs();
    let acceptor = TlsAcceptor::from(make_server_tls_config_mtls(&certs));
    let connector = TlsConnector::from(make_client_tls_config_mtls(&certs));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await.unwrap();
        stream.get_ref().1.protocol_version()
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let server_name = ServerName::try_from("localhost").unwrap().to_owned();
    let stream = connector.connect(server_name, stream).await.unwrap();

    assert_eq!(
        stream.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    assert_eq!(
        server.await.unwrap(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
}

/// BACnet/SC rejects peers that can only negotiate TLS 1.2.
#[tokio::test]
async fn sc_tls_rejects_tls12_only_client() {
    let certs = generate_test_certs();
    let hub_vmac = [0x20; 6];

    let (mut hub, _) = start_sc_hub_mtls(&certs, hub_vmac).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let tcp = TcpStream::connect(hub.local_addr().unwrap()).await.unwrap();
        TlsConnector::from(make_client_tls12_config(&certs))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
    })
    .await;
    hub.stop().await;
    let error = result
        .expect("TLS version negotiation timed out")
        .unwrap_err();
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|e| e.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::AlertReceived(
                rustls::AlertDescription::ProtocolVersion
            ))
        ),
        "expected protocol-version alert, not client-auth refusal: {error:?}"
    );
}

/// mTLS hub rejects a client certificate signed by a CA that the hub does not trust.
#[tokio::test]
async fn sc_mtls_rejects_client_cert_from_wrong_ca() {
    let hub_certs = generate_test_certs();
    let wrong_client_certs = generate_test_certs();
    let hub_vmac = [0x30; 6];

    let (mut hub, url) = start_sc_hub_mtls(&hub_certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls_with_client_identity(&hub_certs, &wrong_client_certs),
        "Expected mTLS hub to reject client cert signed by an untrusted CA",
    )
    .await;

    hub.stop().await;
}

/// mTLS hub rejects an expired client certificate.
#[tokio::test]
async fn sc_mtls_rejects_expired_client_cert() {
    let certs = generate_test_certs_with_expired_client();
    let hub_vmac = [0x40; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls(&certs),
        "Expected mTLS hub to reject an expired client certificate",
    )
    .await;

    hub.stop().await;
}

/// mTLS hub rejects a client certificate that is not valid yet.
#[tokio::test]
async fn sc_mtls_rejects_not_yet_valid_client_cert() {
    let certs = generate_test_certs_with_not_yet_valid_client();
    let hub_vmac = [0x50; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls(&certs),
        "Expected mTLS hub to reject a not-yet-valid client certificate",
    )
    .await;

    hub.stop().await;
}

/// BACnet/SC clients reject an expired hub/server certificate.
#[tokio::test]
async fn sc_tls_rejects_expired_server_cert() {
    let certs = generate_test_certs_with_expired_server();
    let hub_vmac = [0x60; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls(&certs),
        "Expected BACnet/SC client to reject an expired server certificate",
    )
    .await;

    hub.stop().await;
}

/// BACnet/SC clients reject a hub/server certificate that is not valid yet.
#[tokio::test]
async fn sc_tls_rejects_not_yet_valid_server_cert() {
    let certs = generate_test_certs_with_not_yet_valid_server();
    let hub_vmac = [0x70; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls(&certs),
        "Expected BACnet/SC client to reject a not-yet-valid server certificate",
    )
    .await;

    hub.stop().await;
}

/// BACnet/SC clients reject a hub/server certificate whose SAN does not match.
#[tokio::test]
async fn sc_tls_rejects_wrong_server_name() {
    let certs = generate_test_certs_with_wrong_server_name();
    let hub_vmac = [0x80; 6];

    let (mut hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

    assert_connect_fails(
        &url,
        make_client_tls_config_mtls(&certs),
        "Expected BACnet/SC client to reject a server certificate with the wrong SAN",
    )
    .await;

    hub.stop().await;
}

#[test]
fn sc_tls_config_rejects_malformed_cert_or_key() {
    let mut certs = generate_test_certs();
    certs.server_cert_pem = "not a certificate".into();
    assert!(try_make_server_tls_config_mtls(&certs).is_err());

    let mut certs = generate_test_certs();
    certs.server_key_pem = "not a private key".into();
    assert!(try_make_server_tls_config_mtls(&certs).is_err());

    let mut certs = generate_test_certs();
    certs.ca_cert_pem = "not a CA certificate".into();
    assert!(try_make_server_tls_config_mtls(&certs).is_err());
    assert!(try_make_client_tls_config_mtls(&certs).is_err());

    let mut certs = generate_test_certs();
    certs.client_cert_pem = "not a client certificate".into();
    assert!(try_make_client_tls_config_mtls(&certs).is_err());

    let mut certs = generate_test_certs();
    certs.client_key_pem = "not a client private key".into();
    assert!(try_make_client_tls_config_mtls(&certs).is_err());
}

#[test]
fn sc_tls_config_rejects_mismatched_cert_key_pairs() {
    let mut certs = generate_test_certs();
    let other = generate_test_certs();
    certs.server_key_pem = other.server_key_pem;
    assert!(try_make_server_tls_config_mtls(&certs).is_err());

    let mut certs = generate_test_certs();
    let other = generate_test_certs();
    certs.client_key_pem = other.client_key_pem;
    assert!(try_make_client_tls_config_mtls(&certs).is_err());
}

// These are synchronous PEM/configuration tests: no runtime, socket or hub bind.
// AQID is the deliberately invalid DER byte sequence [1, 2, 3], not a credential.
const INVALID_CERT_DER_PEM: &str = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
const MALFORMED_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----\n";

fn assert_hub_config_error(certs: &CertMaterial, expected: &str) {
    let error = try_make_hub_tls_config(certs).unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(message) if message.starts_with(expected)),
        "expected {expected:?}, got {error}"
    );
}

#[test]
fn sc_hub_pem_config_owns_credentials_and_preserves_raw_helper_types() {
    let mut certs = generate_test_certs();
    // Valid multi-certificate material must not be mistaken for malformed tails.
    certs.server_cert_pem.push_str(&certs.ca_cert_pem);
    certs
        .ca_cert_pem
        .push_str(&generate_test_certs().ca_cert_pem);
    let config: Result<ScHubTlsConfig, Error> = try_make_hub_tls_config(&certs);
    let config = config.unwrap();
    // Independent TLS acceptors keep their existing, non-sealed return types.
    let _: Arc<rustls::ServerConfig> = make_server_tls_config_mtls(&certs);
    let raw: Result<Arc<rustls::ServerConfig>, String> = try_make_server_tls_config_mtls(&certs);
    raw.unwrap();
    drop(certs);
    assert_eq!(format!("{config:?}"), "ScHubTlsConfig { .. }");
}

#[test]
fn sc_hub_pem_config_rejects_empty_or_malformed_ca() {
    let mut certs = generate_test_certs();
    let valid = certs.ca_cert_pem.clone();
    for invalid in ["", "not a CA certificate"] {
        certs.ca_cert_pem = invalid.into();
        assert_hub_config_error(&certs, "no CA certificates found");
    }
    for invalid in [
        MALFORMED_CERT_PEM.to_owned(),
        format!("{valid}{MALFORMED_CERT_PEM}"),
        format!("{MALFORMED_CERT_PEM}{valid}"),
    ] {
        certs.ca_cert_pem = invalid;
        assert_hub_config_error(&certs, "failed to parse CA certificates:");
    }
}

#[test]
fn sc_hub_pem_config_rejects_empty_or_malformed_server_chain() {
    let mut certs = generate_test_certs();
    let valid = certs.server_cert_pem.clone();
    for invalid in ["", "not a server certificate"] {
        certs.server_cert_pem = invalid.into();
        assert_hub_config_error(&certs, "no server certificates found");
    }
    for invalid in [
        MALFORMED_CERT_PEM.to_owned(),
        format!("{valid}{MALFORMED_CERT_PEM}"),
        format!("{MALFORMED_CERT_PEM}{valid}"),
    ] {
        certs.server_cert_pem = invalid;
        assert_hub_config_error(&certs, "failed to parse server certificates:");
    }
}

#[test]
fn sc_hub_pem_config_rejects_empty_or_malformed_key() {
    let mut certs = generate_test_certs();
    for invalid in [
        "",
        "not a private key",
        "-----BEGIN PRIVATE KEY-----\n!\n-----END PRIVATE KEY-----\n",
    ] {
        certs.server_key_pem = invalid.into();
        assert_hub_config_error(&certs, "failed to parse server key:");
    }
}

#[test]
fn sc_hub_pem_config_rejects_invalid_der_in_any_certificate_position() {
    let mut certs = generate_test_certs();
    let valid_ca = certs.ca_cert_pem.clone();
    for invalid in [
        INVALID_CERT_DER_PEM.to_owned(),
        format!("{valid_ca}{INVALID_CERT_DER_PEM}"),
        format!("{INVALID_CERT_DER_PEM}{valid_ca}"),
    ] {
        certs.ca_cert_pem = invalid;
        assert_hub_config_error(&certs, "failed to add CA cert:");
    }
    certs.ca_cert_pem = valid_ca;
    let valid_chain = certs.server_cert_pem.clone();
    for invalid in [
        INVALID_CERT_DER_PEM.to_owned(),
        format!("{valid_chain}{INVALID_CERT_DER_PEM}"),
        format!("{INVALID_CERT_DER_PEM}{valid_chain}"),
    ] {
        certs.server_cert_pem = invalid;
        assert_hub_config_error(&certs, "TLS server config error:");
    }
}

#[test]
fn sc_hub_pem_config_rejects_unusable_or_mismatched_key() {
    let mut certs = generate_test_certs();
    certs.server_key_pem = "-----BEGIN PRIVATE KEY-----\nAQID\n-----END PRIVATE KEY-----\n".into();
    assert_hub_config_error(&certs, "TLS server config error:");
    certs.server_key_pem = generate_test_certs().server_key_pem;
    assert_hub_config_error(
        &certs,
        "TLS server config error: keys may not be consistent: KeyMismatch",
    );
}
