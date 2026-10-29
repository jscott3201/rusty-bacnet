use super::unix::{ipv6_packet_info, packet_info};
use super::*;
use nix::sys::socket::{ControlMessageOwned, setsockopt, sockopt};
use std::mem::{offset_of, size_of};

fn destination() -> Ipv6Addr {
    "fd12::1".parse().unwrap()
}

/// The bytes of an `in6_pktinfo` naming `address` on interface `index`.
fn in6_pktinfo_bytes(address: Ipv6Addr, index: u32) -> Vec<u8> {
    let mut bytes = vec![0; size_of::<libc::in6_pktinfo>()];
    let at = offset_of!(libc::in6_pktinfo, ipi6_addr);
    bytes[at..at + 16].copy_from_slice(&address.octets());
    let at = offset_of!(libc::in6_pktinfo, ipi6_ifindex);
    bytes[at..at + 4].copy_from_slice(&index.to_ne_bytes());
    bytes
}

/// The IPv6 packet info a receive hands up, as nix gives it on this target.
fn packet_info_message(index: u32) -> ControlMessageOwned {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
    ))]
    return ControlMessageOwned::Ipv6PacketInfo(libc::in6_pktinfo {
        ipi6_addr: libc::in6_addr {
            s6_addr: destination().octets(),
        },
        ipi6_ifindex: index,
    });
    #[cfg(any(target_os = "dragonfly", target_os = "openbsd", target_os = "netbsd"))]
    return ControlMessageOwned::Unknown(nix::sys::socket::UnknownCmsg {
        cmsg_header: libc::cmsghdr {
            cmsg_len: 0,
            cmsg_level: libc::IPPROTO_IPV6,
            cmsg_type: libc::IPV6_PKTINFO,
        },
        data_bytes: in6_pktinfo_bytes(destination(), index),
    });
}

fn decode_control(index: u32, duplicate: bool) -> io::Result<(IpAddr, Option<u32>)> {
    let copies = if duplicate { 2 } else { 1 };
    let messages = (0..copies).map(|_| packet_info_message(index));
    packet_info(messages, IpVersion::V6).map(|info| {
        assert_eq!(info.arrival_stamp, None);
        (info.destination, info.arrival_index)
    })
}

#[test]
fn packet_info_keeps_destination_and_arrival_index_together() {
    assert_eq!(
        decode_control(7, false).unwrap(),
        (IpAddr::V6(destination()), Some(7))
    );
    // Zero is retained as untrusted metadata; selected-link admission rejects it.
    assert_eq!(decode_control(0, false).unwrap().1, Some(0));
}

#[test]
fn duplicate_or_missing_ipv6_packet_info_is_invalid() {
    assert_eq!(
        decode_control(7, true).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let missing = packet_info([], IpVersion::V6).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::InvalidData);
}

/// Where nix hands IPv6 packet info over as bytes (DragonFly, OpenBSD and
/// NetBSD), a payload too short for an `in6_pktinfo` is invalid. This is
/// those targets' guarantee only: on Linux, Android, Apple and FreeBSD nix
/// decodes the struct itself and trusts the kernel's length, and the zeroed
/// control buffer keeps that read in bounds.
#[test]
fn bytes_parsed_ipv6_packet_info_rejects_a_short_payload() {
    let whole = in6_pktinfo_bytes(destination(), 7);
    assert_eq!(ipv6_packet_info(&whole).unwrap(), (destination(), 7));
    assert_eq!(
        ipv6_packet_info(&whole[..whole.len() - 1])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

/// A datagram that arrives without its packet info, here because the
/// option was turned off again after configuring, fails the receive as
/// invalid metadata rather than passing up with a guessed destination.
#[tokio::test]
async fn an_ipv6_datagram_without_packet_info_is_invalid() {
    let socket = Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None).unwrap();
    socket.set_only_v6(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    let receiver = DestinationReceiver::configure(&socket, IpVersion::V6).unwrap();
    setsockopt(&socket, sockopt::Ipv6RecvPacketInfo, &false).unwrap();
    socket
        .bind(&SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0).into())
        .unwrap();
    let socket = UdpSocket::from_std(socket.into()).unwrap();
    let sender = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    sender
        .send_to(b"v6", socket.local_addr().unwrap())
        .await
        .unwrap();

    let mut buf = [0u8; 8];
    let error = receiver.recv_from(&socket, &mut buf).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
