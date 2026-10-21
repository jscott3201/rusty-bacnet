//! The replies the B/IPv6 receive path owes (#1632): the NAKs a node that
//! isn't a BBMD sends (U.4.4, U.4.5.3, U.4.5.4), and the
//! Address-Resolution-ACK for an Address-Resolution or a trusted
//! Forwarded-Address-Resolution that names this node (U.5; those are in
//! [`forwarded_resolution_tests`]). Datagrams are fed in-process with the
//! addressing the OS would report, and each test reads the reply
//! `handle_datagram` returns for the loop to send.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::frame::encoded;
use super::receive::{Receiver, Reply};
use super::reply_limit::{REPLIES_PER_DESTINATION, REPLIES_PER_WINDOW, REPLY_WINDOW};
use super::resolver::Resolver;
use super::socket::Bip6Socket;
use super::*;
use crate::port::TransportPort;
use crate::udp_metadata::ReceivedDatagram;

mod forwarded_resolution_tests;
mod malformed_request_tests;

const LOCAL_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x32]);
const ASKER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x40]);
const OTHER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x06, 0x41]);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];
const LOCAL_PORT: u16 = 0xBAC0;

fn local_ip() -> Ipv6Addr {
    Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10)
}

/// A node on another link, the source of most requests here.
fn sender() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x30),
        0xBAC0,
        0,
        0,
    )
}

/// The BBMD a foreign-device receiver registers with.
fn bbmd() -> SocketAddrV6 {
    SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0xBAC0, 0, 0)
}

/// The asker a Forwarded-Address-Resolution names, on yet another link.
fn asker() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 9, 0, 0, 0, 0x40),
        0xBAC1,
        0,
        0,
    )
}

/// Sources no node sends from: a group, the unspecified address, an
/// IPv4-mapped address on an IPv6-only socket, port 0.
fn unanswerable_sources() -> [SocketAddrV6; 4] {
    [
        SocketAddrV6::new("ff02::1".parse().unwrap(), 0xBAC0, 0, 0),
        SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0xBAC0, 0, 0),
        SocketAddrV6::new("::ffff:192.0.2.30".parse().unwrap(), 0xBAC0, 0, 0),
        SocketAddrV6::new(*sender().ip(), 0, 0, 0),
    ]
}

/// A receiver for `2001:db8::10`, a foreign device of [`bbmd`] when
/// `foreign`. Its socket is never used: the tests read returned replies.
async fn receiver(foreign: bool) -> Receiver {
    let socket = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
        .await
        .unwrap();
    let (tx, _) = mpsc::channel(4);
    Receiver {
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
    }
}

fn datagram(data: &[u8], peer: SocketAddrV6, destination: Ipv6Addr) -> ReceivedDatagram {
    ReceivedDatagram {
        len: data.len(),
        peer: SocketAddr::V6(peer),
        destination: IpAddr::V6(destination),
        arrival_index: Some(1),
        os_group_delivery: None,
        arrival_stamp: None,
    }
}

/// The reply to `message` from `peer`, sent to `destination`.
async fn reply(
    receiver: &Receiver,
    message: Bvll6<'_>,
    peer: SocketAddrV6,
    destination: Ipv6Addr,
) -> Option<Reply> {
    let frame = encoded(&message).unwrap();
    receiver
        .handle_datagram(&frame, &datagram(&frame, peer, destination))
        .await
}

fn register() -> Bvll6<'static> {
    Bvll6::RegisterForeignDevice {
        source: ASKER_VMAC,
        ttl: 60,
    }
}

fn delete_entry() -> Bvll6<'static> {
    Bvll6::DeleteForeignDeviceTableEntry {
        source: ASKER_VMAC,
        entry: asker().into(),
    }
}

fn distribute() -> Bvll6<'static> {
    Bvll6::DistributeBroadcastToNetwork {
        original_source: ASKER_VMAC,
        npdu: NPDU,
    }
}

fn address_resolution(target: Bip6Vmac) -> Bvll6<'static> {
    Bvll6::AddressResolution {
        source: ASKER_VMAC,
        target,
    }
}

fn virtual_resolution() -> Bvll6<'static> {
    Bvll6::VirtualAddressResolution { source: ASKER_VMAC }
}

fn forwarded_resolution(target: Bip6Vmac, asker: SocketAddrV6) -> Bvll6<'static> {
    Bvll6::ForwardedAddressResolution {
        original_source: ASKER_VMAC,
        target,
        original_address: asker.into(),
    }
}

fn ack_to(to: SocketAddrV6) -> Reply {
    Reply {
        to: to.into(),
        message: Bvll6::AddressResolutionAck {
            source: LOCAL_VMAC,
            destination: ASKER_VMAC,
        },
    }
}

/// U.4.4, U.4.5.3, U.4.5.4: each request a BBMD serves gets its own NAK,
/// from this node's VMAC, at the datagram's source, in either mode. The
/// octets are written out from U.2.1.1.
#[tokio::test]
async fn each_bbmd_request_gets_its_nak_at_its_source() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        for (request, code, octets) in [
            (
                register(),
                Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
                [0x00, 0x90],
            ),
            (
                delete_entry(),
                Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
                [0x00, 0xA0],
            ),
            (
                distribute(),
                Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
                [0x00, 0xC0],
            ),
        ] {
            let nak = reply(&receiver, request, sender(), local_ip()).await;
            let expected = Reply {
                to: sender().into(),
                message: Bvll6::BvlcResult {
                    source: LOCAL_VMAC,
                    code,
                },
            };
            assert_eq!(nak, Some(expected), "{request:?} foreign={foreign}");
            let wire = encoded(&expected.message).unwrap();
            assert_eq!(
                wire[..],
                [0x82, 0x00, 0x00, 0x09, 0x00, 0x06, 0x32, octets[0], octets[1]]
            );
        }
    }
}

/// A BBMD request sent to a group draws no NAK: one forged multicast must
/// not draw one from every member.
#[tokio::test]
async fn a_bbmd_request_to_a_group_is_not_answered() {
    let receiver = receiver(false).await;
    for request in [register(), delete_entry(), distribute()] {
        assert_eq!(
            reply(
                &receiver,
                request,
                sender(),
                BACNET_IPV6_MULTICAST_SITE_LOCAL
            )
            .await,
            None,
            "{request:?} to a group"
        );
        let frame = encoded(&request).unwrap();
        let mut flagged = datagram(&frame, sender(), local_ip());
        flagged.os_group_delivery = Some(true);
        assert_eq!(receiver.handle_datagram(&frame, &flagged).await, None);
    }
}

/// No request is answered at a source no node sends from: an answer there
/// would reach a whole group, or nobody. Each request is answered from a
/// usable source, so each refusal is for the source alone.
#[tokio::test]
async fn no_answer_goes_to_a_source_no_node_sends_from() {
    let receiver = receiver(false).await;
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    for (request, destination) in [
        (register(), local_ip()),
        (delete_entry(), local_ip()),
        (distribute(), local_ip()),
        (address_resolution(LOCAL_VMAC), group),
        (virtual_resolution(), local_ip()),
    ] {
        for source in unanswerable_sources() {
            assert_eq!(
                reply(&receiver, request, source, destination).await,
                None,
                "{request:?} from {source}"
            );
        }
        let answered = reply(&receiver, request, sender(), destination).await;
        assert_eq!(answered.map(|reply| reply.to), Some(sender().into()));
    }
}

/// A source no node sends from is never learned into the VMAC table, by any
/// message that teaches the sender's address; the same messages from an
/// answerable source are. Each sender gets an endpoint of its own, since a
/// newer VMAC at an endpoint replaces the older one.
#[tokio::test]
async fn a_source_no_node_sends_from_is_not_learned() {
    /// A message from a given VMAC, and where it is sent.
    type Kind = (fn(Bip6Vmac) -> Bvll6<'static>, Ipv6Addr);
    let receiver = receiver(false).await;
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    let kinds: [Kind; 3] = [
        (
            |source| Bvll6::AddressResolution {
                source,
                target: LOCAL_VMAC,
            },
            group,
        ),
        (
            |source| Bvll6::VirtualAddressResolution { source },
            local_ip(),
        ),
        (
            |source| Bvll6::OriginalBroadcastNpdu { source, npdu: NPDU },
            group,
        ),
    ];
    let mut checks = Vec::new();
    for (kind, (message, destination)) in (0u8..).zip(kinds) {
        let answerable = SocketAddrV6::new(*sender().ip(), 0xBAC0 + u16::from(kind), 0, 0);
        let mut sources = unanswerable_sources().to_vec();
        sources.push(answerable);
        for (index, source) in (0u8..).zip(sources) {
            let vmac = Bip6Vmac::new([0x00, 0x07 + kind, index]);
            let _ = reply(&receiver, message(vmac), source, destination).await;
            checks.push((vmac, source, (source == answerable).then_some(source)));
        }
    }
    for (vmac, source, expected) in checks {
        assert_eq!(
            receiver.neighbours.lookup(vmac),
            expected,
            "{vmac:?} from {source}"
        );
    }
}

/// This node never sends itself a request, so one from its own address is
/// forged and gets no answer: a NAK, Address-Resolution-ACK or
/// Virtual-Address-Resolution-ACK would only come back to this node. Another
/// port on the same address is another node and is answered.
#[tokio::test]
async fn a_request_from_this_nodes_own_address_is_not_answered() {
    let receiver = receiver(false).await;
    let own = SocketAddrV6::new(local_ip(), LOCAL_PORT, 0, 0);
    let neighbour = SocketAddrV6::new(local_ip(), LOCAL_PORT + 1, 0, 0);
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    for (request, destination) in [
        (register(), local_ip()),
        (delete_entry(), local_ip()),
        (distribute(), local_ip()),
        (address_resolution(LOCAL_VMAC), group),
        (virtual_resolution(), local_ip()),
    ] {
        assert_eq!(
            reply(&receiver, request, own, destination).await,
            None,
            "{request:?}"
        );
        let answer = reply(&receiver, request, neighbour, destination).await;
        assert_eq!(answer.map(|reply| reply.to), Some(neighbour.into()));
    }
}

/// U.5: an Address-Resolution for this node's VMAC is answered at its
/// source; one for another VMAC is not, though its sender is learned.
#[tokio::test]
async fn address_resolution_is_answered_only_for_this_vmac() {
    let receiver = receiver(false).await;
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    assert_eq!(
        reply(&receiver, address_resolution(OTHER_VMAC), sender(), group).await,
        None
    );
    assert_eq!(receiver.neighbours.lookup(ASKER_VMAC), Some(sender()));
    assert_eq!(
        reply(&receiver, address_resolution(LOCAL_VMAC), sender(), group).await,
        Some(ack_to(sender()))
    );
    assert_eq!(
        reply(
            &receiver,
            address_resolution(LOCAL_VMAC),
            sender(),
            local_ip()
        )
        .await,
        None,
        "an Address-Resolution goes to a group"
    );
}

/// Every reply goes to the datagram's source or, for a
/// Forwarded-Address-Resolution that came by a trusted path, to the asker's
/// address it carries: from the configured BBMD by unicast in foreign mode,
/// or on a BACnet group otherwise. That holds whatever the message, sender,
/// destination or mode. The window is moved on after each datagram, so a
/// quota never hides a reply.
#[tokio::test(start_paused = true)]
async fn replies_go_only_to_the_sender_or_a_trusted_askers_address() {
    let stranger = SocketAddrV6::new("2001:db8:0:7::66".parse().unwrap(), 0xBAC0, 0, 0);
    let messages = [
        register(),
        delete_entry(),
        distribute(),
        address_resolution(LOCAL_VMAC),
        address_resolution(OTHER_VMAC),
        forwarded_resolution(LOCAL_VMAC, asker()),
        forwarded_resolution(LOCAL_VMAC, stranger),
        forwarded_resolution(OTHER_VMAC, asker()),
        virtual_resolution(),
        Bvll6::AddressResolutionAck {
            source: ASKER_VMAC,
            destination: LOCAL_VMAC,
        },
        Bvll6::ForwardedNpdu {
            original_source: ASKER_VMAC,
            original_address: stranger.into(),
            npdu: NPDU,
        },
        Bvll6::OriginalBroadcastNpdu {
            source: ASKER_VMAC,
            npdu: NPDU,
        },
    ];
    let (mut to_sender, mut to_named) = (0, 0);
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        for message in messages {
            for peer in [sender(), bbmd(), stranger] {
                for destination in [local_ip(), BACNET_IPV6_MULTICAST_SITE_LOCAL] {
                    tokio::time::advance(REPLY_WINDOW).await;
                    let Some(reply) = reply(&receiver, message, peer, destination).await else {
                        continue;
                    };
                    if reply.to == SocketAddr::V6(peer) {
                        to_sender += 1;
                        continue;
                    }
                    let named = matches!(
                        message,
                        Bvll6::ForwardedAddressResolution { original_address, .. }
                            if reply.to == SocketAddr::V6(original_address.into())
                    );
                    let trusted = if foreign {
                        peer == bbmd() && destination == local_ip()
                    } else {
                        destination == BACNET_IPV6_MULTICAST_SITE_LOCAL
                    };
                    assert!(
                        named && trusted,
                        "{message:?} from {peer} to {destination} (foreign={foreign}) \
                         answered at {}",
                        reply.to
                    );
                    to_named += 1;
                }
            }
        }
    }
    assert!(to_sender > 0 && to_named > 0, "{to_sender} {to_named}");
}

/// The NAK quota is per destination address and totalled across addresses,
/// and a new window starts it afresh. Address-Resolution and
/// Virtual-Address-Resolution answers stay outside it.
#[tokio::test(start_paused = true)]
async fn the_nak_quota_caps_naks_per_address_and_in_total() {
    let receiver = receiver(true).await;
    for _ in 0..REPLIES_PER_DESTINATION {
        assert!(reply(&receiver, register(), sender(), local_ip())
            .await
            .is_some());
    }
    assert_eq!(
        reply(&receiver, register(), sender(), local_ip()).await,
        None
    );
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    assert_eq!(
        reply(&receiver, address_resolution(LOCAL_VMAC), sender(), group).await,
        Some(ack_to(sender())),
        "a collision probe is still answered"
    );
    assert!(reply(&receiver, virtual_resolution(), sender(), local_ip())
        .await
        .is_some());
    assert!(
        reply(&receiver, register(), asker(), local_ip())
            .await
            .is_some(),
        "another address has its own"
    );

    tokio::time::advance(REPLY_WINDOW).await;
    assert!(reply(&receiver, register(), sender(), local_ip())
        .await
        .is_some());

    tokio::time::advance(REPLY_WINDOW).await;
    let mut admitted = 0;
    for host in 0..=u16::try_from(REPLIES_PER_WINDOW / REPLIES_PER_DESTINATION).unwrap() {
        let source = SocketAddrV6::new(
            Ipv6Addr::new(0x2001, 0xdb8, 0, 3, 0, 0, 0, host),
            0xBAC0,
            0,
            0,
        );
        for _ in 0..REPLIES_PER_DESTINATION {
            admitted += u32::from(
                reply(&receiver, delete_entry(), source, local_ip())
                    .await
                    .is_some(),
            );
        }
    }
    assert_eq!(admitted, REPLIES_PER_WINDOW, "the total caps every address");
}

/// A flood of BBMD requests, spread over many source addresses and the
/// asker's own, spends the NAK quota but not the one the answers to a
/// Forwarded-Address-Resolution count against.
#[tokio::test(start_paused = true)]
async fn a_nak_flood_does_not_stop_a_far_answer() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        let mut sources = vec![asker()];
        let hosts = u16::try_from(REPLIES_PER_WINDOW).unwrap();
        sources.extend((0..hosts).map(|host| {
            SocketAddrV6::new(
                Ipv6Addr::new(0x2001, 0xdb8, 0, 4, 0, 0, 0, host),
                0xBAC0,
                0,
                0,
            )
        }));
        for source in sources {
            for _ in 0..REPLIES_PER_DESTINATION {
                let _ = reply(&receiver, register(), source, local_ip()).await;
            }
        }
        assert_eq!(
            reply(&receiver, register(), sender(), local_ip()).await,
            None,
            "the NAK quota is spent"
        );
        let (peer, destination) = if foreign {
            (bbmd(), local_ip())
        } else {
            (sender(), BACNET_IPV6_MULTICAST_SITE_LOCAL)
        };
        assert_eq!(
            reply(
                &receiver,
                forwarded_resolution(LOCAL_VMAC, asker()),
                peer,
                destination
            )
            .await,
            Some(ack_to(asker())),
            "foreign={foreign}"
        );
    }
}

/// End to end on loopback: a running transport sends a Register-Foreign-
/// Device NAK back to the requesting socket, and the loop keeps running.
#[tokio::test]
async fn a_running_transport_naks_a_registration() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x0632));
    let _rx = transport.start().await.unwrap();
    let port = transport.local_address().unwrap().port();
    let to = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let requester = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut buf = [0u8; 64];
    for (request, code) in [
        (register(), Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK),
        (
            distribute(),
            Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        ),
    ] {
        let frame: BytesMut = encoded(&request).unwrap();
        requester.send_to(&frame, to).await.unwrap();
        let (len, from) =
            tokio::time::timeout(Duration::from_secs(2), requester.recv_from(&mut buf))
                .await
                .expect("the NAK arrives")
                .unwrap();
        assert_eq!(from, SocketAddr::V6(to));
        assert_eq!(
            decode_bvll6(&buf[..len]).unwrap(),
            Bvll6::BvlcResult {
                source: LOCAL_VMAC,
                code,
            }
        );
    }
    transport.stop().await.unwrap();
}
