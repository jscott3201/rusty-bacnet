//! External-consumer acceptance of strict TLS policy across public hub startup.
#![cfg(feature = "sc-tls")]

mod hub_tls_support;

use std::{panic::AssertUnwindSafe, time::Duration};

use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use futures_util::{FutureExt, StreamExt};
use hub_tls_support::*;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
use tokio::{io::AsyncReadExt, net::TcpStream};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;

#[test]
fn typed_config_rejects_empty_ca_before_startup() {
    let fixture = Fixture::new();
    let error = ScHubTlsConfig::from_der(vec![], vec![fixture.server.cert], fixture.server.key)
        .unwrap_err();
    assert!(
        matches!(error, bacnet_types::error::Error::Encoding(ref message) if message == "no CA certificates found")
    );
}

#[test]
fn typed_config_validates_all_der_and_key_inputs() {
    let f = Fixture::new();
    let bad = CertificateDer::from(vec![1, 2, 3]);
    let cases = [
        (
            vec![f.ca.clone()],
            vec![],
            f.server.key.clone_key(),
            "no server certificates found",
        ),
        (
            vec![bad.clone()],
            vec![f.server.cert.clone()],
            f.server.key.clone_key(),
            "failed to add CA cert:",
        ),
        (
            vec![f.ca.clone(), bad.clone()],
            vec![f.server.cert.clone()],
            f.server.key.clone_key(),
            "failed to add CA cert:",
        ),
        (
            vec![bad.clone(), f.ca.clone()],
            vec![f.server.cert.clone()],
            f.server.key.clone_key(),
            "failed to add CA cert:",
        ),
        (
            vec![f.ca.clone()],
            vec![bad.clone()],
            f.server.key.clone_key(),
            "TLS server config error:",
        ),
        (
            vec![f.ca.clone()],
            vec![f.server.cert.clone(), bad],
            f.server.key.clone_key(),
            "TLS server config error:",
        ),
        (
            vec![f.ca.clone()],
            vec![f.server.cert.clone()],
            PrivatePkcs8KeyDer::from(vec![1, 2, 3]).into(),
            "TLS server config error:",
        ),
        (
            vec![f.ca.clone()],
            vec![f.server.cert.clone()],
            f.good.key.clone_key(),
            "TLS server config error:",
        ),
    ];
    for (ca, chain, key, expected) in cases {
        let error = ScHubTlsConfig::from_der(ca, chain, key).unwrap_err();
        assert!(
            matches!(error, bacnet_types::error::Error::Encoding(ref message) if message.starts_with(expected)),
            "{error}"
        );
    }
    let config = typed_config(&f);
    assert_eq!(format!("{config:?}"), "ScHubTlsConfig { .. }");
    assert_eq!(format!("{:?}", config.clone()), "ScHubTlsConfig { .. }");
    // Preflight is not local issuer/date certification: the remote peer decides.
    for id in [&f.wrong, &f.expired, &f.future] {
        ScHubTlsConfig::from_der(
            vec![f.ca.clone()],
            vec![id.cert.clone()],
            id.key.clone_key(),
        )
        .unwrap();
    }
}

fn typed_config(f: &Fixture) -> ScHubTlsConfig {
    ScHubTlsConfig::from_der(
        vec![f.ca.clone()],
        vec![f.server.cert.clone(), f.ca.clone()],
        f.server.key.clone_key(),
    )
    .unwrap()
}

async fn typed_hub(f: &Fixture, timeouts: ScHubHandshakeTimeouts) -> ScHub {
    bounded(ScHub::start_with_tls_config(
        "127.0.0.1:0",
        typed_config(f).clone(),
        HUB_VMAC,
        HUB_UUID,
        timeouts,
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn typed_hub_mutual_tls13_connect_relay_and_peer_denials() {
    let f = Fixture::new();
    let hub = typed_hub(&f, ScHubHandshakeTimeouts::default()).await;
    assert_mutual_tls13_connect_relay_and_peer_denials(&f, hub, HUB_UUID).await;
}

async fn assert_mutual_tls13_connect_relay_and_peer_denials(
    f: &Fixture,
    hub: ScHub,
    uuid: [u8; 16],
) {
    let outcome = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        let mut sender = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let mut recipient = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        assert_eq!(sender.get_ref().get_ref().1.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
        connect(&mut sender, 1, uuid).await;
        connect(&mut recipient, 2, uuid).await;
        relay(&mut sender, &mut recipient, 0).await;
        let cases = [
            (None, &rustls::version::TLS13, rustls::AlertDescription::CertificateRequired),
            (Some(&f.wrong), &rustls::version::TLS13, rustls::AlertDescription::UnknownCA),
            (Some(&f.expired), &rustls::version::TLS13, rustls::AlertDescription::CertificateExpired),
            (Some(&f.future), &rustls::version::TLS13, rustls::AlertDescription::CertificateExpired),
            (Some(&f.good), &rustls::version::TLS12, rustls::AlertDescription::ProtocolVersion),
        ];
        for (i, (id, version, alert)) in cases.into_iter().enumerate() {
            let tcp = bounded(TcpStream::connect(address)).await.unwrap();
            let connector = TlsConnector::from(f.client(id, version));
            let result = bounded(connector.connect(ServerName::try_from("localhost").unwrap(), tcp)).await;
            // TLS 1.3 client completion may precede the server's client-cert
            // rejection. Read the actual alert, never accept timeout/EOF as proof.
            let error = match result {
                Err(error) => error,
                Ok(mut tls) => bounded(tls.read(&mut [0; 1])).await.unwrap_err(),
            };
            assert!(matches!(error.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()), Some(rustls::Error::AlertReceived(actual)) if *actual == alert), "expected {alert:?}, got {error:?}");
            relay(&mut sender, &mut recipient, i as u8 + 1).await;
        }
    }).catch_unwind().await;
    finish(hub, outcome).await;
}

#[tokio::test]
async fn typed_hub_honors_each_timeout_and_preserves_established_peers() {
    let f = Fixture::new();
    let budgets = ScHubHandshakeTimeouts::new(
        Duration::from_millis(400),
        Duration::from_millis(500),
        Duration::from_secs(5),
    )
    .unwrap();
    let hub = typed_hub(&f, budgets).await;
    let outcome = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        let mut sender = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let mut recipient =
            websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        connect(&mut sender, 1, HUB_UUID).await;
        connect(&mut recipient, 2, HUB_UUID).await;
        let mut silent = bounded(TcpStream::connect(address)).await.unwrap();
        // These are deadline oracles, not TLS authentication oracles. The 3s
        // bound distinguishes our short budgets from the 10s defaults.
        assert!(matches!(
            bounded(silent.read(&mut [0; 1])).await,
            Ok(0) | Err(_)
        ));
        let tcp = bounded(TcpStream::connect(address)).await.unwrap();
        let mut tls = bounded(
            TlsConnector::from(f.client(Some(&f.good), &rustls::version::TLS13))
                .connect(ServerName::try_from("localhost").unwrap(), tcp),
        )
        .await
        .unwrap();
        assert!(matches!(
            bounded(tls.read(&mut [0; 1])).await,
            Ok(0) | Err(_)
        ));
        let mut idle = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let closed = tokio::time::timeout(Duration::from_secs(6), idle.next())
            .await
            .unwrap();
        assert!(matches!(closed, Some(Ok(Message::Close(_)))), "{closed:?}");
        relay(&mut sender, &mut recipient, 1).await;
    })
    .catch_unwind()
    .await;
    finish(hub, outcome).await;
}

#[tokio::test]
async fn strict_hub_start_family_requires_mutual_tls13_and_preserves_uuid() {
    let fixture = Fixture::new();
    let config = typed_config(&fixture);
    for api in 0..3 {
        let hub = bounded(async {
            match api {
                0 => ScHub::start("127.0.0.1:0", config.clone(), HUB_VMAC).await,
                1 => {
                    ScHub::start_with_uuid("127.0.0.1:0", config.clone(), HUB_VMAC, HUB_UUID).await
                }
                _ => {
                    ScHub::start_with_uuid_and_timeouts(
                        "127.0.0.1:0",
                        config.clone(),
                        HUB_VMAC,
                        HUB_UUID,
                        ScHubHandshakeTimeouts::default(),
                    )
                    .await
                }
            }
        })
        .await
        .unwrap();
        assert_mutual_tls13_connect_relay_and_peer_denials(
            &fixture,
            hub,
            if api == 0 { [0; 16] } else { HUB_UUID },
        )
        .await;
    }
}
