//! Real TLS/SC peers observe identity, heartbeat, retry and cancellation.
use super::*;
use bacnet_transport::{sc_frame::BACNET_SC_HUB_SUBPROTOCOL, sc_tls::TlsWebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
    task::JoinHandle,
    time::Instant,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
    },
};

type Socket = WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>;

const HEARTBEAT_INTERVAL_MS: u64 = 3_000;
// The existing receive loop records its idle origin after starting the timer.
// Its first tick can precede one full idle period, so observe two periods plus
// scheduling margin. This is a fixture deadline, not a transport timing promise.
const FIRST_HEARTBEAT_BUDGET: Duration = Duration::from_millis(2 * HEARTBEAT_INTERVAL_MS + 2_000);

async fn observe<T>(phase: &str, budget: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(budget, future)
        .await
        .unwrap_or_else(|_| panic!("{phase} exceeded {budget:?}"))
}

struct Peer<T>(JoinHandle<T>);
impl<T> Drop for Peer<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[allow(clippy::result_large_err)]
fn subprotocol(_: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
    );
    Ok(response)
}

async fn websocket(listener: &TcpListener, acceptor: &tokio_rustls::TlsAcceptor) -> Socket {
    let (tcp, _) = bounded(listener.accept()).await.unwrap();
    let tls = bounded(acceptor.accept(tcp)).await.unwrap();
    bounded(tokio_tungstenite::accept_hdr_async(tls, subprotocol))
        .await
        .unwrap()
}

async fn accept_sc(ws: &mut Socket) {
    let Message::Binary(request) = bounded(ws.next()).await.unwrap().unwrap() else {
        panic!("expected Connect-Request");
    };
    // Independent offsets for the configured node identity, without product decoding.
    assert_eq!(request.len(), 30);
    assert_eq!(&request[..4], &[6, 0, 0, 1]);
    assert_eq!(&request[4..10], &[2; 6]);
    assert_eq!(&request[10..26], &[3; 16]);
    let mut accept = vec![7, 0, 0, 1];
    accept.extend_from_slice(&[9; 6]);
    accept.extend_from_slice(&[9; 16]);
    accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
    bounded(ws.send(Message::Binary(accept.into())))
        .await
        .unwrap();
}

fn builder(url: &str, certs: &CertMaterial) -> ScClientBuilder {
    BACnetClient::sc_builder()
        .hub_url(url)
        .tls_config(try_make_node_tls_config(certs).unwrap())
        .vmac([2; 6])
        .device_uuid([3; 16])
}

async fn eof(mut tcp: TcpStream) {
    let mut data = [0; 4096];
    bounded(async {
        loop {
            match tcp.read(&mut data).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    break;
                }
                Err(e) => panic!("unexpected TCP read error: {e}"),
            }
        }
    })
    .await;
}

#[tokio::test]
async fn erased_sc_preserves_heartbeat_retry_spacing_and_stop_cancels_pending_tls() {
    let certs = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = tokio_rustls::TlsAcceptor::from(make_server_tls_config_mtls(&certs));
    let (events, mut received) = mpsc::channel(4);
    let mut peer = Peer(tokio::spawn(async move {
        let mut ws = websocket(&listener, &acceptor).await;
        accept_sc(&mut ws).await;
        // A configured heartbeat arrives; withholding its ACK exercises the
        // configured timeout and causes the client's reconnect connector to dial.
        loop {
            let message = observe("first heartbeat frame", FIRST_HEARTBEAT_BUDGET, ws.next())
                .await
                .unwrap()
                .unwrap();
            if let Message::Binary(data) = message
                && data[0] == 10
            {
                events.send(0).await.unwrap();
                break;
            }
        }
        let (mut retry, _) = bounded(listener.accept()).await.unwrap();
        let first = Instant::now();
        retry.shutdown().await.unwrap();
        drop(retry);
        events.send(1).await.unwrap();
        let (pending, _) = bounded(listener.accept()).await.unwrap();
        assert!(
            first.elapsed() >= Duration::from_millis(120),
            "retry attempts must be paced"
        );
        events.send(2).await.unwrap();
        // This third TCP connection never completes TLS. stop() must cancel it.
        eof(pending).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(250), listener.accept())
                .await
                .is_err(),
            "stop must prevent further retries"
        );
        drop(ws);
    }));
    let mut client: BACnetClient<AnyTransport> = observe(
        "initial SC connection",
        Duration::from_secs(5),
        Box::pin(
            builder(&url, &certs)
                .heartbeat_interval_ms(HEARTBEAT_INTERVAL_MS)
                .heartbeat_timeout_ms(3_200)
                .reconnect(ScReconnectConfig {
                    initial_delay_ms: 120,
                    max_delay_ms: 120,
                    max_retries: 10,
                })
                .build_any(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        observe(
            "first heartbeat observation",
            FIRST_HEARTBEAT_BUDGET,
            received.recv()
        )
        .await,
        Some(0)
    );
    for expected in 1..3 {
        assert_eq!(
            observe(
                "retry TCP observation",
                Duration::from_secs(5),
                received.recv()
            )
            .await,
            Some(expected)
        );
    }
    observe(
        "stop during pending TLS",
        Duration::from_secs(5),
        Box::pin(client.stop()),
    )
    .await
    .unwrap();
    bounded(&mut peer.0).await.unwrap();
}

#[tokio::test]
async fn erased_sc_without_reconnect_does_not_redial() {
    let certs = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = tokio_rustls::TlsAcceptor::from(make_server_tls_config_mtls(&certs));
    let (ready, wait) = tokio::sync::oneshot::channel();
    let mut peer = Peer(tokio::spawn(async move {
        let mut ws = websocket(&listener, &acceptor).await;
        accept_sc(&mut ws).await;
        wait.await.unwrap();
        drop(ws);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err()
        );
    }));
    let mut client = Box::pin(builder(&url, &certs).build_any()).await.unwrap();
    let AnyTransport::Sc(transport) = client.transport() else {
        panic!("expected SC");
    };
    let mut state = transport.connection_state_changes();
    ready.send(()).unwrap();
    bounded(async {
        while *state.borrow_and_update() == bacnet_transport::sc::ScConnectionState::Connected {
            state.changed().await.unwrap();
        }
    })
    .await;
    bounded(&mut peer.0).await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn cancel_erased_sc_build_during_tls_releases_its_socket() {
    let certs = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let pending = Box::pin(builder(&url, &certs).build_any());
    let tcp = bounded(async {
        tokio::pin!(pending);
        tokio::select! {
            result = &mut pending => panic!("TLS peer has not handshaken: {:?}", result.err()),
            incoming = listener.accept() => incoming.unwrap().0,
        }
    })
    .await;
    eof(tcp).await;
}

#[tokio::test]
async fn cancel_erased_sc_build_waiting_for_connect_accept_releases_tls() {
    let certs = generate_test_certs();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let acceptor = tokio_rustls::TlsAcceptor::from(make_server_tls_config_mtls(&certs));
    let (ready, request_received) = tokio::sync::oneshot::channel();
    let mut peer = Peer(tokio::spawn(async move {
        let mut ws = websocket(&listener, &acceptor).await;
        let Message::Binary(request) = bounded(ws.next()).await.unwrap().unwrap() else {
            panic!("expected SC connect");
        };
        assert_eq!(request[0], 6);
        ready.send(()).unwrap();
        let closed = bounded(ws.next()).await;
        assert!(closed.is_none() || matches!(closed, Some(Err(_)) | Some(Ok(Message::Close(_)))));
    }));
    {
        let pending = Box::pin(builder(&url, &certs).build_any());
        tokio::pin!(pending);
        tokio::select! {
            result = &mut pending => panic!("SC peer has not accepted: {:?}", result.err()),
            result = request_received => result.unwrap(),
        }
    }
    bounded(&mut peer.0).await.unwrap();
}

#[test]
fn concrete_and_erased_build_futures_stay_small_and_send() {
    fn check(future: impl Future + Send) {
        assert!(
            std::mem::size_of_val(&future) < 2048,
            "build future retains a large startup state: {} bytes",
            std::mem::size_of_val(&future)
        );
    }
    check(BACnetClient::bip_builder().build());
    check(BACnetClient::bip_builder().build_any());
    check(BACnetClient::bip6_builder().build());
    check(BACnetClient::bip6_builder().build_any());
    check(BACnetClient::<bacnet_transport::sc::ScTransport<TlsWebSocket>>::sc_builder().build());
    check(BACnetClient::sc_builder().build_any());
}
