//! Certificate observations for an existing configured SC client.
//! The client integration test compiles and exercises both functions.
#![allow(clippy::print_stdout, clippy::print_stderr)] // Explicit display example.

use bacnet_client::client::BACnetClient;
use bacnet_transport::sc::{ScConnectError, ScHubPeerInfo, ScTransport};
use bacnet_transport::sc_tls::{ScUntrustedPeerInfo, TlsWebSocket};
use bacnet_types::error::Error;

pub async fn show_accepted(
    client: &BACnetClient<ScTransport<TlsWebSocket>>,
) -> Option<ScHubPeerInfo> {
    // None during disconnection/reconnect. A retained result is historical.
    let peer = client.transport().hub_peer_info().await?;
    let tls = peer.tls();
    println!(
        "connection={} handshake={:?}",
        tls.connection_id(),
        tls.handshake_kind()
    );
    print!("leaf SHA-256: ");
    for byte in tls.leaf_sha256() {
        print!("{byte:02x}");
    }
    println!();
    for certificate in tls.certificates() {
        if let Some(metadata) = certificate.metadata() {
            println!(
                "subject={} issuer={} not_after={}",
                metadata.subject(),
                metadata.issuer(),
                metadata.not_after_unix()
            );
        }
    }
    // Resumption can reuse cached certificates; this is not fresh validation
    // or a revocation verdict. certificate.der() explicitly exports bytes.
    Some(peer)
}

pub fn show_rejected(error: &Error) -> Option<ScUntrustedPeerInfo> {
    eprintln!("{error}"); // Preserve the original reason the attempt failed.
    let untrusted = ScConnectError::from_error(error)?.untrusted_peer_info()?;
    for certificate in untrusted.certificates() {
        if let Some(metadata) = certificate.metadata() {
            println!(
                "UNTRUSTED subject={} issuer={} not_after={}",
                metadata.subject(),
                metadata.issuer(),
                metadata.not_after_unix()
            );
        }
    }
    // No accepted connection or verified identity. None means unavailable at
    // the observation point, not proof that no certificate was sent.
    Some(untrusted.clone())
}
