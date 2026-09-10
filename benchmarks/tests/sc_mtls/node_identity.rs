//! Independent Connect-Request byte oracle for the real Rust SC server builder.
use bacnet_benchmarks::sc_helpers::*;
use bacnet_server::server::BACnetServer;
use bacnet_transport::sc::ScReconnectConfig;
use futures_util::{FutureExt, SinkExt, StreamExt};
use std::{future::Future, panic::AssertUnwindSafe, time::Duration};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::tungstenite::{handshake::server::Response, Message};

const UUID: [u8; 16] = [
    0x8e, 0x62, 0xac, 0x46, 0xd7, 0x08, 0x42, 0x26, 0x91, 0x37, 0x76, 0xa3, 0x2b, 0x61, 0x93, 0x15,
];
const VMAC: [u8; 6] = [2, 0, 0, 0, 0, 1];

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("SC identity barrier timed out")
}

#[tokio::test]
async fn sc_server_uuid_wire_bytes_survive_reconnect_and_fresh_builds() {
    let certs = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = tokio_rustls::TlsAcceptor::from(make_server_tls_config_mtls(&certs));
    let (events, mut connected) = mpsc::channel(4);
    let (cut, mut cuts) = mpsc::channel(1);
    let peer = tokio::spawn(async move {
        for round in 0..4 {
            let (tcp, _) = bounded(listener.accept()).await.unwrap();
            let tls = bounded(acceptor.accept(tcp)).await.unwrap();
            let mut ws = bounded(tokio_tungstenite::accept_hdr_async(
                tls,
                |_: &_, mut response: Response| {
                    response.headers_mut().insert(
                        "Sec-WebSocket-Protocol",
                        "hub.bsc.bacnet.org".parse().unwrap(),
                    );
                    Ok(response)
                },
            ))
            .await
            .unwrap();
            let message = bounded(ws.next()).await.unwrap().unwrap();
            let Message::Binary(request) = message else {
                panic!("expected binary Connect-Request, got {message:?}");
            };
            // Base 2020 AB.2.10: header4 + VMAC6 + UUID16 + limits4.
            // Do not use the product decoder to derive the expected offsets.
            assert_eq!(request.len(), 30);
            assert_eq!(&request[..4], &[6, 0, 0, 1]);
            assert_eq!(&request[4..10], &VMAC);
            assert_eq!(&request[10..26], &UUID);
            assert_eq!(&request[26..], &[0x16, 0x49, 0x05, 0xc4]);
            let mut accept = vec![7, 0, 0, 1];
            accept.extend_from_slice(&[2, 0, 0, 0, 0, 9]);
            accept.extend_from_slice(&[
                0x4a, 0x01, 0x5c, 0xf2, 0xec, 0x39, 0x4d, 0x58, 0xac, 0x2d, 0x11, 0xe1, 0x76, 0x1c,
                0xe8, 0x6d,
            ]);
            accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
            bounded(ws.send(Message::Binary(accept.into())))
                .await
                .unwrap();
            // An ACK proves the redial finished installing its receive loop
            // before the test asks the server to stop.
            bounded(ws.send(Message::Binary(vec![10, 0, 0x12, 0x34].into())))
                .await
                .unwrap();
            loop {
                let message = bounded(ws.next()).await.unwrap().unwrap();
                let Message::Binary(data) = message else {
                    panic!("expected heartbeat ACK, got {message:?}");
                };
                if data[0] == 11 {
                    assert_eq!(&data[..], &[11, 0, 0x12, 0x34]);
                    break;
                }
                assert_eq!(data[0], 1);
            }
            events.send(round).await.unwrap();
            // Even rounds inject socket loss. Odd rounds release the peer only
            // after server stop/drop: server shutdown does not promise a wire ACK.
            bounded(cuts.recv()).await.unwrap();
        }
    });
    let mut server = None;
    let result = AssertUnwindSafe(async {
        for lifecycle in 0..2 {
            server = Some(
                bounded(
                    BACnetServer::sc_builder()
                        .hub_url(&url)
                        .tls_config(try_make_node_tls_config(&certs).unwrap())
                        .vmac(VMAC)
                        .device_uuid(UUID)
                        .reconnect(ScReconnectConfig {
                            initial_delay_ms: 10,
                            max_delay_ms: 10,
                            max_retries: 1,
                        })
                        .build(),
                )
                .await
                .unwrap(),
            );
            assert_eq!(bounded(connected.recv()).await, Some(lifecycle * 2));
            cut.send(()).await.unwrap();
            assert_eq!(bounded(connected.recv()).await, Some(lifecycle * 2 + 1));
            bounded(server.as_mut().unwrap().stop()).await.unwrap();
            server = None;
            cut.send(()).await.unwrap();
        }
    })
    .catch_unwind()
    .await;
    if let Some(server) = &mut server {
        bounded(server.stop()).await.unwrap();
    }
    if result.is_err() {
        peer.abort();
    }
    let peer_result = bounded(peer).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    peer_result.unwrap();
}
