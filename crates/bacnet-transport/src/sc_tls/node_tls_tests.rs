use super::{tests::test_tls_pair, *};
use rustls::{HandshakeKind, ProtocolVersion};
use std::time::Duration;
use tokio_rustls::TlsAcceptor;

#[path = "connect_accept_tests.rs"]
mod connect_accept_tests;

#[path = "mu_liveness_tests.rs"]
mod mu_liveness_tests;

async fn observe_connections(rounds: usize) -> Vec<HandshakeKind> {
    // An independent trusted server, deliberately NOT ScHub: no client verifier
    // and no CertificateRequest. This characterizes the local contract's limit.
    let (node, server) = test_tls_pair();
    assert!(server.send_tls13_tickets > 0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = TlsAcceptor::from(server);
    tokio::time::timeout(Duration::from_secs(5), async {
        let accept = async {
            let mut kinds = Vec::new();
            for _ in 0..rounds {
                let (tcp, _) = listener.accept().await.unwrap();
                let tls = acceptor.accept(tcp).await.unwrap();
                assert_eq!(tls.get_ref().1.protocol_version(), Some(ProtocolVersion::TLSv1_3));
                assert!(tls.get_ref().1.peer_certificates().is_none());
                kinds.push(tls.get_ref().1.handshake_kind().unwrap());
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
            for _ in 0..rounds {
                // One factory invocation, same cache/resolver/verifier on every
                // attempt. Receiving WS traffic processes post-handshake tickets.
                let ws = TlsWebSocket::connect(&url, node.clone()).await.unwrap();
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
