//! `send_unicast` to one of the link's group addresses fails, and sends
//! nothing (#1677): a receiver that checks the destination drops an
//! Original-Unicast-NPDU sent to a group, so the caller finds out instead
//! of losing the message. `send_broadcast` reaches the group.

use super::send_sink::SendSink;
use super::*;

/// A normal node at 192.0.2.10 on 192.0.2.0/24, port 47808, and what it
/// sends.
async fn node() -> (RecvContext, SendSink) {
    let sink = SendSink::default();
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
    let (npdu_tx, _) = mpsc::channel(1);
    let ctx = RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0),
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0),
        None,
    );
    (ctx, sink)
}

/// Each group form, as a B/IP MAC, is refused before anything is encoded or
/// sent; one node's address is not.
///
/// Ported from ACE IoT's bacnet-router-adapters
/// (ACE-IoT-Solutions/rusty-bacnet#5), thanks @acedrew.
#[tokio::test]
async fn unicast_group_destinations_are_rejected_before_encoding_or_send() {
    let (ctx, sink) = node().await;
    let npdu = [0x01, 0x00, 0x10, 0x08];
    for group in [
        [192, 0, 2, 255, 0xBA, 0xC0],
        [192, 0, 2, 255, 0xBA, 0xC1],
        [255, 255, 255, 255, 0xBA, 0xC0],
        [239, 1, 2, 3, 0xBA, 0xC0],
    ] {
        let error = ctx.node.send_unicast(&npdu, &group).await.unwrap_err();
        assert!(matches!(error, Error::Encoding(_)), "{group:?}: {error:?}");
        assert!(error.to_string().contains("group address"), "{error}");
    }
    assert!(sink.take().is_empty(), "nothing went out");
    // Not counted as a send over the budget.
    assert_eq!(ctx.node.send_counters().npdu_too_long, 0);

    let peer = [192, 0, 2, 20, 0xBA, 0xC0];
    ctx.node.send_unicast(&npdu, &peer).await.unwrap();
    ctx.node.send_broadcast(&npdu, None).await.unwrap();
    let sent: Vec<_> = sink.take().into_iter().map(|sent| sent.to).collect();
    assert_eq!(
        sent,
        [
            SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 20), 0xBAC0),
            SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0),
        ]
    );
}

/// The same through a started transport's `TransportPort::send_unicast`, to
/// its own broadcast MAC.
#[tokio::test]
async fn a_started_transport_refuses_a_unicast_to_its_broadcast_mac() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let broadcast = encode_bip_mac(Ipv4Addr::BROADCAST.octets(), transport.port);
    assert!(transport.is_broadcast_mac(&broadcast));
    let error = transport
        .send_unicast(&[0x01, 0x00], &broadcast)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("group address"), "{error}");
    transport.stop().await.unwrap();
}
