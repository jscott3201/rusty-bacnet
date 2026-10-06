//! A datagram whose UDP source is a group address (#1504), and a
//! Forwarded-NPDU whose originating address is one of the link's group
//! destinations (#1493), are malformed. The receive path takes that address
//! as the NPDU's source, so handing one up would let a forged I-Am bind a
//! device to a group, and send the answer to a request to every node in it.
//! These tests feed such frames in-process and over a real socket, with an
//! I-Am inside, and show that nothing is handed up, nothing is forwarded or
//! registered, and each one is counted, while a unicast sender still gets
//! through.

use super::groups::{BipGroups, GroupSources};
use super::*;
use crate::udp_metadata::ReceivedDatagram;
use bytes::Bytes;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::{timeout, Duration};

/// An NPDU carrying I-Am for Device 7.
const I_AM: &[u8] = &[
    0x01, 0x00, 0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x07, 0x22, 0x05, 0xC4, 0x91, 0x00, 0x21, 0x0F,
];
const LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const SUBNET_BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);
const SENDER: ([u8; 4], u16) = ([192, 0, 2, 30], 0xBAC0);
/// A station's own address, a usable origin.
const STATION: ([u8; 4], u16) = ([192, 0, 2, 20], 0xBAC1);

/// Every kind of B/IP group address as an origin: the limited broadcast,
/// the subnet broadcast at this link's port and at another one, and IPv4
/// multicast addresses.
const GROUP_ORIGINS: [([u8; 4], u16); 5] = [
    ([255, 255, 255, 255], 0xBAC0),
    ([192, 0, 2, 255], 0xBAC0),
    ([192, 0, 2, 255], 0xBAC1),
    ([224, 0, 0, 1], 0xBAC0),
    ([239, 255, 255, 250], 0xBAC0),
];

fn forwarded(origin: ([u8; 4], u16)) -> BvllMessage {
    BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: Bytes::from_static(I_AM),
        originating_ip: Some(origin.0),
        originating_port: Some(origin.1),
    }
}

async fn loopback_socket() -> (Arc<super::BipSocket>, u16) {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    (Arc::new(super::BipSocket::new(socket, None)), port)
}

/// The counters a receive context counts its refusals in.
#[derive(Default)]
struct Drops {
    /// UDP sources (#1504).
    sender: Arc<AtomicU64>,
    /// Forwarded-NPDU origins (#1493).
    origin: Arc<AtomicU64>,
}

/// A receive context for a node at 192.0.2.10 on a subnet whose broadcast
/// address is 192.0.2.255, with `bbmd` if given, whose refusals count in
/// `drops`.
async fn context(
    bbmd: Option<BbmdState>,
    broadcast_port: u16,
    drops: &Drops,
) -> (RecvContext, mpsc::Receiver<ReceivedNpdu>) {
    let (socket, _) = loopback_socket().await;
    let (npdu_tx, npdu_rx) = mpsc::channel(8);
    let ctx = RecvContext {
        local_mac: encode_bip_mac(LOCAL.octets(), 0xBAC0),
        socket,
        npdu_tx,
        bbmd: bbmd.map(|state| Arc::new(std::sync::Mutex::new(state))),
        broadcast_addr: SUBNET_BROADCAST,
        broadcast_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        group_sources: GroupSources::new(
            BipGroups::new(SUBNET_BROADCAST, 0xBAC0, LOCAL),
            Arc::clone(&drops.sender),
            Arc::clone(&drops.origin),
        ),
        force_dbtn_forward_failure: false,
    };
    (ctx, npdu_rx)
}

#[tokio::test]
async fn a_forwarded_npdu_from_a_group_origin_is_dropped_and_counted() {
    let drops = Drops::default();
    let (ctx, mut rx) = context(None, 0xBAC0, &drops).await;

    for (count, origin) in (1..).zip(GROUP_ORIGINS) {
        for delivery in [Delivery::Broadcast, Delivery::Unicast] {
            handle_bvll_message(&forwarded(origin), SENDER, delivery, &ctx).await;
            assert!(rx.try_recv().is_err(), "{origin:?} {delivery:?}");
        }
        assert_eq!(
            drops.origin.load(Ordering::Relaxed),
            2 * count,
            "{origin:?}"
        );
    }

    // A station's address is still taken as the source.
    handle_bvll_message(&forwarded(STATION), SENDER, Delivery::Broadcast, &ctx).await;
    let received = rx.try_recv().expect("a unicast origin is handed up");
    assert_eq!(received.npdu.as_ref(), I_AM);
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(STATION.0, STATION.1)
    );
    assert_eq!(
        drops.origin.load(Ordering::Relaxed),
        2 * GROUP_ORIGINS.len() as u64
    );
    assert_eq!(drops.sender.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_bbmd_forwards_a_group_origin_nowhere() {
    let local_broadcast_sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let local_broadcast_port = local_broadcast_sink.local_addr().unwrap().port();
    let foreign_device = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let foreign_port = foreign_device.local_addr().unwrap().port();
    let (_, bbmd_port) = loopback_socket().await;
    let mut state = BbmdState::new(LOCAL.octets(), bbmd_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    // The sender is a BDT peer whose mask asks for a local rebroadcast.
    state
        .set_bdt(vec![BdtEntry {
            ip: SENDER.0,
            port: SENDER.1,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
    assert_eq!(
        state.register_foreign_device(Ipv4Addr::LOCALHOST.octets(), foreign_port, 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let drops = Drops::default();
    let (mut ctx, mut rx) = context(Some(state), local_broadcast_port, &drops).await;
    // Rebroadcasts go to the sink, so the link's broadcast IP is loopback.
    ctx.broadcast_addr = Ipv4Addr::LOCALHOST;

    for origin in GROUP_ORIGINS {
        handle_bvll_message(&forwarded(origin), SENDER, Delivery::Unicast, &ctx).await;
    }
    assert!(rx.try_recv().is_err(), "nothing is handed up");
    assert_eq!(
        drops.origin.load(Ordering::Relaxed),
        GROUP_ORIGINS.len() as u64
    );

    // A station's frame goes to the subnet and the foreign device, and is
    // the first either of them sees.
    handle_bvll_message(&forwarded(STATION), SENDER, Delivery::Unicast, &ctx).await;
    assert_eq!(rx.try_recv().unwrap().npdu.as_ref(), I_AM);
    let mut buf = [0u8; 2048];
    for (sink, label) in [
        (&local_broadcast_sink, "subnet"),
        (&foreign_device, "foreign device"),
    ] {
        let (len, _) = timeout(Duration::from_secs(2), sink.recv_from(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("{label} got nothing"))
            .unwrap();
        let frame = decode_bvll(&buf[..len]).unwrap();
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, Some(STATION.0), "{label}");
    }
}

#[tokio::test]
async fn a_running_transport_counts_group_origins() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut rx = transport.start().await.unwrap();
    let (_, port) = decode_bip_mac(transport.local_mac()).unwrap();
    let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    for (ip, origin_port) in [([255, 255, 255, 255], port), ([224, 0, 0, 1], 0xBAC0)] {
        let mut frame = BytesMut::new();
        crate::bvll::encode_bvll_forwarded(&mut frame, ip, origin_port, I_AM).unwrap();
        peer.send_to(&frame, to).await.unwrap();
    }
    let station = [127, 0, 0, 1];
    let mut frame = BytesMut::new();
    crate::bvll::encode_bvll_forwarded(&mut frame, station, 0xBAC1, I_AM).unwrap();
    peer.send_to(&frame, to).await.unwrap();

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("the station's frame arrives")
        .unwrap();
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip_mac(station, 0xBAC1),
        "the group origins went nowhere"
    );
    assert_eq!(transport.forwarded_group_origin_drops(), 2);
    assert_eq!(transport.group_source_drops(), 0);
    transport.stop().await.unwrap();
}

/// Every kind of B/IP group address as a UDP source (#1504): the limited
/// broadcast, the subnet broadcast at this link's port and another, and
/// IPv4 multicast addresses.
const GROUP_SENDERS: [([u8; 4], u16); 5] = [
    ([255, 255, 255, 255], 0xBAC0),
    ([192, 0, 2, 255], 0xBAC0),
    ([192, 0, 2, 255], 0xBAC1),
    ([224, 0, 0, 1], 0xBAC0),
    ([239, 255, 255, 250], 0xBAC0),
];

/// Feed `frame` to the receive path of a transport bound to 192.0.2.10, from
/// UDP source `sender`, as though the OS reported it sent to `destination`.
async fn receive_from(
    ctx: &RecvContext,
    sender: ([u8; 4], u16),
    destination: Ipv4Addr,
    frame: &[u8],
) {
    let received = ReceivedDatagram {
        len: frame.len(),
        peer: std::net::SocketAddr::from((sender.0, sender.1)),
        destination: destination.into(),
        arrival_index: None,
        os_group_delivery: None,
    };
    let local = IngressAddresses {
        local_ip: LOCAL,
        unicast_ips: vec![LOCAL],
        wildcard_bind: false,
        listener_interface: None,
        interface_mismatch_seen: Default::default(),
    };
    handle_datagram(frame, &received, Arrival::Primary, &local, ctx).await;
}

fn frame(function: BvlcFunction, payload: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, payload).unwrap();
    buf
}

#[tokio::test]
async fn a_datagram_from_a_group_source_is_dropped_before_its_bvlc_function() {
    let drops = Drops::default();
    let (ctx, mut rx) = context(None, 0xBAC0, &drops).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, I_AM);
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, I_AM);
    let mut forwarded = BytesMut::new();
    crate::bvll::encode_bvll_forwarded(&mut forwarded, STATION.0, STATION.1, I_AM).unwrap();
    // Each would otherwise hand an I-Am up from the group source, or from
    // the station that the group source claims to forward for.
    let datagrams = [
        (&unicast, LOCAL),
        (&broadcast, SUBNET_BROADCAST),
        (&forwarded, LOCAL),
    ];

    for (count, sender) in (1..).zip(GROUP_SENDERS) {
        for (frame, destination) in datagrams {
            receive_from(&ctx, sender, destination, frame).await;
            assert!(rx.try_recv().is_err(), "{sender:?} to {destination}");
        }
        let expected = datagrams.len() as u64 * count;
        assert_eq!(drops.sender.load(Ordering::Relaxed), expected, "{sender:?}");
    }
    assert_eq!(drops.origin.load(Ordering::Relaxed), 0);

    // A station's datagrams still go through.
    for (frame, destination) in datagrams {
        receive_from(&ctx, SENDER, destination, frame).await;
        let received = rx.try_recv().expect("a station's datagram is handed up");
        assert_eq!(received.npdu.as_ref(), I_AM);
    }
    let all = (GROUP_SENDERS.len() * datagrams.len()) as u64;
    assert_eq!(drops.sender.load(Ordering::Relaxed), all);
}

#[tokio::test]
async fn a_bbmd_registers_and_forwards_nothing_from_a_group_source() {
    let bdt_peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let foreign_device = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port_of = |socket: &UdpSocket| socket.local_addr().unwrap().port();
    let mut state = BbmdState::new(LOCAL.octets(), 0xBAC0);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
        .set_bdt(vec![BdtEntry {
            ip: Ipv4Addr::LOCALHOST.octets(),
            port: port_of(&bdt_peer),
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
    let localhost = Ipv4Addr::LOCALHOST.octets();
    assert_eq!(
        state.register_foreign_device(localhost, port_of(&foreign_device), 60),
        BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let drops = Drops::default();
    let (ctx, mut rx) = context(Some(state), 0xBAC0, &drops).await;
    let register = frame(BvlcFunction::REGISTER_FOREIGN_DEVICE, &60u16.to_be_bytes());
    let distribute = frame(BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, I_AM);
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, I_AM);

    for sender in GROUP_SENDERS {
        receive_from(&ctx, sender, LOCAL, &register).await;
        receive_from(&ctx, sender, LOCAL, &distribute).await;
        receive_from(&ctx, sender, SUBNET_BROADCAST, &broadcast).await;
    }
    assert!(rx.try_recv().is_err(), "nothing is handed up");
    let refused = 3 * GROUP_SENDERS.len() as u64;
    assert_eq!(drops.sender.load(Ordering::Relaxed), refused);
    let bbmd = ctx.bbmd.as_ref().unwrap();
    assert_eq!(bbmd.lock().unwrap().fdt().len(), 1, "no group registered");

    // A station's broadcast is handed up, and is the first frame the BDT
    // peer and the foreign device see.
    receive_from(&ctx, SENDER, SUBNET_BROADCAST, &broadcast).await;
    assert_eq!(rx.try_recv().unwrap().npdu.as_ref(), I_AM);
    let mut buf = [0u8; 2048];
    for (sink, label) in [(&bdt_peer, "BDT peer"), (&foreign_device, "foreign device")] {
        let received = timeout(Duration::from_secs(2), sink.recv_from(&mut buf)).await;
        let (len, _) = received.expect(label).unwrap();
        let frame = decode_bvll(&buf[..len]).unwrap();
        assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
        assert_eq!(frame.originating_ip, Some(SENDER.0), "{label}");
    }
}

/// A broadcast address that is one of this node's own addresses, as a
/// loopback test or a misconfigured /32 sets it up, names this node when it
/// sends, at any port; configured anywhere else it is a group at every port.
#[test]
fn a_broadcast_address_that_is_this_nodes_own_is_no_group_source() {
    let sources = |broadcast, interface| {
        let groups = BipGroups::new(broadcast, 0xBAC0, interface);
        GroupSources::new(groups, Arc::default(), Arc::default())
    };
    let own = sources(LOCAL, LOCAL);
    let loopback = sources(Ipv4Addr::LOCALHOST, LOCAL);
    let elsewhere = sources(LOCAL, Ipv4Addr::new(192, 0, 2, 11));
    for port in [0xBAC0, 0xBAC1] {
        assert!(own.admits_sender(SocketAddrV4::new(LOCAL, port)), "{port}");
        let from_loopback = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        assert!(loopback.admits_sender(from_loopback), "{port}");
        assert!(
            !elsewhere.admits_sender(SocketAddrV4::new(LOCAL, port)),
            "{port}"
        );
    }
}
