//! What a B/IP transport's frame tap sees on the receive side (#957): a
//! broadcast listener's datagram only when the node keeps it, by the same
//! verdict as the node's own listener check; and a transport's receiver
//! ends once the transport is dropped.

use super::*;
use crate::tap::{FrameTap, TapDirection, TapPeer};
use crate::udp_metadata::ReceivedDatagram;
use std::net::{IpAddr, SocketAddr};

const LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);
const SENDER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 30), 0xBAC0);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

/// A receive context for a node at `own` on a socket reporting to `tap`.
async fn context(
    own: Ipv4Addr,
    broadcast: Ipv4Addr,
    tap: &FrameTap,
) -> (RecvContext, mpsc::Receiver<ReceivedNpdu>) {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let socket = Arc::new(BipSocket::new(socket, None).tapped(tap.clone()));
    let (npdu_tx, npdu_rx) = mpsc::channel(16);
    let ctx = RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(own, 0xBAC0),
        SocketAddrV4::new(broadcast, 0xBAC0),
        None,
    );
    (ctx, npdu_rx)
}

fn addresses(own: Ipv4Addr, listener_interface: Option<u32>) -> IngressAddresses {
    IngressAddresses {
        unicast_ips: vec![own],
        wildcard_bind: false,
        listener_interface,
        interface_mismatch_seen: Default::default(),
    }
}

/// One listener datagram: the node's address, its broadcast address, the
/// datagram, its destination, its arrival interface, and whether it is kept.
type Case<'a> = (Ipv4Addr, Ipv4Addr, &'a [u8], Ipv4Addr, Option<u32>, bool);

/// Datagrams a broadcast listener reads: kept ones reach the tap, refused
/// ones don't, and each verdict is the node's (its listener-mismatch
/// count). The primary's datagrams are left to the read, not tapped here.
#[tokio::test]
async fn a_broadcast_listener_taps_only_what_the_node_keeps() {
    let broadcast = raw_frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    let unicast = raw_frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    let result = raw_frame(BvlcFunction::BVLC_RESULT, &[0x00, 0x00]);
    let garbage = [0x81, 0x0B, 0x00];
    let cases: [Case<'_>; 7] = [
        (LOCAL, BROADCAST, &broadcast, BROADCAST, Some(2), true),
        (
            LOCAL,
            BROADCAST,
            &broadcast,
            Ipv4Addr::BROADCAST,
            None,
            true,
        ),
        // Unicast to this node, or to another local address, on the
        // wildcard listener.
        (LOCAL, BROADCAST, &unicast, LOCAL, Some(2), false),
        (
            LOCAL,
            BROADCAST,
            &unicast,
            Ipv4Addr::new(192, 0, 2, 99),
            None,
            false,
        ),
        // Another interface's broadcast.
        (LOCAL, BROADCAST, &broadcast, BROADCAST, Some(3), false),
        // A broadcast address that is the node's own: a point-to-point
        // function passes the address checks and is still refused.
        (LOCAL, LOCAL, &result, LOCAL, None, false),
        // A broadcast that doesn't decode is the node's to count.
        (LOCAL, BROADCAST, &garbage, BROADCAST, None, true),
    ];
    for (index, (own, broadcast_ip, data, destination, arrival_index, kept)) in
        cases.into_iter().enumerate()
    {
        let tap = FrameTap::new();
        let mut frames = tap.subscribe();
        let (ctx, _npdus) = context(own, broadcast_ip, &tap).await;
        let local = addresses(own, Some(2));
        let received = ReceivedDatagram {
            len: data.len(),
            peer: SocketAddr::V4(SENDER),
            destination: IpAddr::V4(destination),
            arrival_index,
            os_group_delivery: None,
            arrival_stamp: None,
        };
        handle_datagram(data, &received, Arrival::BroadcastListener, &local, &ctx).await;
        let refused = ctx.drops.snapshot().listener_mismatch == 1;
        assert_eq!(kept, !refused, "case {index}: the node's verdict");
        // Only the datagram itself, not anything the node sent for it.
        let tapped: Vec<_> = std::iter::from_fn(|| frames.try_recv())
            .filter(|frame| frame.direction == TapDirection::Inbound)
            .collect();
        if kept {
            assert_eq!(tapped.len(), 1, "case {index}");
            assert_eq!(&tapped[0].bytes[..], data);
            assert_eq!(tapped[0].peer, TapPeer::Udp(SocketAddr::V4(SENDER)));
        } else {
            assert!(tapped.is_empty(), "case {index}: {tapped:?}");
        }

        // The primary socket's datagrams are tapped as they are read.
        handle_datagram(data, &received, Arrival::Primary, &local, &ctx).await;
        let primary = std::iter::from_fn(|| frames.try_recv())
            .filter(|frame| frame.direction == TapDirection::Inbound)
            .count();
        assert_eq!(primary, 0, "case {index}");
    }
}

/// Stopped, a transport's receiver waits on; dropped, it ends.
#[tokio::test]
async fn a_dropped_transport_ends_its_tap() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut frames = transport.frame_tap().subscribe();
    let _npdus = transport.start().await.unwrap();
    transport.stop().await.unwrap();
    let wait = tokio::time::timeout(std::time::Duration::from_millis(50), frames.recv()).await;
    assert!(wait.is_err(), "a stopped transport may start again");
    drop(transport);
    let ended = tokio::time::timeout(std::time::Duration::from_secs(30), frames.recv())
        .await
        .expect("the queue closes with the transport");
    assert!(ended.is_none());
}
