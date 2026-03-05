//! Integration tests for BACnet/SC mTLS enforcement (ASHRAE 135-2020 Annex AB.3).

use bacnet_benchmarks::sc_helpers::*;
use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::{ScConnectionState, ScTransport};
use bacnet_transport::sc_tls::TlsWebSocket;

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
