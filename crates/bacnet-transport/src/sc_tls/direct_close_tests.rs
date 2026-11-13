//! The close frames the direct acceptor sends (#2520), read by a real TLS
//! initiator: 1003 for a data frame that is not binary, before or after the
//! Connect exchange (Annex AB.7.5.3, as the hub does), 1000 after answering
//! the initiator's Disconnect-Request (AB.6.2.3), and the goodbye each
//! established peer gets when the listener stops.
use super::super::{DirectFrame, DirectWs, DirectWsRead};
use super::*;
use crate::sc::ScDisconnectCause;
use futures_util::SinkExt;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

/// How the initiator reads the acceptor ending the WebSocket.
async fn closed_with(peer: &crate::sc_tls::TlsWebSocket) -> ScDisconnectCause {
    let error = tokio::time::timeout(Duration::from_secs(5), peer.recv())
        .await
        .expect("the acceptor did not close in time")
        .unwrap_err();
    ScDisconnectCause::from_error(&error)
}

/// What an established peer reads when the listener stops (#2520): a
/// Disconnect-Request, then a close with 1000, and nothing it had queued.
pub(super) async fn assert_goodbye(peer: &crate::sc_tls::TlsWebSocket) {
    let next = || tokio::time::timeout(Duration::from_secs(5), peer.recv());
    let request = next()
        .await
        .expect("no Disconnect-Request in time")
        .unwrap();
    assert_eq!(
        decode_sc_message(&request).unwrap().function,
        ScFunction::DisconnectRequest
    );
    let closed = next().await.expect("no close in time").unwrap_err();
    assert_eq!(
        crate::sc::ScDisconnectCause::from_error(&closed),
        crate::sc::ScDisconnectCause::WebSocketClosed { close_status: 1000 },
        "{closed}"
    );
}

async fn send_text(peer: &crate::sc_tls::TlsWebSocket) {
    let text = Message::Text("not BVLC".into());
    peer.write.lock().await.send(text).await.unwrap();
}

#[tokio::test]
async fn a_text_frame_before_the_connect_request_is_closed_with_1003() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let peer = crate::sc_tls::TlsWebSocket::connect_direct(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["peer".into()]),
    )
    .await
    .unwrap();
    send_text(&peer).await;
    assert_eq!(
        closed_with(&peer).await,
        ScDisconnectCause::WebSocketClosed { close_status: 1003 }
    );
    listener.stop().await;
}

#[tokio::test]
async fn a_text_frame_on_an_established_link_is_closed_with_1003() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (peer, _conn) = dial_and_handshake(&url, ca.node_config(vec!["peer".into()])).await;
    send_text(&peer).await;
    assert_eq!(
        closed_with(&peer).await,
        ScDisconnectCause::WebSocketClosed { close_status: 1003 }
    );
    listener.stop().await;
}

#[tokio::test]
async fn answering_a_disconnect_request_closes_with_1000() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (peer, _conn) = dial_and_handshake(&url, ca.node_config(vec!["peer".into()])).await;
    let mut buf = BytesMut::new();
    encode_sc_message(
        &mut buf,
        &crate::sc::direct_membership::disconnect_request(),
    );
    peer.send(&buf).await.unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(5), peer.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&ack).unwrap().function,
        ScFunction::DisconnectAck
    );
    assert_eq!(
        closed_with(&peer).await,
        ScDisconnectCause::WebSocketClosed { close_status: 1000 }
    );
    listener.stop().await;
}

/// Stopping the listener sends each established peer its goodbye, while a
/// peer still in its handshake is dropped and does not hold `stop` up for
/// the close grace.
#[tokio::test]
async fn stopping_the_listener_says_goodbye_only_to_established_peers() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (established, _conn) = dial_and_handshake(&url, ca.node_config(vec!["peer".into()])).await;
    // Upgraded, but no Connect-Request yet.
    let pending =
        crate::sc_tls::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["other".into()]))
            .await
            .unwrap();
    let began = std::time::Instant::now();
    listener.stop().await;
    assert!(
        began.elapsed() < crate::sc::CLOSE_GRACE,
        "{:?}",
        began.elapsed()
    );
    assert_goodbye(&established).await;
    let error = tokio::time::timeout(Duration::from_secs(5), pending.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert_ne!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::WebSocketClosed { close_status: 1000 },
        "a pending peer gets no goodbye: {error}"
    );
}

/// What the acceptor wrote during a handshake, in order.
#[derive(Debug, PartialEq)]
enum Sent {
    /// A BVLC message, by function.
    Data(ScFunction),
    Close(Option<u16>),
}

/// Records each frame the handshake writes.
struct Written(Arc<std::sync::Mutex<Vec<Sent>>>);
impl DirectWs for Written {
    type Read = OneFrame;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()> {
        let function = decode_sc_message(data).unwrap().function;
        self.0.lock().unwrap().push(Sent::Data(function));
        Ok(())
    }
    async fn send_close(&mut self, status: Option<u16>) -> Result<(), ()> {
        self.0.lock().unwrap().push(Sent::Close(status));
        Ok(())
    }
}

/// One frame from the initiator, then nothing more.
struct OneFrame(Option<DirectFrame>);
impl DirectWsRead for OneFrame {
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>> {
        match self.0.take() {
            Some(frame) => Some(Ok(frame)),
            None => std::future::pending().await,
        }
    }
}

/// The acceptor refuses a Connect exchange as the hub does (#2520): a
/// refusal NAK, then 1000; for the initiator's fault (a malformed request,
/// something other than a Connect-Request, a frame too long or one that
/// does not decode) its NAK where one applies, then a close with no status;
/// 1000 when the connect wait runs out; 1003 for a text frame.
#[tokio::test(start_paused = true)]
async fn the_acceptor_closes_each_refused_handshake_by_why() {
    use crate::sc_frame::connect_test_support::valid_connect;
    let ca = TestCa::generate();
    let config = DirectAcceptConfig::new(
        loopback_addr(),
        LISTENER_VMAC,
        LISTENER_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    let heartbeat = crate::sc::direct_membership::bare_control(ScFunction::HeartbeatRequest, 1);
    let cases: [(&str, Option<DirectFrame>, Vec<Sent>); 7] = [
        (
            "reserved VMAC",
            Some(DirectFrame::Binary(valid_connect(6, [0; 6]))),
            vec![Sent::Data(ScFunction::Result), Sent::Close(None)],
        ),
        (
            "the listener's own VMAC",
            Some(DirectFrame::Binary(valid_connect(6, LISTENER_VMAC))),
            vec![Sent::Data(ScFunction::Result), Sent::Close(Some(1000))],
        ),
        (
            "not a Connect-Request",
            Some(DirectFrame::Binary(heartbeat.to_vec())),
            vec![Sent::Close(None)],
        ),
        (
            "too long",
            Some(DirectFrame::Binary(vec![1; 1498])),
            vec![Sent::Close(None)],
        ),
        (
            "undecodable",
            Some(DirectFrame::Binary(vec![0xff])),
            vec![Sent::Close(None)],
        ),
        ("silence", None, vec![Sent::Close(Some(1000))]),
        (
            "text",
            Some(DirectFrame::NonBinary),
            vec![Sent::Close(Some(1003))],
        ),
    ];
    let mut wrong = Vec::new();
    for (name, frame, expected) in cases {
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let membership = Arc::new(crate::sc::direct_membership::DirectMembership::default());
        let outcome = super::super::serve_handshake(
            &mut Written(sent.clone()),
            &mut OneFrame(frame),
            &config,
            loopback_addr(),
            &membership,
            &super::super::DirectWarnings::default(),
        )
        .await;
        assert!(outcome.is_none(), "{name}");
        let sent = std::mem::take(&mut *sent.lock().unwrap());
        if sent != expected {
            wrong.push((name, sent));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// Over real TLS: a peer that sends nothing reads a close with 1000 when
/// the connect wait runs out, and a refused one reads its NAK, then 1000.
#[tokio::test]
async fn a_refused_or_expired_handshake_reads_its_close_over_tls() {
    let ca = TestCa::generate();
    let (mut listener, _rx) =
        start_listener(&ca, |c| c.with_connect_timeout(Duration::from_millis(200))).await;
    let url = direct_url(&listener.local_addr());
    let silent =
        crate::sc_tls::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["a".into()]))
            .await
            .unwrap();
    assert_eq!(
        closed_with(&silent).await,
        ScDisconnectCause::WebSocketClosed { close_status: 1000 }
    );
    let refused =
        crate::sc_tls::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["b".into()]))
            .await
            .unwrap();
    let request = crate::sc_frame::connect_test_support::valid_connect(6, LISTENER_VMAC);
    refused.send(&request).await.unwrap();
    let nak = tokio::time::timeout(Duration::from_secs(5), refused.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&nak).unwrap().function,
        ScFunction::Result
    );
    assert_eq!(
        closed_with(&refused).await,
        ScDisconnectCause::WebSocketClosed { close_status: 1000 }
    );
    listener.stop().await;
}
