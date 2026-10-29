//! Successful-session observations against independent peers and actual hubs.
#![cfg(feature = "sc-tls")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::{ScConnectionState, ScReconnectConfig, ScTransport, WebSocketPort};
use bacnet_transport::sc_hub::{
    ScHub, ScHubCertificateBinding, ScHubCertificateBindings, ScHubTlsConfig,
};
use bacnet_transport::sc_tls::{ScHandshakeKind, ScNodeTlsConfig, TlsWebSocket};
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

struct Fixture {
    ca: CertificateDer<'static>,
    node: ScNodeTlsConfig,
    leaves: Vec<(CertificateDer<'static>, Vec<u8>)>,
}

impl Fixture {
    fn new() -> Self {
        let mut ca = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.distinguished_name = rcgen::DistinguishedName::new();
        ca.distinguished_name
            .push(rcgen::DnType::CommonName, "snapshot CA");
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let issuer = rcgen::Issuer::from_params(&ca, &key);
        let ca = ca.self_signed(&key).unwrap().der().clone();
        let leaves: Vec<_> = (0..4)
            .map(|index| {
                let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
                params.distinguished_name = rcgen::DistinguishedName::new();
                params
                    .distinguished_name
                    .push(rcgen::DnType::CommonName, format!("peer {index}"));
                let key = rcgen::KeyPair::generate().unwrap();
                (
                    params.signed_by(&key, &issuer).unwrap().der().clone(),
                    key.serialize_der(),
                )
            })
            .collect();
        let node = ScNodeTlsConfig::from_der(
            vec![ca.clone()],
            vec![leaves[0].0.clone()],
            PrivatePkcs8KeyDer::from(leaves[0].1.clone()).into(),
        )
        .unwrap();
        Self { ca, node, leaves }
    }
    fn hub_config(&self, index: usize) -> ScHubTlsConfig {
        ScHubTlsConfig::from_der(
            vec![self.ca.clone()],
            vec![self.leaves[index].0.clone()],
            PrivatePkcs8KeyDer::from(self.leaves[index].1.clone()).into(),
        )
        .unwrap()
    }
    async fn hub(&self, index: usize) -> ScHub {
        ScHub::start("127.0.0.1:0", self.hub_config(index), [0x10; 6], [0x10; 16])
            .await
            .unwrap()
    }
}

fn url(hub: &ScHub) -> String {
    format!("wss://localhost:{}", hub.local_addr().unwrap().port())
}

#[tokio::test]
async fn exact_order_omitted_anchor_and_unparseable_extra_do_not_change_tls_acceptance() {
    let fixture = Fixture::new();
    // The root is omitted. The unrelated extra and malformed extra are not
    // needed for the direct trusted-issuer path; retain their original bytes.
    let chain = vec![
        fixture.leaves[1].0.clone(),
        fixture.leaves[2].0.clone(),
        CertificateDer::from(vec![0xff, 0x00, 0x01]),
    ];
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(
            chain.clone(),
            PrivatePkcs8KeyDer::from(fixture.leaves[1].1.clone()).into(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(tcp)
            .await
            .unwrap();
        #[allow(clippy::result_large_err)]
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tls,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "hub.bsc.bacnet.org".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        // A second dial would be an observable violation while getters run.
        let got = tokio::select! {
            message = ws.next() => message.unwrap().unwrap(),
            _ = listener.accept() => panic!("inspection opened a second connection"),
        };
        assert_eq!(got.into_data(), b"inspection complete"[..]);
        ws.send(Message::Binary(b"done".to_vec().into()))
            .await
            .unwrap();
    });
    let ws = TlsWebSocket::connect(&format!("wss://localhost:{}", address.port()), fixture.node)
        .await
        .unwrap();
    let info = ws.peer_info().clone();
    for _ in 0..10 {
        assert!(Arc::ptr_eq(&info, ws.peer_info()));
        assert_eq!(info.peer_address(), address);
        assert_eq!(info.handshake_kind(), ScHandshakeKind::Full);
        assert_eq!(info.certificates().len(), 3);
        for (actual, expected) in info.certificates().iter().zip(&chain) {
            assert_eq!(actual.der(), expected.as_ref());
        }
    }
    let meta = info.certificates()[0].metadata().unwrap();
    assert_eq!(meta.subject(), "CN=peer 1");
    assert_eq!(meta.issuer(), "CN=snapshot CA");
    assert!(info.certificates()[2].metadata().is_none());
    assert_eq!(
        info.leaf_sha256().as_slice(),
        aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &chain[0]).as_ref()
    );
    ws.send(b"inspection complete").await.unwrap();
    assert_eq!(ws.recv().await.unwrap(), b"done");
    peer.await.unwrap();
    drop(ws);
    assert_eq!(info.certificates()[0].der(), chain[0].as_ref());
}

async fn wait_peer(transport: &ScTransport<TlsWebSocket>, expected: std::net::SocketAddr) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if transport
                .hub_peer_info()
                .await
                .is_some_and(|info| info.tls().peer_address() == expected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("expected accepted active peer");
}

#[tokio::test]
async fn same_socket_sc_retry_retains_the_physical_connection_id() {
    let fixture = Fixture::new();
    let config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(
            vec![fixture.leaves[1].0.clone()],
            PrivatePkcs8KeyDer::from(fixture.leaves[1].1.clone()).into(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (disconnect, signal) = tokio::sync::oneshot::channel();
    let (retry, retried) = tokio::sync::oneshot::channel();
    let (finish, finished) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(tcp)
            .await
            .unwrap();
        #[allow(clippy::result_large_err)]
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tls,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "hub.bsc.bacnet.org".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        async fn admit(
            ws: &mut tokio_tungstenite::WebSocketStream<
                tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
            >,
        ) {
            let request = loop {
                let request = ws.next().await.unwrap().unwrap().into_data();
                if request[0] == 6 {
                    break request;
                }
                assert_eq!(request[0], 10); // Existing retry starts after heartbeat expiry.
            };
            let mut accept = vec![7, 0, request[2], request[3]];
            accept.extend_from_slice(&[0x10; 6]);
            accept.extend_from_slice(&[0x10; 16]);
            accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
            ws.send(Message::Binary(accept.into())).await.unwrap();
        }
        admit(&mut ws).await;
        signal.await.unwrap();
        ws.send(Message::Binary(vec![8, 0, 0x55, 0x22].into()))
            .await
            .unwrap();
        // Startup may schedule a heartbeat before the forced disconnect.
        loop {
            let data = ws.next().await.unwrap().unwrap().into_data();
            if data[0] == 9 {
                assert_eq!(&data[..], &[9, 0, 0x55, 0x22]);
                break;
            }
            assert_eq!(data[0], 10);
        }
        tokio::select! {
            () = admit(&mut ws) => {},
            _ = listener.accept() => panic!("SC retry unexpectedly redialed"),
        }
        retry.send(()).unwrap();
        finished.await.unwrap();
    });
    let ws = TlsWebSocket::connect(&format!("wss://localhost:{}", address.port()), fixture.node)
        .await
        .unwrap();
    let id = ws.peer_info().connection_id();
    let mut transport = ScTransport::new(ws, [0x22; 6])
        .with_device_uuid([0x22; 16])
        .with_heartbeat_interval_ms(3_000)
        .with_heartbeat_timeout_ms(3_001)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100,
            max_retries: 1,
        });
    let _rx = transport.start().await.unwrap();
    let old = transport.hub_peer_info().await.unwrap();
    let mut state = transport.connection_state_changes();
    disconnect.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while *state.borrow_and_update() == ScConnectionState::Connected {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(transport.hub_peer_info().await.is_none());
    tokio::time::timeout(Duration::from_secs(8), retried)
        .await
        .unwrap()
        .unwrap();
    wait_peer(&transport, address).await;
    assert_eq!(
        transport
            .hub_peer_info()
            .await
            .unwrap()
            .tls()
            .connection_id(),
        id
    );
    assert_eq!(old.tls().connection_id(), id);
    transport.stop().await.unwrap();
    finish.send(()).unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn active_snapshot_follows_failover_failed_candidate_and_primary_restoration() {
    let fixture = Fixture::new();
    let mut primary = fixture.hub(1).await;
    let mut failover = fixture.hub(2).await;
    let mut replacement = fixture.hub(3).await;
    let rejected = fixture.hub_config(1).with_certificate_bindings(
        ScHubCertificateBindings::new(vec![
            ScHubCertificateBinding::new([0x55; 16], vec![[0x55; 6]], vec![[0x55; 32]]).unwrap(),
        ])
        .unwrap(),
    );
    let mut rejected = ScHub::start("127.0.0.1:0", rejected, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let selected = Arc::new(Mutex::new(url(&primary)));
    let dial_target = selected.clone();
    let policy = fixture.node.clone();
    let fallback_url = url(&failover);
    let fallback_policy = fixture.node.clone();
    let fallback_gate = Arc::new(tokio::sync::Notify::new());
    let dial_gate = fallback_gate.clone();
    let ws = TlsWebSocket::connect(&url(&primary), fixture.node.clone())
        .await
        .unwrap();
    let id = ws.peer_info().connection_id();
    let mut transport = ScTransport::new(ws, [0x22; 6])
        .with_device_uuid([0x22; 16])
        .with_connect_timeout_ms(300)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 80,
            max_delay_ms: 80,
            max_retries: 1,
        })
        .with_connector(move || {
            let url = dial_target.lock().unwrap().clone();
            let policy = policy.clone();
            async move { TlsWebSocket::connect(&url, policy).await }
        })
        .with_failover_connector(move || {
            let url = fallback_url.clone();
            let policy = fallback_policy.clone();
            let gate = dial_gate.clone();
            async move {
                gate.notified().await;
                TlsWebSocket::connect(&url, policy).await
            }
        });
    assert!(transport.hub_peer_info().await.is_none());
    let _rx = transport.start().await.unwrap();
    let original = transport.hub_peer_info().await.unwrap();
    assert_eq!(original.tls().connection_id(), id);
    assert_eq!(
        original.tls().certificates()[0].der(),
        fixture.leaves[1].0.as_ref()
    );
    let mut state = transport.connection_state_changes();
    primary.stop().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while *state.borrow_and_update() == ScConnectionState::Connected {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(transport.hub_peer_info().await.is_none());
    fallback_gate.notify_one();
    wait_peer(&transport, failover.local_addr().unwrap()).await;
    let fallback = transport.hub_peer_info().await.unwrap();
    assert_ne!(fallback.tls().connection_id(), id);
    assert_ne!(fallback.tls().leaf_sha256(), original.tls().leaf_sha256());
    assert_eq!(
        fallback.tls().certificates()[0].der(),
        fixture.leaves[2].0.as_ref()
    );

    // A real, trusted TLS/WebSocket candidate is denied by SC leaf binding.
    *selected.lock().unwrap() = url(&rejected);
    tokio::time::timeout(Duration::from_secs(3), async {
        while rejected.status().await.admin_denied == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        transport
            .hub_peer_info()
            .await
            .unwrap()
            .tls()
            .connection_id(),
        fallback.tls().connection_id()
    );
    *selected.lock().unwrap() = url(&replacement);
    wait_peer(&transport, replacement.local_addr().unwrap()).await;
    let restored = transport.hub_peer_info().await.unwrap();
    assert_eq!(
        restored.tls().certificates()[0].der(),
        fixture.leaves[3].0.as_ref()
    );
    assert_ne!(
        restored.tls().connection_id(),
        fallback.tls().connection_id()
    );
    assert_eq!(
        original.tls().certificates()[0].der(),
        fixture.leaves[1].0.as_ref()
    );
    transport.stop().await.unwrap();
    assert!(transport.hub_peer_info().await.is_none());
    drop(transport);
    assert_eq!(
        restored.tls().certificates()[0].der(),
        fixture.leaves[3].0.as_ref()
    );
    failover.stop().await;
    rejected.stop().await;
    replacement.stop().await;
}
