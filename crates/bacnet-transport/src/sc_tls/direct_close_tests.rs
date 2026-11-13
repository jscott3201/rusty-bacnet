//! The close frames the direct acceptor sends (#2520), read by a real TLS
//! initiator: 1003 for a data frame that is not binary, before or after the
//! Connect exchange (Annex AB.7.5.3, as the hub does), 1000 after answering
//! the initiator's Disconnect-Request (AB.6.2.3), and the goodbye each
//! established peer gets when the listener stops.
use super::*;
use crate::sc::ScDisconnectCause;
use futures_util::SinkExt;
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
