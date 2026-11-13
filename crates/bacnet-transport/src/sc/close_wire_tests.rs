//! The close frames this node sends when it ends a hub or an outbound
//! direct connection (#2520), read off real TLS sockets by a raw WebSocket
//! peer standing in for the hub or the direct acceptor.
//!
//! Annex AB.7.5.5 asks for the close handshake on an intentional close: an
//! orderly end says 1000, and a peer that stopped answering gets a close
//! with no status, as the hub sends for a peer's fault. A data frame that is
//! not binary gets 1003 (AB.7.5.3). The node writes the frame and drops the
//! socket without waiting for the answering close.
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::Response;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

use super::STOP_SEND_BUDGET;
use crate::port::TransportPort;
use crate::sc::direct_membership::{DirectMembership, DirectRole, Membership};
use crate::sc::direct_pool::PooledDirect;
use crate::sc::direct_socket::DirectSocket;
use crate::sc::{ScDisconnectCause, ScEvent, ScTransport};
use crate::sc_frame::{
    BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL, ScFunction, ScMessage, Vmac,
    decode_sc_message, encode_sc_message,
};
use crate::sc_tls::TlsWebSocket;
use crate::sc_tls::direct_accept::direct_accept_tests::TestCa;

type PeerWs = WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>;

/// Generous: real TLS on a shared runner.
const WAIT: Duration = Duration::from_secs(10);
const NODE_VMAC: Vmac = [0x02, 0, 0, 0, 0, 0x52];
const NODE_UUID: [u8; 16] = [0x52; 16];

/// A loopback listener for a raw peer, and the CA both ends trust.
struct RawPeer {
    listener: TcpListener,
    ca: TestCa,
}

impl RawPeer {
    async fn bind() -> Self {
        Self {
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
            ca: TestCa::generate(),
        }
    }

    fn url(&self) -> String {
        let port = self.listener.local_addr().unwrap().port();
        format!("wss://localhost:{port}")
    }

    /// The node's side of the TLS policy.
    fn node_tls(&self) -> crate::sc_tls::ScNodeTlsConfig {
        self.ca.node_config(vec!["node".into()])
    }

    /// Accept one connection and its upgrade, selecting `subprotocol`.
    async fn accept(self, subprotocol: &'static str) -> PeerWs {
        let (tcp, _) = self.listener.accept().await.unwrap();
        let tls = self.ca.acceptor().accept(tcp).await.unwrap();
        // The handshake callback's signature is tungstenite's.
        #[allow(clippy::result_large_err)]
        let answer = move |_: &_, mut response: Response| {
            let value = subprotocol.parse().unwrap();
            response
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", value);
            Ok(response)
        };
        tokio_tungstenite::accept_hdr_async(tls, answer)
            .await
            .unwrap()
    }
}

/// The next frame `peer` reads.
async fn next(peer: &mut PeerWs) -> Message {
    tokio::time::timeout(WAIT, peer.next())
        .await
        .expect("nothing arrived in time")
        .expect("the stream ended")
        .expect("transport error")
}

/// The next frame, which must be a BVLC message.
async fn next_bvlc(peer: &mut PeerWs) -> ScMessage {
    match next(peer).await {
        Message::Binary(data) => decode_sc_message(&data).unwrap(),
        other => panic!("expected a BVLC message, got {other:?}"),
    }
}

async fn send(peer: &mut PeerWs, message: &ScMessage) {
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, message);
    peer.send(Message::Binary(buf.freeze())).await.unwrap();
}

fn bare(function: ScFunction, message_id: u16, payload: Vec<u8>) -> ScMessage {
    ScMessage {
        function,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    }
}

/// Answer the node's Connect-Request as a hub would.
async fn accept_connect(peer: &mut PeerWs) {
    let request = next_bvlc(peer).await;
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = vec![0x10; 6];
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    send(
        peer,
        &bare(ScFunction::ConnectAccept, request.message_id, payload),
    )
    .await;
}

/// The close frame with `code`, or with no status for `None`, as the peer
/// reads it. Built here, not from the node's code, so the tests check the
/// wire rather than agree with a broken helper.
fn closed(code: Option<CloseCode>) -> Message {
    Message::Close(code.map(|code| CloseFrame {
        code,
        reason: "".into(),
    }))
}

/// Dial `url` as the node does and return the socket.
async fn dial(url: &str, tls: crate::sc_tls::ScNodeTlsConfig) -> TlsWebSocket {
    tokio::time::timeout(WAIT, TlsWebSocket::connect(url, tls))
        .await
        .expect("the dial did not end")
        .expect("the dial failed")
}

/// Start `transport` and return its events, past `Connected`.
async fn connected(
    transport: &mut ScTransport<TlsWebSocket>,
) -> tokio::sync::broadcast::Receiver<ScEvent> {
    let mut events = transport.connection_events();
    let _rx = tokio::time::timeout(WAIT, transport.start())
        .await
        .unwrap()
        .unwrap();
    let event = tokio::time::timeout(WAIT, events.recv()).await.unwrap();
    assert!(matches!(event, Ok(ScEvent::Connected { .. })), "{event:?}");
    events
}

/// `stop` sends a Disconnect-Request, then a close with 1000, and returns
/// at once against a hub that reads nothing until it has: neither the
/// Disconnect-ACK nor the answering close is awaited.
#[tokio::test]
async fn stop_sends_a_disconnect_request_then_a_close_with_1000() {
    let hub = RawPeer::bind().await;
    let (url, tls) = (hub.url(), hub.node_tls());
    let (stopped, until_stopped) = tokio::sync::oneshot::channel();
    let hub = tokio::spawn(async move {
        let mut ws = hub.accept(BACNET_SC_HUB_SUBPROTOCOL).await;
        accept_connect(&mut ws).await;
        // Silent until the node has stopped.
        until_stopped.await.unwrap();
        let request = next_bvlc(&mut ws).await;
        assert_eq!(request.function, ScFunction::DisconnectRequest);
        assert_eq!(next(&mut ws).await, closed(Some(CloseCode::Normal)));
    });
    let mut transport =
        ScTransport::new(dial(&url, tls).await, NODE_VMAC).with_device_uuid(NODE_UUID);
    let _events = connected(&mut transport).await;
    let began = Instant::now();
    transport.stop().await.unwrap();
    assert!(began.elapsed() < STOP_SEND_BUDGET, "{:?}", began.elapsed());
    stopped.send(()).unwrap();
    hub.await.unwrap();
}

/// A hub that leaves a heartbeat unanswered gets a close with no status:
/// the peer's fault (#2520). Without reconnect the socket is given up.
#[tokio::test]
async fn a_heartbeat_timeout_closes_without_a_status() {
    let hub = RawPeer::bind().await;
    let (url, tls) = (hub.url(), hub.node_tls());
    let hub = tokio::spawn(async move {
        let mut ws = hub.accept(BACNET_SC_HUB_SUBPROTOCOL).await;
        accept_connect(&mut ws).await;
        let mut probes = 0;
        loop {
            match next(&mut ws).await {
                Message::Binary(data) => {
                    let message = decode_sc_message(&data).unwrap();
                    assert_eq!(message.function, ScFunction::HeartbeatRequest);
                    probes += 1;
                }
                close => {
                    assert_eq!(close, closed(None));
                    break;
                }
            }
        }
        assert!(probes >= 1);
    });
    let mut transport = ScTransport::new(dial(&url, tls).await, NODE_VMAC)
        .with_device_uuid(NODE_UUID)
        .with_test_heartbeat_timing_ms(100, 300);
    let mut events = connected(&mut transport).await;
    let event = tokio::time::timeout(WAIT, events.recv()).await.unwrap();
    assert!(
        matches!(
            event,
            Ok(ScEvent::Disconnected {
                cause: ScDisconnectCause::HeartbeatTimeout,
                ..
            })
        ),
        "{event:?}"
    );
    hub.await.unwrap();
    transport.stop().await.unwrap();
}

/// A hub that never answers the Connect-Request gets a close with no
/// status when the connect wait runs out and the node gives the socket up.
#[tokio::test]
async fn an_unanswered_connect_request_is_closed_without_a_status() {
    let hub = RawPeer::bind().await;
    let (url, tls) = (hub.url(), hub.node_tls());
    let hub = tokio::spawn(async move {
        let mut ws = hub.accept(BACNET_SC_HUB_SUBPROTOCOL).await;
        let request = next_bvlc(&mut ws).await;
        assert_eq!(request.function, ScFunction::ConnectRequest);
        assert_eq!(next(&mut ws).await, closed(None));
    });
    let mut transport = ScTransport::new(dial(&url, tls).await, NODE_VMAC)
        .with_device_uuid(NODE_UUID)
        .with_connect_timeout_ms(200);
    let error = match tokio::time::timeout(WAIT, transport.start()).await.unwrap() {
        Err(error) => error,
        Ok(_) => panic!("an unanswered node cannot connect"),
    };
    assert!(
        matches!(error, bacnet_types::error::Error::Timeout(_)),
        "{error}"
    );
    hub.await.unwrap();
}

/// Having answered the hub's Disconnect-Request, the node closes with 1000
/// (Annex AB.6.2.2) rather than waiting for the hub to.
#[tokio::test]
async fn answering_the_hubs_disconnect_request_closes_with_1000() {
    let hub = RawPeer::bind().await;
    let (url, tls) = (hub.url(), hub.node_tls());
    let hub = tokio::spawn(async move {
        let mut ws = hub.accept(BACNET_SC_HUB_SUBPROTOCOL).await;
        accept_connect(&mut ws).await;
        send(
            &mut ws,
            &bare(ScFunction::DisconnectRequest, 0x4321, Vec::new()),
        )
        .await;
        let ack = next_bvlc(&mut ws).await;
        assert_eq!(
            (ack.function, ack.message_id),
            (ScFunction::DisconnectAck, 0x4321)
        );
        assert_eq!(next(&mut ws).await, closed(Some(CloseCode::Normal)));
    });
    let mut transport =
        ScTransport::new(dial(&url, tls).await, NODE_VMAC).with_device_uuid(NODE_UUID);
    let mut events = connected(&mut transport).await;
    let event = tokio::time::timeout(WAIT, events.recv()).await.unwrap();
    assert!(
        matches!(
            event,
            Ok(ScEvent::Disconnected {
                cause: ScDisconnectCause::PeerClose,
                ..
            })
        ),
        "{event:?}"
    );
    hub.await.unwrap();
    transport.stop().await.unwrap();
}

/// An outbound direct link over a real TLS socket to a raw acceptor, with
/// its worker running; returns the membership and the acceptor's end.
async fn direct_link() -> (Arc<Membership>, tokio::task::JoinHandle<()>, PeerWs) {
    let acceptor = RawPeer::bind().await;
    let (url, tls) = (acceptor.url(), acceptor.node_tls());
    let peer = tokio::spawn(acceptor.accept(BACNET_SC_DIRECT_SUBPROTOCOL));
    let ws = tokio::time::timeout(WAIT, TlsWebSocket::connect_direct(&url, tls))
        .await
        .unwrap()
        .unwrap();
    let peer = peer.await.unwrap();
    let owner = Arc::new(DirectMembership::default());
    let member = owner
        .reserve([2; 16], [2; 6], [1; 16], [1; 6], DirectRole::Outbound, 16)
        .unwrap()
        .commit();
    let permits = Arc::new(tokio::sync::Semaphore::new(1));
    let (pooled, worker) = PooledDirect::start(
        DirectSocket::<TlsWebSocket>::Verified(Box::new(ws)),
        member,
        permits.try_acquire_owned().unwrap(),
        None,
        (1476, 1476),
        (60_000, 120_000),
    );
    (pooled.member, worker, peer)
}

/// Retiring an outbound direct link, as `stop` does, sends a
/// Disconnect-Request, then a close with 1000.
#[tokio::test]
async fn a_retired_direct_link_sends_a_disconnect_request_then_a_close_with_1000() {
    let (member, worker, mut peer) = direct_link().await;
    member.retire();
    let request = next_bvlc(&mut peer).await;
    assert_eq!(request.function, ScFunction::DisconnectRequest);
    assert_eq!(next(&mut peer).await, closed(Some(CloseCode::Normal)));
    tokio::time::timeout(WAIT, worker).await.unwrap().unwrap();
}

/// A text frame on an outbound direct link is answered with 1003, as the
/// hub and the direct acceptor answer one (Annex AB.7.5.3).
#[tokio::test]
async fn a_text_frame_on_an_outbound_direct_link_is_closed_with_1003() {
    let (_member, worker, mut peer) = direct_link().await;
    peer.send(Message::Text("not BVLC".into())).await.unwrap();
    assert_eq!(next(&mut peer).await, closed(Some(CloseCode::Unsupported)));
    tokio::time::timeout(WAIT, worker).await.unwrap().unwrap();
}

/// A text frame during the outbound direct Connect exchange is answered
/// with 1003 too, and fails the exchange.
#[tokio::test]
async fn a_text_frame_before_the_direct_connect_accept_is_closed_with_1003() {
    let acceptor = RawPeer::bind().await;
    let (url, tls) = (acceptor.url(), acceptor.node_tls());
    let peer = tokio::spawn(async move {
        let mut peer = acceptor.accept(BACNET_SC_DIRECT_SUBPROTOCOL).await;
        let request = next_bvlc(&mut peer).await;
        assert_eq!(request.function, ScFunction::ConnectRequest);
        peer.send(Message::Text("not BVLC".into())).await.unwrap();
        assert_eq!(next(&mut peer).await, closed(Some(CloseCode::Unsupported)));
    });
    let ws = tokio::time::timeout(WAIT, TlsWebSocket::connect_direct(&url, tls))
        .await
        .unwrap()
        .unwrap();
    let socket = DirectSocket::<TlsWebSocket>::Verified(Box::new(ws));
    let conn = Arc::new(tokio::sync::Mutex::new(crate::sc::ScConnection::new(
        NODE_VMAC, NODE_UUID,
    )));
    let error = crate::sc::handshake::perform_handshake(&socket, &conn, None, 5_000)
        .await
        .unwrap_err();
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::WebSocket(crate::sc::ScWebSocketErrorKind::NonBinaryFrame),
        "{error}"
    );
    peer.await.unwrap();
}
