//! Each way the B/IP receive path drops a datagram moves exactly one of the
//! transport's receive counters (#1624). Most datagrams go in through
//! `handle_datagram`, as the receive loop hands them over, for a node at
//! 192.0.2.10 on a subnet whose broadcast address is 192.0.2.255; each test
//! compares the whole snapshot, so a drop counted twice, or under another
//! reason, fails it.

use super::ingress::{AfterReceiveError, receive_failed};
use super::rate_limit::MANAGEMENT_RATE_PER_SOURCE_IP;
use super::receive_counters::ReceiveDrops;
use super::*;
use crate::udp_metadata::ReceivedDatagram;
use crate::warn_throttle::WARNING_INTERVAL;
use std::io;
use tokio::time::{Duration, timeout};

const LOCAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const SUBNET_BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 255);
/// This node's own B/IP address.
const OWN: ([u8; 4], u16) = ([192, 0, 2, 10], 0xBAC0);
/// Another node on the subnet.
const PEER: ([u8; 4], u16) = ([192, 0, 2, 30], 0xBAC0);
/// A node on another subnet, as a Forwarded-NPDU's origin.
const STATION: ([u8; 4], u16) = ([198, 51, 100, 20], 0xBAC0);
/// The interface a broadcast listener keeps broadcasts from.
const LISTENER_INTERFACE: u32 = 2;
/// A Who-Is.
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

/// A receive context and what the receive loop knows of its addresses.
struct Node {
    ctx: RecvContext,
    rx: mpsc::Receiver<ReceivedNpdu>,
    local: IngressAddresses,
}

impl Node {
    /// A node at 192.0.2.10, a BBMD with `bbmd` if given, whose channel to
    /// the network layer holds `capacity` NPDUs.
    async fn new(bbmd: Option<BbmdState>, capacity: usize) -> Self {
        Self::with_broadcast(bbmd, capacity, SUBNET_BROADCAST).await
    }

    /// [`Node::new`] with `broadcast` as its subnet's broadcast IP.
    async fn with_broadcast(bbmd: Option<BbmdState>, capacity: usize, broadcast: Ipv4Addr) -> Self {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let (npdu_tx, rx) = mpsc::channel(capacity);
        let ctx = RecvContext::for_test(
            Arc::new(super::BipSocket::new(socket, None)),
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(OWN.0), OWN.1),
            SocketAddrV4::new(broadcast, OWN.1),
            bbmd.map(|state| Arc::new(std::sync::Mutex::new(state))),
        );
        let local = IngressAddresses {
            unicast_ips: vec![LOCAL],
            wildcard_bind: false,
            listener_interface: Some(LISTENER_INTERFACE),
            interface_mismatch_seen: Default::default(),
        };
        Self { ctx, rx, local }
    }

    /// Feed `datagram` from `from` to `to`, as the primary socket read it.
    async fn feed(&self, datagram: &[u8], from: ([u8; 4], u16), to: Ipv4Addr) {
        self.feed_on(Arrival::Primary, None, datagram, from, to)
            .await;
    }

    /// Feed `datagram` from `from` to `to`, read by `arrival` from the
    /// interface `index`.
    async fn feed_on(
        &self,
        arrival: Arrival,
        index: Option<u32>,
        datagram: &[u8],
        from: ([u8; 4], u16),
        to: Ipv4Addr,
    ) {
        let received = ReceivedDatagram {
            len: datagram.len(),
            peer: std::net::SocketAddr::from(from),
            destination: to.into(),
            arrival_index: index,
            os_group_delivery: None,
            arrival_stamp: None,
        };
        handle_datagram(datagram, &received, arrival, &self.local, &self.ctx).await;
    }

    fn counters(&self) -> ReceiveCounters {
        self.ctx.drops.snapshot()
    }

    fn nothing_handed_up(&mut self) {
        assert!(self.rx.try_recv().is_err(), "an NPDU was handed up");
    }
}

/// Counters where only what `set` writes moved.
fn only(set: impl FnOnce(&mut ReceiveCounters)) -> ReceiveCounters {
    let mut counters = ReceiveCounters::default();
    set(&mut counters);
    counters
}

fn frame(function: BvlcFunction, payload: &[u8]) -> BytesMut {
    raw_frame(function, payload)
}

/// A BBMD at 192.0.2.10 that takes foreign devices, with `bdt`.
fn bbmd(bdt: Vec<BdtEntry>) -> BbmdState {
    let mut state = BbmdState::new(OWN.0, OWN.1);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state.set_bdt(bdt).unwrap();
    state
}

#[tokio::test]
async fn a_well_formed_datagram_counts_nothing() {
    let mut node = Node::new(None, 8).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&unicast, PEER, LOCAL).await;
    assert_eq!(node.rx.try_recv().unwrap().npdu.as_ref(), NPDU);
    assert_eq!(node.counters(), ReceiveCounters::default());
}

#[tokio::test]
async fn a_datagram_shorter_than_a_header_counts_as_too_short() {
    let mut node = Node::new(None, 8).await;
    node.feed(&[0x81, 0x0A, 0x00], PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.too_short = 1));
}

#[tokio::test]
async fn a_datagram_of_another_type_counts_as_not_bip() {
    let mut node = Node::new(None, 8).await;
    // A B/IPv6 Original-Unicast-NPDU's type octet.
    let mut datagram = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    datagram[0] = 0x82;
    node.feed(&datagram, PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.not_bip = 1));
}

#[tokio::test]
async fn a_length_field_that_disagrees_counts_as_length_mismatch() {
    let mut node = Node::new(None, 8).await;
    let mut padded = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU).to_vec();
    padded.push(0x00);
    node.feed(&padded, PEER, LOCAL).await;
    let cut = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&cut[..cut.len() - 1], PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.length_mismatch = 2));
}

#[tokio::test]
async fn a_function_sent_to_the_wrong_address_counts_as_destination_mismatch() {
    let mut node = Node::new(None, 8).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    node.feed(&broadcast, PEER, LOCAL).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&unicast, PEER, SUBNET_BROADCAST).await;
    let read_bdt = frame(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, &[]);
    node.feed(&read_bdt, PEER, SUBNET_BROADCAST).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.destination_mismatch = 3));
}

#[tokio::test]
async fn a_broadcast_listener_counts_what_it_does_not_keep() {
    let mut node = Node::new(None, 8).await;
    let listener = Arrival::BroadcastListener;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed_on(listener, None, &unicast, PEER, LOCAL).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    let elsewhere = Some(LISTENER_INTERFACE + 1);
    node.feed_on(listener, elsewhere, &broadcast, PEER, SUBNET_BROADCAST)
        .await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.listener_mismatch = 2));

    // A broadcast on the listener's own interface is kept.
    let own = Some(LISTENER_INTERFACE);
    node.feed_on(listener, own, &broadcast, PEER, SUBNET_BROADCAST)
        .await;
    assert_eq!(node.rx.try_recv().unwrap().npdu.as_ref(), NPDU);
    assert_eq!(node.counters(), only(|c| c.listener_mismatch = 2));
}

/// What reaches a listener without being this link's broadcast is the
/// listener's to drop, whatever its BVLC function would make of the
/// address: a unicast to another local address that no socket on the port
/// is bound to (which reaches the wildcard listener of macOS and the BSDs),
/// and another subnet's broadcast.
#[tokio::test]
async fn a_broadcast_listener_drops_what_is_not_this_links_broadcast() {
    let mut node = Node::new(None, 8).await;
    let listener = Arrival::BroadcastListener;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    let unbound = Ipv4Addr::new(192, 0, 2, 11);
    node.feed_on(listener, None, &unicast, PEER, unbound).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    let other_subnet = Ipv4Addr::new(10, 0, 0, 255);
    node.feed_on(listener, None, &broadcast, PEER, other_subnet)
        .await;
    // Not even a BVLL header is read.
    node.feed_on(listener, None, &[0x00], PEER, unbound).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.listener_mismatch = 3));

    // Where the broadcast address is the interface's own, as in a loopback
    // test, a unicast sent to it is still not the listener's to keep.
    let mut node = Node::with_broadcast(None, 8, LOCAL).await;
    node.feed_on(listener, None, &unicast, PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.listener_mismatch = 1));
}

#[tokio::test]
async fn an_over_limit_management_request_counts_as_rate_limited() {
    let node = Node::new(None, 8).await;
    node.ctx
        .management_limiter
        .lock()
        .unwrap()
        .freeze_clock(std::time::Instant::now());
    // Within the quota a node that isn't a BBMD NAKs each one.
    let read_bdt = frame(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, &[]);
    for _ in 0..MANAGEMENT_RATE_PER_SOURCE_IP {
        node.feed(&read_bdt, PEER, LOCAL).await;
    }
    assert_eq!(node.counters(), ReceiveCounters::default());
    node.feed(&read_bdt, PEER, LOCAL).await;
    // The limit goes before the payload is read, so a malformed request
    // over it counts as rate limited only.
    let malformed = frame(BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0x3C]);
    node.feed(&malformed, PEER, LOCAL).await;
    assert_eq!(node.counters(), only(|c| c.rate_limited = 2));
}

#[tokio::test]
async fn an_unknown_function_counts_as_unknown_function() {
    let mut node = Node::new(None, 8).await;
    node.feed(&frame(BvlcFunction::from_raw(0x0C), NPDU), PEER, LOCAL)
        .await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.unknown_function = 1));
}

#[tokio::test]
async fn a_payload_that_does_not_fit_its_function_counts_as_malformed() {
    let mut node = Node::new(None, 8).await;
    // Too short for a Forwarded-NPDU's origin: dropped.
    let short = frame(BvlcFunction::FORWARDED_NPDU, &[192, 0, 2, 20, 0xBA]);
    node.feed(&short, PEER, SUBNET_BROADCAST).await;
    // A malformed BBMD request still draws its NAK, and a malformed
    // BVLC-Result no request waits for is dropped.
    let register = frame(BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0x3C]);
    node.feed(&register, PEER, LOCAL).await;
    let result = frame(BvlcFunction::BVLC_RESULT, &[0x00]);
    node.feed(&result, PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.malformed_payload = 3));
}

#[tokio::test]
async fn frames_from_this_node_count_as_own_echo() {
    let mut node = Node::new(None, 8).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&unicast, OWN, LOCAL).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    node.feed(&broadcast, OWN, SUBNET_BROADCAST).await;
    // A Forwarded-NPDU naming this node as its origin.
    node.feed(&forwarded_wire(OWN, NPDU), PEER, SUBNET_BROADCAST)
        .await;
    let distribute = frame(BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, NPDU);
    node.feed(&distribute, OWN, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.own_echo = 4));

    // A BBMD's own local rebroadcast of another node's Forwarded-NPDU.
    let mut bbmd = Node::new(Some(bbmd(Vec::new())), 8).await;
    bbmd.feed(&forwarded_wire(STATION, NPDU), OWN, SUBNET_BROADCAST)
        .await;
    bbmd.nothing_handed_up();
    assert_eq!(bbmd.counters(), only(|c| c.own_echo = 1));
}

#[tokio::test]
async fn a_bbmd_counts_a_forwarded_npdu_from_outside_its_bdt() {
    let mut node = Node::new(Some(bbmd(Vec::new())), 8).await;
    node.feed(&forwarded_wire(STATION, NPDU), PEER, LOCAL).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.forwarded_not_from_peer = 1));
}

#[tokio::test]
async fn a_full_channel_counts_each_npdu_it_drops() {
    let mut node = Node::new(None, 1).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&unicast, PEER, LOCAL).await;
    assert_eq!(node.counters(), ReceiveCounters::default());
    node.feed(&unicast, PEER, LOCAL).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    node.feed(&broadcast, PEER, SUBNET_BROADCAST).await;
    node.feed(&forwarded_wire(STATION, NPDU), PEER, SUBNET_BROADCAST)
        .await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_full = 3));
    assert!(node.rx.try_recv().is_ok(), "the first NPDU is handed up");
    node.nothing_handed_up();

    // A BBMD's Forwarded-NPDU from a BDT peer, and a foreign device's
    // Distribute-Broadcast-To-Network.
    let peer_row = BdtEntry {
        ip: PEER.0,
        port: PEER.1,
        broadcast_mask: [255, 255, 255, 255],
    };
    let mut state = bbmd(vec![peer_row]);
    let foreign = ([192, 0, 2, 40], 0xBAC0);
    let registered = state.register_foreign_device(foreign.0, foreign.1, 60);
    assert_eq!(registered, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let node = Node::new(Some(state), 1).await;
    node.feed(&unicast, PEER, LOCAL).await;
    node.feed(&forwarded_wire(STATION, NPDU), PEER, LOCAL).await;
    let distribute = frame(BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, NPDU);
    node.feed(&distribute, foreign, LOCAL).await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_full = 2));
}

/// Once the network layer has closed its end, every NPDU is dropped as
/// closed, not full, the BBMD's included.
#[tokio::test]
async fn a_closed_channel_counts_each_npdu_it_drops() {
    let mut node = Node::new(None, 8).await;
    node.rx.close();
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    node.feed(&unicast, PEER, LOCAL).await;
    let broadcast = frame(BvlcFunction::ORIGINAL_BROADCAST_NPDU, NPDU);
    node.feed(&broadcast, PEER, SUBNET_BROADCAST).await;
    node.feed(&forwarded_wire(STATION, NPDU), PEER, SUBNET_BROADCAST)
        .await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_closed = 3));

    let peer_row = BdtEntry {
        ip: PEER.0,
        port: PEER.1,
        broadcast_mask: [255, 255, 255, 255],
    };
    let mut state = bbmd(vec![peer_row]);
    let foreign = ([192, 0, 2, 40], 0xBAC0);
    let registered = state.register_foreign_device(foreign.0, foreign.1, 60);
    assert_eq!(registered, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let mut node = Node::new(Some(state), 8).await;
    node.rx.close();
    node.feed(&forwarded_wire(STATION, NPDU), PEER, LOCAL).await;
    let distribute = frame(BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, NPDU);
    node.feed(&distribute, foreign, LOCAL).await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_closed = 2));
}

/// At most one warning per interval, carrying the drops since the last one.
#[tokio::test(start_paused = true)]
async fn the_channel_full_warning_goes_out_at_most_once_per_interval() {
    let drops = ReceiveDrops::default();
    assert_eq!(drops.npdu_channel_full("unicast"), Some(1));
    assert_eq!(drops.npdu_channel_full("unicast"), None);
    assert_eq!(drops.npdu_channel_full("broadcast"), None);
    tokio::time::advance(WARNING_INTERVAL - Duration::from_millis(1)).await;
    assert_eq!(drops.npdu_channel_full("forwarded"), None);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(drops.npdu_channel_full("unicast"), Some(4));
    assert_eq!(drops.npdu_channel_full("unicast"), None);
    assert_eq!(drops.snapshot(), only(|c| c.npdu_channel_full = 6));
}

#[tokio::test]
async fn each_failed_read_counts_as_a_receive_error() {
    let drops = ReceiveDrops::default();
    let invalid = io::Error::new(io::ErrorKind::InvalidData, "truncated");
    let fatal = io::Error::other("socket closed");
    assert_eq!(
        receive_failed(None, &invalid, &drops),
        AfterReceiveError::Continue
    );
    assert_eq!(
        receive_failed(Some(1), &invalid, &drops),
        AfterReceiveError::Continue
    );
    assert_eq!(
        receive_failed(Some(1), &fatal, &drops),
        AfterReceiveError::CloseListener(1)
    );
    assert_eq!(
        receive_failed(None, &fatal, &drops),
        AfterReceiveError::Stop
    );
    assert_eq!(drops.snapshot(), only(|c| c.receive_errors = 4));

    // An IPv4 socket that reported a non-IPv4 peer.
    let mut node = Node::new(None, 8).await;
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    let received = ReceivedDatagram {
        len: unicast.len(),
        peer: "[2001:db8::1]:47808".parse().unwrap(),
        destination: LOCAL.into(),
        arrival_index: None,
        os_group_delivery: None,
        arrival_stamp: None,
    };
    let (local, ctx) = (&node.local, &node.ctx);
    handle_datagram(&unicast, &received, Arrival::Primary, local, ctx).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.receive_errors = 1));
}

/// Through a running transport's sockets: a datagram larger than the
/// receive buffer and one too short for a header are counted, the next is
/// handed up, and the totals survive a restart.
#[tokio::test]
async fn a_running_transport_reports_its_drops() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut rx = transport.start().await.unwrap();
    let (_, port) = decode_bip_mac(transport.local_mac()).unwrap();
    let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    let oversized = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &[0x01; 4096]);
    sender.send_to(&oversized, to).await.unwrap();
    sender.send_to(&[0x81, 0x0A], to).await.unwrap();
    let unicast = frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, NPDU);
    sender.send_to(&unicast, to).await.unwrap();
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("the well-formed datagram arrives")
        .unwrap();
    assert_eq!(received.npdu.as_ref(), NPDU);
    let expected = only(|c| {
        c.receive_errors = 1;
        c.too_short = 1;
    });
    assert_eq!(transport.receive_counters(), expected);

    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(transport.receive_counters(), expected);
    transport.stop().await.unwrap();
}
