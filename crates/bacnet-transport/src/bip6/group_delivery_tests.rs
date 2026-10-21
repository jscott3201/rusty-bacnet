//! An Original-Unicast-NPDU that was sent to a B/IPv6 multicast group is never
//! handed up as a directed NPDU (#1301). The receive path drops it, and the
//! NPDU's `link_layer_group` follows the address as well as the function.
//! Datagrams are fed in-process with the destination the OS would report.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::sync::mpsc;

use super::frame::{encoded, raw_frame};
use super::receive::Receiver;
use super::resolver::Resolver;
use super::socket::Bip6Socket;
use super::*;
use crate::port::ReceivedNpdu;
use crate::port::TransportPort;
use crate::udp_metadata::ReceivedDatagram;

/// A ReadProperty confirmed request in an NPDU that expects a reply.
const CONFIRMED_REQUEST: &[u8] = &[
    0x01, 0x04, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x55,
];
const LOCAL_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x05, 0x15]);
const SENDER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x05, 0x1E]);

fn local_ip() -> Ipv6Addr {
    Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10)
}

fn sender() -> SocketAddrV6 {
    SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x30),
        0xBAC0,
        0,
        0,
    )
}

/// A receiver bound in-process to `2001:db8::10` whose only listener is the
/// returned NPDU channel. The socket is only used for address-resolution
/// replies, which these tests never draw.
async fn receiver() -> (Receiver, mpsc::Receiver<ReceivedNpdu>) {
    let socket = Bip6Socket::bind(Ipv6Addr::LOCALHOST, 0, None)
        .await
        .unwrap();
    let (tx, rx) = mpsc::channel(4);
    let receiver = Receiver {
        socket: Arc::new(socket),
        tx,
        local_port: 0xBAC0,
        vmac: LOCAL_VMAC,
        local_ip: local_ip(),
        unicast_ips: vec![local_ip()],
        wildcard_bind: false,
        foreign_bbmd: None,
        neighbours: Resolver::with_own(LOCAL_VMAC),
        forwarded_group_origin_drops: Arc::default(),
        drops: Arc::default(),
        nak_quota: Default::default(),
        far_ack_quota: Default::default(),
    };
    (receiver, rx)
}

fn original_unicast() -> BytesMut {
    original_unicast_to(LOCAL_VMAC)
}

fn original_unicast_to(destination_vmac: Bip6Vmac) -> BytesMut {
    encoded(&Bvll6::OriginalUnicastNpdu {
        source: SENDER_VMAC,
        destination: destination_vmac,
        npdu: CONFIRMED_REQUEST,
    })
    .unwrap()
}

fn original_broadcast() -> BytesMut {
    encoded(&Bvll6::OriginalBroadcastNpdu {
        source: SENDER_VMAC,
        npdu: CONFIRMED_REQUEST,
    })
    .unwrap()
}

fn datagram(
    data: &[u8],
    destination: Ipv6Addr,
    os_group_delivery: Option<bool>,
) -> ReceivedDatagram {
    ReceivedDatagram {
        len: data.len(),
        peer: SocketAddr::V6(sender()),
        destination: IpAddr::V6(destination),
        arrival_index: Some(1),
        os_group_delivery,
        arrival_stamp: None,
    }
}

#[tokio::test]
async fn original_unicast_sent_to_a_multicast_group_is_dropped() {
    let (receiver, mut rx) = receiver().await;
    let unicast = original_unicast();

    // Each BACnet multicast scope, and a datagram to this node's own address
    // that Windows flags as a group delivery.
    for (destination, os) in [
        (BACNET_IPV6_MULTICAST_LINK_LOCAL, None),
        (BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
        (BACNET_IPV6_MULTICAST_ORG_LOCAL, None),
        (local_ip(), Some(true)),
    ] {
        receiver
            .handle_datagram(&unicast, &datagram(&unicast, destination, os))
            .await;
        assert!(rx.try_recv().is_err(), "{destination} {os:?}");
    }

    receiver
        .handle_datagram(&unicast, &datagram(&unicast, local_ip(), None))
        .await;
    let directed = rx
        .try_recv()
        .expect("a directed Original-Unicast-NPDU is handed up");
    assert_eq!(directed.npdu.as_ref(), CONFIRMED_REQUEST);
    assert!(!directed.link_layer_group);

    let broadcast = original_broadcast();
    receiver
        .handle_datagram(
            &broadcast,
            &datagram(&broadcast, BACNET_IPV6_MULTICAST_LINK_LOCAL, None),
        )
        .await;
    let group = rx
        .try_recv()
        .expect("an Original-Broadcast-NPDU to the group is handed up");
    assert!(group.link_layer_group);
}

#[tokio::test]
async fn original_unicast_takes_its_group_flag_from_the_destination() {
    let (receiver, mut rx) = receiver().await;
    let unicast = original_unicast();
    for (destination, os, group) in [
        (BACNET_IPV6_MULTICAST_LINK_LOCAL, None, true),
        (local_ip(), Some(true), true),
        (local_ip(), None, false),
    ] {
        let frame = decode_bvll6(&unicast).unwrap();
        receiver
            .dispatch(frame, sender(), &datagram(&unicast, destination, os))
            .await;
        let npdu = rx.try_recv().expect("dispatch hands the NPDU up");
        assert_eq!(npdu.link_layer_group, group, "{destination} {os:?}");
    }
}

/// A directed frame that names another node's virtual address is dropped,
/// even at this node's own unicast address (U.3).
#[tokio::test]
async fn original_unicast_for_another_vmac_is_dropped() {
    let (receiver, mut rx) = receiver().await;
    let other = original_unicast_to(Bip6Vmac::new([0x00, 0x05, 0x16]));
    receiver
        .handle_datagram(&other, &datagram(&other, local_ip(), None))
        .await;
    assert!(rx.try_recv().is_err());

    let ours = original_unicast();
    receiver
        .handle_datagram(&ours, &datagram(&ours, local_ip(), None))
        .await;
    assert!(
        rx.try_recv().is_ok(),
        "the same frame for this VMAC is handed up"
    );
}

/// The MAC is a VMAC, which always names one node (U.1.3): a B/IPv6
/// broadcast is the network layer's zero-length MAC. No MAC is this link's
/// broadcast or a group destination, an 18-octet multicast address
/// included, and the owned rule agrees with the live one (#1479, #1637).
#[test]
fn no_mac_is_a_group_destination() {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0xBAC0, None);
    let owned = transport.group_destinations();
    let multicast = |ip: Ipv6Addr| [&ip.octets()[..], &0xBAC0u16.to_be_bytes()].concat();
    let macs = [
        vec![0x00, 0x04, 0xD2],
        vec![0xFF, 0xFF, 0xFF],
        vec![0xFF],
        Vec::new(),
        multicast(BACNET_IPV6_MULTICAST_LINK_LOCAL),
        multicast("ff02::1".parse().unwrap()),
    ];
    for mac in macs {
        assert!(!transport.is_broadcast_mac(&mac), "{mac:02X?}");
        assert!(!transport.is_group_destination(&mac), "{mac:02X?}");
        assert!(!owned.contains(&mac), "{mac:02X?}");
    }
}

/// A Forwarded-NPDU from `origin`, carrying a ReadProperty request.
fn forwarded_from(origin: SocketAddrV6) -> BytesMut {
    encoded(&Bvll6::ForwardedNpdu {
        original_source: SENDER_VMAC,
        original_address: origin.into(),
        npdu: CONFIRMED_REQUEST,
    })
    .unwrap()
}

/// #1493: the original source of a Forwarded-NPDU becomes the NPDU's source,
/// so one that is a multicast group, a group destination at any port, makes
/// the frame malformed. It is dropped and counted, and never handed up; one
/// from a node's own address still is.
#[tokio::test]
async fn a_forwarded_npdu_from_a_multicast_origin_is_dropped_and_counted() {
    let (receiver, mut rx) = receiver().await;
    let origins: [(Ipv6Addr, u16); 3] = [
        (BACNET_IPV6_MULTICAST_SITE_LOCAL, 0xBAC0),
        ("ff02::1".parse().unwrap(), 0xBAC0),
        ("ff0e::1:3".parse().unwrap(), 0x1234),
    ];
    for (ip, port) in origins {
        let frame = forwarded_from(SocketAddrV6::new(ip, port, 0, 0));
        receiver
            .handle_datagram(
                &frame,
                &datagram(&frame, BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
            )
            .await;
        assert!(rx.try_recv().is_err(), "{ip}");
    }
    assert_eq!(
        receiver
            .forwarded_group_origin_drops
            .load(std::sync::atomic::Ordering::Relaxed),
        origins.len() as u64
    );

    let station = SocketAddrV6::new("2001:db8::20".parse().unwrap(), 0xBAC0, 0, 0);
    let frame = forwarded_from(station);
    receiver
        .handle_datagram(
            &frame,
            &datagram(&frame, BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
        )
        .await;
    let npdu = rx.try_recv().expect("a unicast origin is handed up");
    assert_eq!(npdu.source_mac.as_slice(), SENDER_VMAC.octets());
}

/// A Forwarded-NPDU whose NPDU is shorter than an NPDU's version and control
/// octets is dropped before its origin is looked at, as the old payload
/// decoder refused it (#1621): not handed up, not learned, and not counted
/// as a group origin even when it has one, but as a malformed NPDU (#1643).
/// Without a whole address the codec refuses the frame itself, as a
/// malformed payload.
#[tokio::test]
async fn a_forwarded_npdu_too_short_for_an_npdu_is_dropped_before_its_origin() {
    let (receiver, mut rx) = receiver().await;
    let group = Bip6Address::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, 0xBAC0);
    let station = Bip6Address::new("2001:db8::20".parse().unwrap(), 0xBAC0);
    for origin in [group, station] {
        for npdu in [&[][..], &[0x01]] {
            let frame = encoded(&Bvll6::ForwardedNpdu {
                original_source: SENDER_VMAC,
                original_address: origin,
                npdu,
            })
            .unwrap();
            receiver
                .handle_datagram(
                    &frame,
                    &datagram(&frame, BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
                )
                .await;
        }
    }
    let mut short_address = SENDER_VMAC.octets().to_vec();
    short_address.extend_from_slice(&group.to_octets()[..17]);
    let frame = raw_frame(Bvlc6Function::FORWARDED_NPDU.to_raw(), &short_address);
    receiver
        .handle_datagram(
            &frame,
            &datagram(&frame, BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
        )
        .await;

    assert!(rx.try_recv().is_err());
    assert_eq!(receiver.neighbours.lookup(SENDER_VMAC), None);
    assert_eq!(
        receiver
            .forwarded_group_origin_drops
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let expected = Bip6ReceiveCounters {
        malformed_forwarded_npdu: 4,
        malformed_payload: 1,
        ..Bip6ReceiveCounters::default()
    };
    assert_eq!(receiver.drops.snapshot(), expected);
}

/// The transport reports the receiver's count.
#[tokio::test]
async fn a_running_transport_counts_multicast_origins() {
    let bbmd = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
    let SocketAddr::V6(bbmd_addr) = bbmd.local_addr().unwrap() else {
        unreachable!()
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(45));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *bbmd_addr.ip(),
        bbmd_port: bbmd_addr.port(),
        ttl: 60,
    });
    let mut rx = transport.start().await.unwrap();
    let port = transport.local_address().unwrap().port();
    let to = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let station = SocketAddrV6::new("2001:db8::20".parse().unwrap(), 0xBAC0, 0, 0);
    let group = SocketAddrV6::new("ff02::1".parse().unwrap(), 0xBAC0, 0, 0);
    for origin in [group, station] {
        bbmd.send_to(&forwarded_from(origin), to).await.unwrap();
    }
    let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("the station's frame arrives")
        .unwrap();
    assert_eq!(
        received.source_mac.as_slice(),
        SENDER_VMAC.octets(),
        "the group origin went nowhere"
    );
    assert_eq!(
        transport.lookup_vmac(SENDER_VMAC),
        Some(station),
        "the station's address is learned for its VMAC"
    );
    assert_eq!(transport.forwarded_group_origin_drops(), 1);
    transport.stop().await.unwrap();
}
