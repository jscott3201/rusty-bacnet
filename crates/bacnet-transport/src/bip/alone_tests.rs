//! A BBMD alone on its subnet skips the local rebroadcast of forwarded and
//! distributed broadcasts (Annex J.4.5, J.7.5, #1668).

use super::*;
use bytes::Bytes;
use tokio::time::{timeout, Duration};

async fn sink() -> (UdpSocket, u16) {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, port)
}

async fn recv_bvll(socket: &UdpSocket) -> BvllMessage {
    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(5), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLL frame")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

async fn assert_no_bvll(socket: &UdpSocket, label: &str) {
    let mut recv_buf = [0u8; 2048];
    assert!(
        timeout(Duration::from_millis(100), socket.recv_from(&mut recv_buf))
            .await
            .is_err(),
        "{label} received an unexpected BVLL frame"
    );
}

/// A BBMD on loopback with a two-hop own row, one two-hop peer at
/// `peer_port`, a foreign device at `fd_port`, and its local broadcast at
/// `broadcast_port`, alone on its subnet or not.
async fn context(
    alone: bool,
    peer_port: u16,
    fd_ports: &[u16],
    broadcast_port: u16,
) -> (RecvContext, mpsc::Receiver<ReceivedNpdu>) {
    let socket = Arc::new(super::BipSocket::new(sink().await.0, None));
    let local_port = socket.local_addr().unwrap().port();
    let (npdu_tx, npdu_rx) = mpsc::channel(4);
    let localhost = Ipv4Addr::LOCALHOST.octets();
    let mut state = BbmdState::new(localhost, local_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state.set_alone_on_subnet(alone);
    state
        .set_bdt(vec![BdtEntry {
            ip: localhost,
            port: peer_port,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
    for &port in fd_ports {
        assert_eq!(
            state.register_foreign_device(localhost, port, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    let ctx = RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, local_port),
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, broadcast_port),
        Some(Arc::new(std::sync::Mutex::new(state))),
    );
    (ctx, npdu_rx)
}

/// A peer's Forwarded-NPDU, sent to this BBMD alone (its own row is
/// two-hop): rebroadcast on the subnet unless the BBMD is alone there, and
/// sent to the foreign device either way.
#[tokio::test]
async fn a_bbmd_alone_on_its_subnet_sends_a_peers_forward_to_foreign_devices_only() {
    for alone in [false, true] {
        let (peer, peer_port) = sink().await;
        let (device, device_port) = sink().await;
        let (local, local_port) = sink().await;
        let (ctx, mut npdu_rx) = context(alone, peer_port, &[device_port], local_port).await;
        let origin = ([192, 0, 2, 20], 0xBAC0);
        let msg = forwarded_message(origin, &[0x01, 0x00, 0xAA, 0xBB]);
        let sender = (Ipv4Addr::LOCALHOST.octets(), peer_port);

        handle_bvll_message(&msg, sender, Delivery::Unicast, &ctx).await;

        let received = timeout(Duration::from_secs(5), npdu_rx.recv())
            .await
            .expect("timed out waiting for the NPDU")
            .expect("NPDU channel closed");
        assert_eq!(received.npdu, msg.npdu().unwrap(), "alone: {alone}");
        let frame = recv_bvll(&device).await;
        assert_eq!(origin_of(&frame), Some(origin), "alone: {alone}");
        if alone {
            assert_no_bvll(&local, "the local broadcast of a lone BBMD").await;
        } else {
            let frame = recv_bvll(&local).await;
            assert_eq!(origin_of(&frame), Some(origin));
        }
        assert_no_bvll(&peer, "the peer").await;
    }
}

/// A foreign device's Distribute-Broadcast-To-Network: to the subnet unless
/// the BBMD is alone there, and to the peer and the other foreign device
/// either way, with no NAK.
#[tokio::test]
async fn a_bbmd_alone_on_its_subnet_distributes_to_peers_and_foreign_devices_only() {
    for alone in [false, true] {
        let (peer, peer_port) = sink().await;
        let (origin, origin_port) = sink().await;
        let (other, other_port) = sink().await;
        let (local, local_port) = sink().await;
        let (ctx, mut npdu_rx) =
            context(alone, peer_port, &[origin_port, other_port], local_port).await;
        let sender = (Ipv4Addr::LOCALHOST.octets(), origin_port);
        let msg = BvllMessage {
            function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
            payload: Bytes::from_static(&[0x01, 0x20, 0xBA, 0xC0, 0xDB, 0x10]),
        };

        handle_bvll_message(&msg, sender, Delivery::Unicast, &ctx).await;

        let received = timeout(Duration::from_secs(5), npdu_rx.recv())
            .await
            .expect("timed out waiting for the NPDU")
            .expect("NPDU channel closed");
        assert_eq!(received.npdu.as_ref(), msg.payload.as_ref());
        for (label, socket) in [("the peer", &peer), ("the other device", &other)] {
            let frame = recv_bvll(socket).await;
            assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
            assert_eq!(origin_of(&frame), Some(sender), "{label}");
        }
        if alone {
            assert_no_bvll(&local, "the local broadcast of a lone BBMD").await;
        } else {
            let frame = recv_bvll(&local).await;
            assert_eq!(origin_of(&frame), Some(sender));
        }
        assert_no_bvll(&origin, "the originating device (no NAK, no echo)").await;
    }
}

/// The transport's setting reaches the BBMD state `start()` builds.
#[tokio::test]
async fn the_setting_reaches_the_started_bbmd() {
    for alone in [false, true] {
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.enable_bbmd(vec![]);
        transport.set_bbmd_alone_on_subnet(alone);
        let _rx = transport.start().await.unwrap();
        let state = transport.bbmd_state().unwrap();
        assert_eq!(bbmd::lock(state).alone_on_subnet(), alone);
        transport.stop().await.unwrap();
    }
}
