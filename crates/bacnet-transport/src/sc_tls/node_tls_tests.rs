use super::{tests::test_tls_pair, *};
use rustls::{HandshakeKind, ProtocolVersion};
use std::time::Duration;
use tokio_rustls::TlsAcceptor;

#[path = "connect_accept_tests.rs"]
mod connect_accept_tests;

#[path = "dial_failure_tests.rs"]
mod dial_failure_tests;

#[path = "mu_liveness_tests.rs"]
mod mu_liveness_tests;

#[path = "rejection_deadline_tests.rs"]
mod rejection_deadline_tests;

async fn observe_connections(rounds: usize) -> Vec<HandshakeKind> {
    // An independent trusted server, deliberately NOT ScHub: no client verifier
    // and no CertificateRequest. This characterizes the local contract's limit.
    let (node, server) = test_tls_pair();
    assert!(server.send_tls13_tickets > 0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = TlsAcceptor::from(server);
    let rejected_acceptor = TlsAcceptor::from(test_tls_pair().1);
    tokio::time::timeout(Duration::from_secs(5), async {
        let accept = async {
            let mut kinds = Vec::new();
            if rounds == 2 {
                let (tcp, _) = listener.accept().await.unwrap();
                assert!(rejected_acceptor.accept(tcp).await.is_err());
            }
            for _ in 0..rounds {
                let (tcp, _) = listener.accept().await.unwrap();
                let tls = acceptor.accept(tcp).await.unwrap();
                assert_eq!(tls.get_ref().1.protocol_version(), Some(ProtocolVersion::TLSv1_3));
                assert!(tls.get_ref().1.peer_certificates().is_none());
                kinds.push(tls.get_ref().1.handshake_kind().unwrap());
                // The handshake callback signature is fixed by tungstenite.
                #[allow(clippy::result_large_err)]
                let mut ws = tokio_tungstenite::accept_hdr_async(tls,
                    |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        response.headers_mut().insert("Sec-WebSocket-Protocol", BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap());
                        Ok(response)
                    }).await.unwrap();
                ws.send(Message::Binary(b"ticket barrier".to_vec().into())).await.unwrap();
                assert_eq!(ws.next().await.unwrap().unwrap().into_data(), b"node reply"[..]);
                ws.send(Message::Binary(b"done".to_vec().into())).await.unwrap();
            }
            kinds
        };
        let connect = async {
            if rounds == 2 {
                // Rejection on this shared policy cannot contaminate the
                // subsequent full or resumed successful observations.
                let error = TlsWebSocket::connect(&url, node.clone()).await.err().unwrap();
                assert!(error.to_string().contains("invalid peer certificate"));
                let diagnostic = ScConnectError::from_error(&error).unwrap().untrusted_peer_info().unwrap();
                assert!(!diagnostic.certificates().is_empty());
            }
            let mut previous: Option<std::sync::Arc<super::ScTlsPeerInfo>> = None;
            for round in 0..rounds {
                // One factory invocation, same cache/resolver/verifier on every
                // attempt. Receiving WS traffic processes post-handshake tickets.
                let ws = TlsWebSocket::connect(&url, node.clone()).await.unwrap();
                let info = ws.peer_info().clone();
                assert_eq!(info.handshake_kind(), if round == 0 { super::ScHandshakeKind::Full } else { super::ScHandshakeKind::Resumed });
                assert_eq!(info.peer_address(), listener.local_addr().unwrap());
                assert_eq!(info.server_name(), "localhost");
                assert_eq!(info.protocol_version(), Some("TLSv1_3"));
                assert!(info.cipher_suite().unwrap().starts_with("TLS13_"));
                if let Some(previous) = previous {
                    assert_ne!(previous.connection_id(), info.connection_id());
                    assert_eq!(previous.certificates(), info.certificates());
                    assert_eq!(previous.leaf_sha256(), info.leaf_sha256());
                }
                previous = Some(info);
                assert_eq!(ws.recv().await.unwrap(), b"ticket barrier");
                ws.send(b"node reply").await.unwrap();
                assert_eq!(ws.recv().await.unwrap(), b"done");
            }
        };
        let (kinds, ()) = tokio::join!(accept, connect);
        kinds
    }).await.expect("TLS/WS observation timed out")
}

#[tokio::test]
async fn node_tls_local_contract_allows_trusted_server_without_certificate_request() {
    assert_eq!(observe_connections(1).await, vec![HandshakeKind::Full]);
}

#[tokio::test]
async fn node_tls_clones_preserve_normal_tls13_resumption() {
    assert_eq!(
        observe_connections(2).await,
        vec![HandshakeKind::Full, HandshakeKind::Resumed]
    );
}
