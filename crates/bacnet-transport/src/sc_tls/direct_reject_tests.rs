//! A peer the direct listener refuses during TLS reads the alert, and its
//! active and pending slots come back afterwards (#950).
use super::*;
use crate::port_ownership::connect_loopback;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What a WebSocket client writes straight after its TLS 1.3 Finished.
const UPGRADE: &[u8] = b"GET /.bacnet/sc HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: dc.bsc.bacnet.org\r\n\r\n";

fn client_without_certificate(ca: &TestCa) -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.ca.clone()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

#[tokio::test]
async fn a_refused_tls_peer_reads_the_alert_and_its_slots_are_released() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let tcp = connect_loopback(listener.local_addr()).await;
    let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let connected = tokio::time::timeout(
        Duration::from_secs(3),
        client_without_certificate(&ca).connect(server_name, tcp),
    )
    .await
    .unwrap();
    // A TLS 1.3 client can finish before the listener checks its (empty)
    // certificate. It then sends the upgrade request, which the listener never
    // reads; the alert must still arrive rather than a reset.
    let error = match connected {
        Err(error) => error,
        Ok(mut tls) => match tls.write_all(UPGRADE).await {
            Err(error) => error,
            Ok(()) => tokio::time::timeout(Duration::from_secs(3), tls.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap_err(),
        },
    };
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|e| e.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRequired
            ))
        ),
        "expected the CertificateRequired alert, got {error:?}"
    );
    wait_counts(&listener, 0, 0).await;
    listener.stop().await;
}

#[tokio::test]
async fn a_raw_peer_that_never_closes_gets_fin_and_releases_its_slots() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let mut raw = connect_loopback(listener.local_addr()).await;
    raw.write_all(b"not a TLS record\r\n\r\n").await.unwrap();
    // The alert, then FIN: reading reaches end of stream, not a reset.
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut buf = [0u8; 256];
        while raw.read(&mut buf).await.unwrap() > 0 {}
    })
    .await
    .unwrap();
    // `raw` stays open, so the linger, not the peer, ends the drain.
    wait_counts(&listener, 0, 0).await;
    listener.stop().await;
}

/// The socket is dropped and the slots come back. Whether the byte cap or the
/// linger ends the drain isn't observable here; tls_reject's unit tests prove
/// the cap.
#[tokio::test]
async fn a_raw_peer_that_keeps_writing_is_dropped_and_releases_its_slots() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let mut raw = connect_loopback(listener.local_addr()).await;
    // The TLS accept waits for a ClientHello, so the slots are held now.
    wait_counts(&listener, 1, 1).await;
    let writer = tokio::spawn(async move {
        raw.write_all(b"not a TLS record\r\n\r\n").await.unwrap();
        let junk = [0x17u8; 4096];
        while raw.write_all(&junk).await.is_ok() {}
    });
    wait_counts(&listener, 0, 0).await;
    // The listener dropped the socket, so the writer's sends start failing.
    tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .unwrap()
        .unwrap();
    listener.stop().await;
}
