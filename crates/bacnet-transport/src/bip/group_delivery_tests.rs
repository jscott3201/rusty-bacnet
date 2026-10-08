//! An Original-Unicast-NPDU that was sent to a broadcast address is never
//! handed up as a directed NPDU (#1301). The receive path drops it, and the
//! handler takes the NPDU's `link_layer_group` from how the datagram arrived.
//! Datagrams are fed in-process with the destination the OS would report.

use super::ingress::arrived_elsewhere;
use super::*;
use crate::udp_metadata::ReceivedDatagram;
use bytes::Bytes;
use std::net::{IpAddr, SocketAddr};

/// A ReadProperty confirmed request in an NPDU that expects a reply.
const CONFIRMED_REQUEST: &[u8] = &[
    0x01, 0x04, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x55,
];
const LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const SUBNET_BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);
const SENDER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 30), 0xBAC0);

/// A receive context whose only listener is the returned NPDU channel.
async fn context(broadcast_addr: Ipv4Addr) -> (RecvContext, mpsc::Receiver<ReceivedNpdu>) {
    let socket = Arc::new(super::BipSocket::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
        None,
    ));
    let (npdu_tx, npdu_rx) = mpsc::channel(4);
    let ctx = RecvContext {
        local_mac: encode_bip_mac(LOCAL.octets(), 0xBAC0),
        socket,
        npdu_tx,
        bbmd: None,
        broadcast_addr,
        broadcast_port: 0xBAC0,
        client_management: Arc::default(),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
        group_sources: super::groups::GroupSources::detached(),
    };
    (ctx, npdu_rx)
}

fn frame(function: BvlcFunction) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, CONFIRMED_REQUEST).unwrap();
    buf
}

/// Feed one datagram to the receive path as though the OS reported it sent
/// to `destination`, and return the NPDU it handed up, if any.
async fn receive(
    ctx: &RecvContext,
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    local: &IngressAddresses,
    function: BvlcFunction,
    destination: Ipv4Addr,
    os_group_delivery: Option<bool>,
) -> Option<ReceivedNpdu> {
    let arrival = Arrival::Primary;
    receive_on(
        ctx,
        rx,
        local,
        arrival,
        function,
        destination,
        os_group_delivery,
    )
    .await
}

/// [`receive`] on the socket `arrival` names.
async fn receive_on(
    ctx: &RecvContext,
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    local: &IngressAddresses,
    arrival: Arrival,
    function: BvlcFunction,
    destination: Ipv4Addr,
    os_group_delivery: Option<bool>,
) -> Option<ReceivedNpdu> {
    let data = frame(function);
    let received = ReceivedDatagram {
        len: data.len(),
        peer: SocketAddr::V4(SENDER),
        destination: IpAddr::V4(destination),
        arrival_index: None,
        os_group_delivery,
    };
    handle_datagram(&data, &received, arrival, local, ctx).await;
    rx.try_recv().ok()
}

fn bound_to(local_ip: Ipv4Addr) -> IngressAddresses {
    IngressAddresses {
        local_ip,
        unicast_ips: vec![local_ip],
        wildcard_bind: false,
        listener_interface: None,
        interface_mismatch_seen: Default::default(),
    }
}

#[tokio::test]
async fn original_unicast_takes_its_group_flag_from_the_delivery() {
    let (ctx, mut rx) = context(SUBNET_BROADCAST).await;
    let msg = BvllMessage {
        function: BvlcFunction::ORIGINAL_UNICAST_NPDU,
        payload: Bytes::from_static(CONFIRMED_REQUEST),
        originating_ip: None,
        originating_port: None,
    };
    let sender = (SENDER.ip().octets(), SENDER.port());
    for (delivery, group) in [(Delivery::Broadcast, true), (Delivery::Unicast, false)] {
        handle_bvll_message(&msg, sender, delivery, &ctx).await;
        let npdu = rx.try_recv().expect("the handler hands the NPDU up");
        assert_eq!(npdu.npdu.as_ref(), CONFIRMED_REQUEST);
        assert_eq!(npdu.link_layer_group, group, "{delivery:?}");
    }
}

#[tokio::test]
async fn original_unicast_sent_to_a_broadcast_address_is_dropped() {
    let (ctx, mut rx) = context(SUBNET_BROADCAST).await;
    let local = bound_to(LOCAL);
    let unicast = BvlcFunction::ORIGINAL_UNICAST_NPDU;

    // The subnet and limited broadcast addresses, and a datagram to this
    // node's own address that Windows flags as a link-layer broadcast.
    for (destination, os) in [
        (SUBNET_BROADCAST, None),
        (Ipv4Addr::BROADCAST, None),
        (LOCAL, Some(true)),
    ] {
        assert!(
            receive(&ctx, &mut rx, &local, unicast, destination, os)
                .await
                .is_none(),
            "{destination} {os:?}"
        );
    }

    let directed = receive(&ctx, &mut rx, &local, unicast, LOCAL, None)
        .await
        .expect("a directed Original-Unicast-NPDU is handed up");
    assert_eq!(directed.npdu.as_ref(), CONFIRMED_REQUEST);
    assert!(!directed.link_layer_group);

    let broadcast = BvlcFunction::ORIGINAL_BROADCAST_NPDU;
    let group = receive(&ctx, &mut rx, &local, broadcast, SUBNET_BROADCAST, None)
        .await
        .expect("an Original-Broadcast-NPDU to the subnet is handed up");
    assert!(group.link_layer_group);
}

/// Loopback tests often configure the broadcast address as the node's own
/// address. A datagram sent there fits either function, so the function
/// decides.
#[tokio::test]
async fn the_function_decides_where_the_broadcast_address_is_the_local_one() {
    let (ctx, mut rx) = context(Ipv4Addr::LOCALHOST).await;
    let local = bound_to(Ipv4Addr::LOCALHOST);
    for (function, group) in [
        (BvlcFunction::ORIGINAL_UNICAST_NPDU, false),
        (BvlcFunction::ORIGINAL_BROADCAST_NPDU, true),
    ] {
        let npdu = receive(&ctx, &mut rx, &local, function, Ipv4Addr::LOCALHOST, None)
            .await
            .expect("both functions are handed up");
        assert_eq!(npdu.link_layer_group, group, "{function:?}");
    }
}

#[test]
fn a_broadcast_address_that_is_a_host_address_is_flagged_unless_loopback() {
    let other = Ipv4Addr::new(192, 0, 2, 11);
    for (broadcast, own, flagged) in [
        (LOCAL, vec![LOCAL], true),
        (other, vec![LOCAL, other], true),
        (SUBNET_BROADCAST, vec![LOCAL], false),
        (Ipv4Addr::BROADCAST, vec![LOCAL], false),
        (Ipv4Addr::LOCALHOST, vec![Ipv4Addr::LOCALHOST], false),
    ] {
        assert_eq!(
            broadcast_is_own_address(broadcast, LOCAL, &own),
            flagged,
            "{broadcast} {own:?}"
        );
    }
}

/// A transport bound to its interface address on Unix also reads a wildcard
/// listener on the port (#1538). Unicast reaches the listener only when no
/// socket is bound to its destination, so the listener keeps broadcasts
/// only, even one whose destination reads as this node's own address.
#[tokio::test]
async fn the_broadcast_listener_keeps_only_broadcasts() {
    let (ctx, mut rx) = context(SUBNET_BROADCAST).await;
    let local = bound_to(LOCAL);
    let unicast = BvlcFunction::ORIGINAL_UNICAST_NPDU;
    let listener = Arrival::BroadcastListener;
    let on_listener = receive_on(&ctx, &mut rx, &local, listener, unicast, LOCAL, None);
    assert!(on_listener.await.is_none());
    assert!(receive(&ctx, &mut rx, &local, unicast, LOCAL, None)
        .await
        .is_some());

    let broadcast = BvlcFunction::ORIGINAL_BROADCAST_NPDU;
    for destination in [SUBNET_BROADCAST, Ipv4Addr::BROADCAST] {
        let npdu = receive_on(
            &ctx,
            &mut rx,
            &local,
            listener,
            broadcast,
            destination,
            None,
        )
        .await
        .expect("the listener hands up a broadcast");
        assert!(npdu.link_layer_group, "{destination}");
    }
}

/// In per-address mode a broadcast listener keeps only what arrived on the
/// transport's own interface (#1538); with either index unknown, it keeps
/// it. The primary socket is not filtered.
#[tokio::test]
async fn a_listener_keeps_only_broadcasts_from_its_own_interface() {
    assert!(arrived_elsewhere(Some(2), Some(3)));
    for (own, arrival) in [
        (Some(2), Some(2)),
        (Some(2), None),
        (Some(2), Some(0)),
        (None, Some(3)),
        (None, None),
    ] {
        assert!(!arrived_elsewhere(own, arrival), "{own:?} {arrival:?}");
    }

    let (ctx, mut rx) = context(SUBNET_BROADCAST).await;
    let mut local = bound_to(LOCAL);
    local.listener_interface = Some(2);
    let data = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    let mut handed_up = Vec::new();
    for (arrival, index) in [
        (Arrival::BroadcastListener, Some(3)),
        (Arrival::BroadcastListener, Some(2)),
        (Arrival::BroadcastListener, None),
        (Arrival::Primary, Some(3)),
    ] {
        let received = ReceivedDatagram {
            len: data.len(),
            peer: SocketAddr::V4(SENDER),
            destination: IpAddr::V4(SUBNET_BROADCAST),
            arrival_index: index,
            os_group_delivery: None,
        };
        handle_datagram(&data, &received, arrival, &local, &ctx).await;
        handed_up.push(rx.try_recv().is_ok());
    }
    assert_eq!(handed_up, [false, true, true, true]);
}
