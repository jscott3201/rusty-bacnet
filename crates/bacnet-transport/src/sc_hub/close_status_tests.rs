//! The close frame the hub sends as it ends a connection (#2463): status
//! 1000 when it ends one on purpose (a same-UUID replacement retires the
//! old connection, or the peer's Disconnect-Request has been answered), and
//! the status-less frame it always sent when the peer's fault ends it. The
//! graceful shutdown and Connect-wait closes are pinned in their own tests.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::*;
use tokio::io::AsyncWriteExt;

/// Register `peer` as `vmac` with Device UUID `uuid`.
async fn register(peer: &mut ClientWs, vmac: Vmac, uuid: DeviceUuid) {
    peer.send(request(vmac, uuid)).await.unwrap();
    assert!(matches!(poll_io(peer.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
}

/// The next frame `peer` reads.
async fn next(peer: &mut ClientWs) -> Message {
    poll_io(peer.next())
        .await
        .expect("peer stream ended")
        .expect("peer transport error")
}

#[tokio::test]
async fn a_replaced_registration_is_closed_with_status_1000() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut old = tls.websocket(hub.address).await;
    register(&mut old, [0x42; 6], [0x42; 16]).await;
    // The same device connects again under another VMAC: the hub retires
    // the old connection, an orderly end.
    let mut new = tls.websocket(hub.address).await;
    register(&mut new, [0x43; 6], [0x42; 16]).await;
    assert_eq!(next(&mut old).await, closed_normally());
    old.flush().await.unwrap();
    hub.hub.stop().await;
}

#[tokio::test]
async fn a_peers_disconnect_request_is_acked_then_closed_with_status_1000() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    register(&mut peer, [0x42; 6], [0x42; 16]).await;
    let request = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    peer.send(Message::Binary(buf.freeze())).await.unwrap();
    let Message::Binary(ack) = next(&mut peer).await else {
        panic!("expected the Disconnect-ACK first");
    };
    assert_eq!(
        decode_sc_message(&ack).unwrap().function,
        ScFunction::DisconnectAck
    );
    assert_eq!(next(&mut peer).await, closed_normally());
    peer.flush().await.unwrap();
    hub.hub.stop().await;
}

/// What a registered peer does wrong.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// Declares a frame past the hub's size limit: the hub's read fails.
    Oversize,
    /// Sends a second Connect-Request on a connected link.
    SecondConnect,
}

/// The peer's fault ends the connection: not an orderly end, so the close
/// frame keeps no status, as before #2463.
#[tokio::test]
async fn a_connection_the_peers_fault_ends_keeps_its_status_less_close() {
    for fault in [Fault::Oversize, Fault::SecondConnect] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut peer = tls.websocket(hub.address).await;
        register(&mut peer, [0x42; 6], [0x42; 16]).await;
        match fault {
            Fault::Oversize => {
                // A masked binary header declaring 5706 bytes; no body.
                let header = [0x82, 0xfe, 0x16, 0x4a, 1, 2, 3, 4];
                peer.get_mut().write_all(&header).await.unwrap();
                peer.get_mut().flush().await.unwrap();
            }
            Fault::SecondConnect => peer.send(request([0x42; 6], [0x42; 16])).await.unwrap(),
        }
        assert_eq!(next(&mut peer).await, Message::Close(None), "{fault:?}");
        hub.hub.stop().await;
    }
}
