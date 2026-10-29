//! Each way the B/IPv6 receive path drops a datagram, or holds back a reply,
//! moves exactly one of the transport's receive counters (#1643). Datagrams
//! go in through `handle_datagram`, as the receive loop hands them over, for
//! a node at 2001:db8::10 whose VMAC is 00 06 50; each test compares the
//! whole snapshot, so a drop counted twice, or under another reason, fails
//! it.

use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::Level;

use super::frame::{encoded, raw_frame};
use super::receive::{Receiver, Reply, receive_failed};
use super::receive_counters::ReceiveDrops;
use super::reply_limit::REPLIES_PER_DESTINATION;
use super::resolver::Resolver;
use super::socket::{Bip6Socket, Inbound};
use super::*;
use crate::log_capture::LogCapture;
use crate::port::{ReceivedNpdu, TransportPort};
use crate::udp_metadata::ReceivedDatagram;
use crate::warn_throttle::WARNING_INTERVAL;

const LOCAL_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x50]);
const PEER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x51]);
const OTHER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x52]);
const LOCAL_PORT: u16 = 0xBAC0;
/// A Who-Is.
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const GROUP: Ipv6Addr = BACNET_IPV6_MULTICAST_SITE_LOCAL;

fn local_ip() -> Ipv6Addr {
    Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10)
}

/// This node's own B/IPv6 address.
fn own() -> SocketAddrV6 {
    SocketAddrV6::new(local_ip(), LOCAL_PORT, 0, 0)
}

/// Another node on the link.
fn peer() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x30),
        0xBAC0,
        0,
        0,
    )
}

/// The BBMD a foreign-device node registers with.
fn bbmd() -> SocketAddrV6 {
    SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0xBAC0, 0, 0)
}

/// A node on another link: a Forwarded-NPDU's origin, or an asker.
fn station() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 9, 0, 0, 0, 0x20),
        0xBAC0,
        0,
        0,
    )
}

/// A source no node sends from.
fn group_source() -> SocketAddrV6 {
    SocketAddrV6::new("ff02::1".parse().unwrap(), 0xBAC0, 0, 0)
}

/// A receiver and the network layer's end of its channel.
struct Node {
    receiver: Receiver,
    rx: mpsc::Receiver<ReceivedNpdu>,
}

impl Node {
    /// A node at 2001:db8::10, a foreign device of [`bbmd`] when `foreign`,
    /// whose channel to the network layer holds `capacity` NPDUs. Its
    /// socket is never used: replies are returned, not sent.
    async fn new(foreign: bool, capacity: usize) -> Self {
        let socket = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
            .await
            .unwrap();
        let (tx, rx) = mpsc::channel(capacity);
        let receiver = Receiver {
            socket: Arc::new(socket),
            tx,
            local_port: LOCAL_PORT,
            vmac: LOCAL_VMAC,
            local_ip: local_ip(),
            unicast_ips: vec![local_ip()],
            wildcard_bind: false,
            foreign_bbmd: foreign.then(|| super::foreign_target::ForeignTarget::at(bbmd())),
            neighbours: Resolver::with_own(LOCAL_VMAC),
            forwarded_group_origin_drops: Arc::default(),
            drops: Arc::default(),
            nak_quota: Default::default(),
            far_ack_quota: Default::default(),
        };
        Self { receiver, rx }
    }

    /// Feed `datagram` from `from`, sent to `to`.
    async fn feed(&self, datagram: &[u8], from: SocketAddrV6, to: Ipv6Addr) -> Option<Reply> {
        let received = ReceivedDatagram {
            len: datagram.len(),
            peer: SocketAddr::V6(from),
            destination: IpAddr::V6(to),
            arrival_index: Some(1),
            os_group_delivery: None,
            arrival_stamp: None,
        };
        self.receiver.handle_datagram(datagram, &received).await
    }

    /// Feed `message` from `from`, sent to `to`.
    async fn send(&self, message: Bvll6<'_>, from: SocketAddrV6, to: Ipv6Addr) -> Option<Reply> {
        self.feed(&encoded(&message).unwrap(), from, to).await
    }

    fn counters(&self) -> Bip6ReceiveCounters {
        self.receiver.drops.snapshot()
    }

    fn nothing_handed_up(&mut self) {
        assert!(self.rx.try_recv().is_err(), "an NPDU was handed up");
    }
}

/// Counters where only what `set` writes moved.
fn only(set: impl FnOnce(&mut Bip6ReceiveCounters)) -> Bip6ReceiveCounters {
    let mut counters = Bip6ReceiveCounters::default();
    set(&mut counters);
    counters
}

fn unicast_to(destination: Bip6Vmac) -> Bvll6<'static> {
    Bvll6::OriginalUnicastNpdu {
        source: PEER_VMAC,
        destination,
        npdu: NPDU,
    }
}

fn broadcast() -> Bvll6<'static> {
    Bvll6::OriginalBroadcastNpdu {
        source: PEER_VMAC,
        npdu: NPDU,
    }
}

fn forwarded(origin: SocketAddrV6, npdu: &[u8]) -> BytesMut {
    encoded(&Bvll6::ForwardedNpdu {
        original_source: PEER_VMAC,
        original_address: origin.into(),
        npdu,
    })
    .unwrap()
}

fn forwarded_resolution(target: Bip6Vmac, asker: SocketAddrV6) -> Bvll6<'static> {
    Bvll6::ForwardedAddressResolution {
        original_source: PEER_VMAC,
        target,
        original_address: asker.into(),
    }
}

fn register() -> Bvll6<'static> {
    Bvll6::RegisterForeignDevice {
        source: PEER_VMAC,
        ttl: 60,
    }
}

#[tokio::test]
async fn well_formed_datagrams_count_nothing() {
    let mut node = Node::new(false, 8).await;
    node.send(unicast_to(LOCAL_VMAC), peer(), local_ip()).await;
    node.send(broadcast(), peer(), GROUP).await;
    node.feed(&forwarded(station(), NPDU), peer(), GROUP).await;
    for _ in 0..3 {
        assert_eq!(node.rx.try_recv().unwrap().npdu.as_ref(), NPDU);
    }
    // Messages that are handled, with or without an answer.
    let var = Bvll6::VirtualAddressResolution { source: PEER_VMAC };
    assert!(node.send(var, peer(), local_ip()).await.is_some());
    assert!(node.send(register(), peer(), local_ip()).await.is_some());
    let resolution = Bvll6::AddressResolution {
        source: PEER_VMAC,
        target: OTHER_VMAC,
    };
    assert_eq!(node.send(resolution, peer(), GROUP).await, None);
    let elsewhere = forwarded_resolution(OTHER_VMAC, station());
    assert_eq!(node.send(elsewhere, peer(), GROUP).await, None);
    let ack = Bvll6::AddressResolutionAck {
        source: PEER_VMAC,
        destination: LOCAL_VMAC,
    };
    assert_eq!(node.send(ack, peer(), local_ip()).await, None);
    let result = Bvll6::BvlcResult {
        source: PEER_VMAC,
        code: Bvlc6ResultCode::SUCCESSFUL_COMPLETION,
    };
    assert_eq!(node.send(result, peer(), local_ip()).await, None);
    node.nothing_handed_up();
    assert_eq!(node.counters(), Bip6ReceiveCounters::default());
}

#[tokio::test]
async fn failed_reads_and_unusable_metadata_count_as_receive_errors() {
    let drops = ReceiveDrops::default();
    let invalid = io::Error::new(io::ErrorKind::InvalidData, "truncated");
    let fatal = io::Error::other("socket closed");
    assert!(receive_failed(&invalid, &drops), "one datagram lost");
    assert!(!receive_failed(&fatal, &drops), "the loop stops");
    assert_eq!(drops.snapshot(), only(|c| c.receive_errors = 2));

    let mut node = Node::new(false, 8).await;
    let invalid = io::Error::new(io::ErrorKind::InvalidData, "truncated");
    assert!(node.receiver.on_read(Err(invalid), &[]).await);
    // An IPv6 socket that reported an IPv4 peer.
    let frame = encoded(&unicast_to(LOCAL_VMAC)).unwrap();
    let received = ReceivedDatagram {
        len: frame.len(),
        peer: "192.0.2.30:47808".parse().unwrap(),
        destination: IpAddr::V6(local_ip()),
        arrival_index: Some(1),
        os_group_delivery: None,
        arrival_stamp: None,
    };
    assert_eq!(node.receiver.handle_datagram(&frame, &received).await, None);
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.receive_errors = 2));
}

#[tokio::test]
async fn a_datagram_for_another_link_counts_as_link_mismatch() {
    let node = Node::new(false, 8).await;
    assert!(node.receiver.on_read(Ok(Inbound::OffLink), &[]).await);
    assert!(node.receiver.on_read(Ok(Inbound::OffLink), &[]).await);
    assert_eq!(node.counters(), only(|c| c.link_mismatch = 2));
}

#[tokio::test]
async fn a_datagram_shorter_than_a_header_counts_as_too_short() {
    let mut node = Node::new(false, 8).await;
    node.feed(&[], peer(), local_ip()).await;
    node.feed(&[0x82, 0x01, 0x00], peer(), local_ip()).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.too_short = 2));
}

#[tokio::test]
async fn a_datagram_of_another_type_counts_as_not_bip6() {
    let mut node = Node::new(false, 8).await;
    // A B/IP Original-Unicast-NPDU's type octet.
    let mut datagram = encoded(&unicast_to(LOCAL_VMAC)).unwrap();
    datagram[0] = 0x81;
    node.feed(&datagram, peer(), local_ip()).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.not_bip6 = 1));
}

#[tokio::test]
async fn a_length_field_that_disagrees_counts_as_length_mismatch() {
    let mut node = Node::new(false, 8).await;
    let frame = encoded(&unicast_to(LOCAL_VMAC)).unwrap();
    let mut padded = frame.to_vec();
    padded.push(0x00);
    node.feed(&padded, peer(), local_ip()).await;
    node.feed(&frame[..frame.len() - 1], peer(), local_ip())
        .await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.length_mismatch = 2));
}

#[tokio::test]
async fn an_unknown_function_counts_as_unknown_function() {
    let node = Node::new(false, 8).await;
    // 0x0B's clause was removed from Annex U; 0x0D was never defined.
    for function in [0x0B, 0x0D] {
        let frame = raw_frame(function, &PEER_VMAC.octets());
        assert_eq!(node.feed(&frame, peer(), local_ip()).await, None);
    }
    assert_eq!(node.counters(), only(|c| c.unknown_function = 2));
}

/// A payload of the wrong length for its function. A malformed BBMD request
/// still draws its NAK (malformed_request_tests), counted once.
#[tokio::test]
async fn a_payload_that_does_not_fit_its_function_counts_as_malformed() {
    let mut node = Node::new(false, 8).await;
    let vmac = PEER_VMAC.octets();
    for (function, payload, to) in [
        (
            Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
            &vmac[..2],
            local_ip(),
        ),
        (Bvlc6Function::BVLC_RESULT, &vmac[..], local_ip()),
        (Bvlc6Function::ADDRESS_RESOLUTION, &vmac[..], GROUP),
        (Bvlc6Function::FORWARDED_NPDU, &vmac[..], GROUP),
        (Bvlc6Function::ORIGINAL_UNICAST_NPDU, &vmac[..], local_ip()),
    ] {
        let frame = raw_frame(function.to_raw(), payload);
        assert_eq!(node.feed(&frame, peer(), to).await, None, "{function}");
    }
    let register = raw_frame(Bvlc6Function::REGISTER_FOREIGN_DEVICE.to_raw(), &vmac);
    assert!(node.feed(&register, peer(), local_ip()).await.is_some());
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.malformed_payload = 6));
}

/// U.3: a directed frame or an ACK naming another node's VMAC.
#[tokio::test]
async fn a_message_for_another_vmac_counts_as_destination_vmac_mismatch() {
    let mut node = Node::new(false, 8).await;
    node.send(unicast_to(OTHER_VMAC), peer(), local_ip()).await;
    let ack = Bvll6::AddressResolutionAck {
        source: PEER_VMAC,
        destination: OTHER_VMAC,
    };
    node.send(ack, peer(), local_ip()).await;
    let virtual_ack = Bvll6::VirtualAddressResolutionAck {
        source: PEER_VMAC,
        destination: OTHER_VMAC,
    };
    node.send(virtual_ack, peer(), local_ip()).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.destination_vmac_mismatch = 3));
}

#[tokio::test]
async fn a_function_sent_to_the_wrong_address_counts_as_destination_mismatch() {
    let mut node = Node::new(false, 8).await;
    let var = Bvll6::VirtualAddressResolution { source: PEER_VMAC };
    let resolution = Bvll6::AddressResolution {
        source: PEER_VMAC,
        target: LOCAL_VMAC,
    };
    for (message, to) in [
        (broadcast(), local_ip()),
        (unicast_to(LOCAL_VMAC), GROUP),
        (var, GROUP),
        (register(), GROUP),
        (resolution, local_ip()),
    ] {
        assert_eq!(node.send(message, peer(), to).await, None, "{message:?}");
    }
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.destination_mismatch = 5));
}

#[tokio::test]
async fn a_forwarded_npdu_by_an_untrusted_path_counts_as_untrusted() {
    // Normal mode takes one from a BACnet group only.
    let mut node = Node::new(false, 8).await;
    node.feed(&forwarded(station(), NPDU), peer(), local_ip())
        .await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.untrusted_forwarded_npdu = 1));

    // A foreign device takes one only by unicast from its BBMD.
    let mut node = Node::new(true, 8).await;
    node.feed(&forwarded(station(), NPDU), peer(), local_ip())
        .await;
    node.feed(&forwarded(station(), NPDU), bbmd(), GROUP).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.untrusted_forwarded_npdu = 2));
}

#[tokio::test]
async fn a_forwarded_resolution_by_an_untrusted_path_counts_as_untrusted() {
    let request = forwarded_resolution(LOCAL_VMAC, station());
    // Normal mode answers one from a BACnet group, from an answerable source.
    let node = Node::new(false, 8).await;
    assert_eq!(node.send(request, peer(), local_ip()).await, None);
    assert_eq!(node.send(request, group_source(), GROUP).await, None);
    let expected = only(|c| c.untrusted_forwarded_resolution = 2);
    assert_eq!(node.counters(), expected);

    // A foreign device answers one by unicast from its BBMD only.
    let node = Node::new(true, 8).await;
    assert_eq!(node.send(request, peer(), local_ip()).await, None);
    assert_eq!(node.send(request, bbmd(), GROUP).await, None);
    assert_eq!(node.counters(), expected);
}

/// An NPDU too short for its version and control octets is dropped before
/// the origin is looked at; one with an unsupported version once the origin
/// has passed.
#[tokio::test]
async fn a_forwarded_npdu_that_is_not_an_npdu_counts_as_malformed() {
    let mut node = Node::new(false, 8).await;
    node.feed(&forwarded(station(), &[0x01]), peer(), GROUP)
        .await;
    let link_local = SocketAddrV6::new("fe80::20".parse().unwrap(), 0xBAC0, 0, 0);
    node.feed(&forwarded(link_local, &[]), peer(), GROUP).await;
    node.feed(&forwarded(station(), &[0x02, 0x00]), peer(), GROUP)
        .await;
    node.nothing_handed_up();
    assert_eq!(node.receiver.neighbours.lookup(PEER_VMAC), None);
    assert_eq!(node.counters(), only(|c| c.malformed_forwarded_npdu = 3));
}

/// A Forwarded-NPDU's origin, or the asker of a Forwarded-Address-Resolution
/// that names this node, that can't be used without a scope ID. One that
/// names another node is not this node's to answer, and not a drop.
#[tokio::test]
async fn an_address_unusable_without_a_scope_counts_as_unusable() {
    let mut node = Node::new(false, 8).await;
    let link_local = SocketAddrV6::new("fe80::20".parse().unwrap(), 0xBAC0, 0, 0);
    let loopback = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0xBAC0, 0, 0);
    node.feed(&forwarded(link_local, NPDU), peer(), GROUP).await;
    let ours = forwarded_resolution(LOCAL_VMAC, loopback);
    assert_eq!(node.send(ours, peer(), GROUP).await, None);
    let theirs = forwarded_resolution(OTHER_VMAC, loopback);
    assert_eq!(node.send(theirs, peer(), GROUP).await, None);
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.unusable_address = 2));
}

/// A request this node would answer, from a source no node sends from or
/// from this node's own address. An Address-Resolution for another VMAC
/// asks nothing of this node, so its source doesn't matter.
#[tokio::test]
async fn a_request_from_an_unanswerable_source_counts_as_unanswerable() {
    let node = Node::new(false, 8).await;
    let var = Bvll6::VirtualAddressResolution { source: PEER_VMAC };
    let ours = Bvll6::AddressResolution {
        source: PEER_VMAC,
        target: LOCAL_VMAC,
    };
    let theirs = Bvll6::AddressResolution {
        source: PEER_VMAC,
        target: OTHER_VMAC,
    };
    let port_zero = SocketAddrV6::new(*peer().ip(), 0, 0, 0);
    for (message, from, to) in [
        (var, group_source(), local_ip()),
        (ours, port_zero, GROUP),
        (register(), own(), local_ip()),
        (theirs, port_zero, GROUP),
    ] {
        assert_eq!(node.send(message, from, to).await, None, "{message:?}");
    }
    assert_eq!(node.counters(), only(|c| c.unanswerable_request = 3));
}

/// An NPDU from a source no answer could reach is never learned, so its
/// source MAC would name a VMAC this node can't reach at that address. It
/// is dropped and counted, not handed up (#1637); from an answerable
/// source the same frame goes up.
#[tokio::test]
async fn an_npdu_from_an_unanswerable_source_counts_as_unanswerable_source() {
    let mut node = Node::new(false, 8).await;
    let sources = [
        group_source(),
        SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0xBAC0, 0, 0),
        SocketAddrV6::new("::ffff:192.0.2.30".parse().unwrap(), 0xBAC0, 0, 0),
        SocketAddrV6::new(*peer().ip(), 0, 0, 0),
    ];
    for source in sources {
        node.send(unicast_to(LOCAL_VMAC), source, local_ip()).await;
        node.send(broadcast(), source, GROUP).await;
    }
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.unanswerable_source = 8));
    assert_eq!(node.receiver.neighbours.lookup(PEER_VMAC), None);
    node.send(unicast_to(LOCAL_VMAC), peer(), local_ip()).await;
    assert_eq!(node.rx.try_recv().unwrap().npdu.as_ref(), NPDU);
}

#[tokio::test]
async fn frames_from_this_node_count_as_own_echo() {
    let mut node = Node::new(false, 8).await;
    node.send(unicast_to(LOCAL_VMAC), own(), local_ip()).await;
    node.send(broadcast(), own(), GROUP).await;
    node.nothing_handed_up();
    assert_eq!(node.counters(), only(|c| c.own_echo = 2));
}

/// A Forwarded-NPDU by a trusted path whose origin is this node: its own
/// broadcast come back through a BBMD. It is neither handed up nor learned,
/// though the same frame from another origin would be.
#[tokio::test]
async fn a_forwarded_npdu_from_this_node_counts_as_own_echo() {
    // From a BACnet group in normal mode, and from the configured BBMD by
    // unicast for a foreign device.
    for (foreign, from, to) in [(false, peer(), GROUP), (true, bbmd(), local_ip())] {
        let mut node = Node::new(foreign, 8).await;
        node.feed(&forwarded(own(), NPDU), from, to).await;
        node.nothing_handed_up();
        assert_eq!(node.receiver.neighbours.lookup(PEER_VMAC), None);
        assert_eq!(
            node.counters(),
            only(|c| c.own_echo = 1),
            "foreign={foreign}"
        );

        node.feed(&forwarded(station(), NPDU), from, to).await;
        assert_eq!(node.rx.try_recv().unwrap().npdu.as_ref(), NPDU);
        assert_eq!(node.counters(), only(|c| c.own_echo = 1));
    }
}

#[tokio::test]
async fn a_full_channel_counts_each_npdu_it_drops() {
    let mut node = Node::new(false, 1).await;
    node.send(unicast_to(LOCAL_VMAC), peer(), local_ip()).await;
    assert_eq!(node.counters(), Bip6ReceiveCounters::default());
    node.send(unicast_to(LOCAL_VMAC), peer(), local_ip()).await;
    node.send(broadcast(), peer(), GROUP).await;
    node.feed(&forwarded(station(), NPDU), peer(), GROUP).await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_full = 3));
    assert!(node.rx.try_recv().is_ok(), "the first NPDU is handed up");
    node.nothing_handed_up();
}

/// Once the network layer has closed its end, every NPDU is dropped as
/// closed, not full.
#[tokio::test]
async fn a_closed_channel_counts_each_npdu_it_drops() {
    let mut node = Node::new(false, 8).await;
    node.rx.close();
    node.send(unicast_to(LOCAL_VMAC), peer(), local_ip()).await;
    node.send(broadcast(), peer(), GROUP).await;
    node.feed(&forwarded(station(), NPDU), peer(), GROUP).await;
    assert_eq!(node.counters(), only(|c| c.npdu_channel_closed = 3));
}

/// A NAK the quota holds back is a reply suppressed: the request was
/// handled, so it counts nowhere else.
#[tokio::test(start_paused = true)]
async fn a_nak_over_the_quota_counts_as_suppressed() {
    let node = Node::new(false, 8).await;
    for _ in 0..REPLIES_PER_DESTINATION {
        assert!(node.send(register(), peer(), local_ip()).await.is_some());
    }
    assert_eq!(node.send(register(), peer(), local_ip()).await, None);
    assert_eq!(node.counters(), only(|c| c.naks_suppressed = 1));
}

#[tokio::test(start_paused = true)]
async fn a_far_answer_over_the_quota_counts_as_suppressed() {
    let node = Node::new(true, 8).await;
    let request = forwarded_resolution(LOCAL_VMAC, station());
    for _ in 0..REPLIES_PER_DESTINATION {
        assert!(node.send(request, bbmd(), local_ip()).await.is_some());
    }
    assert_eq!(node.send(request, bbmd(), local_ip()).await, None);
    assert_eq!(node.counters(), only(|c| c.far_acks_suppressed = 1));
}

/// A full channel warns at most once per interval, with the drops since the
/// last warning; the rest log at debug.
#[tokio::test(start_paused = true)]
async fn the_channel_full_warning_goes_out_at_most_once_per_interval() {
    let (logs, _guard) = LogCapture::start();
    let node = Node::new(false, 1).await;
    for _ in 0..4 {
        node.send(broadcast(), peer(), GROUP).await;
    }
    tokio::time::advance(WARNING_INTERVAL).await;
    node.send(broadcast(), peer(), GROUP).await;
    let full = logs.matching("B/IPv6: NPDU channel full");
    let seen: Vec<_> = full
        .iter()
        .map(|event| (event.level, event.fields.get("dropped").cloned()))
        .collect();
    let warned = |dropped: &str| (Level::WARN, Some(dropped.to_string()));
    assert_eq!(
        seen,
        [
            warned("1"),
            (Level::DEBUG, None),
            (Level::DEBUG, None),
            warned("3"),
        ]
    );
    assert_eq!(node.counters(), only(|c| c.npdu_channel_full = 4));
}

/// Anyone can send a Virtual-Address-Resolution-ACK from this node's VMAC,
/// or a BVLC-Result NAK, so neither logs above debug more than once per
/// interval. The ACK is dropped and counted as from this node's VMAC; the
/// NAK is handled, not a drop.
#[tokio::test(start_paused = true)]
async fn collision_and_nak_logs_go_out_at_most_once_per_interval() {
    let (logs, _guard) = LogCapture::start();
    let node = Node::new(false, 8).await;
    let collision = Bvll6::VirtualAddressResolutionAck {
        source: LOCAL_VMAC,
        destination: LOCAL_VMAC,
    };
    let nak = Bvll6::BvlcResult {
        source: PEER_VMAC,
        code: Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
    };
    for _ in 0..3 {
        node.send(collision, peer(), local_ip()).await;
        node.send(nak, peer(), local_ip()).await;
    }
    tokio::time::advance(WARNING_INTERVAL).await;
    node.send(collision, peer(), local_ip()).await;
    node.send(nak, peer(), local_ip()).await;

    let collisions = logs.matching("BIP6 VMAC collision");
    let levels: Vec<_> = collisions.iter().map(|event| event.level).collect();
    let [first, .., last] = &collisions[..] else {
        panic!("{collisions:?}")
    };
    assert_eq!(
        levels,
        [Level::WARN, Level::DEBUG, Level::DEBUG, Level::WARN]
    );
    assert_eq!(
        (&first.fields["seen"][..], &last.fields["seen"][..]),
        ("1", "3")
    );

    let naks = logs.matching("BIP6: BVLC-Result NAK");
    let levels: Vec<_> = naks.iter().map(|event| event.level).collect();
    assert_eq!(
        levels,
        [Level::ERROR, Level::DEBUG, Level::DEBUG, Level::ERROR]
    );
    assert_eq!(naks[3].fields["naks"], "3");
    assert_eq!(node.counters(), only(|c| c.own_vmac_source = 4));
}

/// Through a running transport's socket: a datagram larger than the receive
/// buffer and one too short for a header are counted, the next is answered,
/// and the totals survive a restart.
#[tokio::test]
async fn a_running_transport_reports_its_drops() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x0650));
    let _rx = transport.start().await.unwrap();
    let port = transport.local_address().unwrap().port();
    let to = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();

    sender.send_to(&[0x82; 4096], to).await.unwrap();
    sender.send_to(&[0x82, 0x06], to).await.unwrap();
    let var = encoded(&Bvll6::VirtualAddressResolution { source: PEER_VMAC }).unwrap();
    sender.send_to(&var, to).await.unwrap();
    let mut buf = [0u8; 64];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), sender.recv_from(&mut buf))
        .await
        .expect("the request after the drops is answered")
        .unwrap();
    assert!(matches!(
        decode_bvll6(&buf[..len]).unwrap(),
        Bvll6::VirtualAddressResolutionAck { .. }
    ));
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
