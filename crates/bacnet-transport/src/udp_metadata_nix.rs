//! The Unix receive path on nix's safe socket API (#1882): one direct
//! `recvmsg` with the control messages decoded by nix, and nix's socket
//! options. Where nix hands a message over undecoded, its bytes are parsed
//! here, safely.

use std::io::IoSliceMut;
use std::os::fd::AsRawFd;

use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, SockaddrStorage, recvmsg, setsockopt, sockopt,
};

use super::*;

/// Ask for the packet info that names each datagram's destination: IPv6
/// packet info, and for IPv4 `IP_PKTINFO` on Linux or `IP_RECVDSTADDR` plus
/// `IP_RECVIF` on macOS and the BSDs.
pub(super) fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    match version {
        IpVersion::V6 => setsockopt(udp_socket, sockopt::Ipv6RecvPacketInfo, &true)?,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        IpVersion::V4 => setsockopt(udp_socket, sockopt::Ipv4PacketInfo, &true)?,
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        IpVersion::V4 => {
            setsockopt(udp_socket, sockopt::Ipv4RecvDstAddr, &true)?;
            // IP_RECVDSTADDR carries no interface; IP_RECVIF adds it. Only a
            // B/IP transport sharing its port by address reads it, so
            // without it the socket still works.
            if let Err(error) = setsockopt(udp_socket, sockopt::Ipv4RecvIf, &true) {
                tracing::debug!(%error, "IP_RECVIF unavailable; arrival interfaces unknown");
            }
        }
    }
    Ok(())
}

/// One datagram from `udp_socket` into `buf`: its length, its sender and
/// what its control messages say about it. The call goes straight to the
/// kernel, so a datagram already queued reads whatever tokio's readiness
/// says (#1546).
pub(super) fn recv(
    udp_socket: &UdpSocket,
    buf: &mut [u8],
    version: IpVersion,
) -> io::Result<(usize, SocketAddr, PacketInfo)> {
    let mut control = ControlBuffer::new();
    let mut data = [IoSliceMut::new(buf)];
    let message = recvmsg::<SockaddrStorage>(
        udp_socket.as_raw_fd(),
        &mut data,
        Some(control.bytes_mut()),
        MsgFlags::empty(),
    )?;
    if message
        .flags
        .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
    {
        return Err(invalid_metadata("truncated UDP payload or packet metadata"));
    }
    let peer = message
        .address
        .as_ref()
        .and_then(socket_addr)
        .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
    let info = packet_info(message.cmsgs()?, version)?;
    Ok((message.bytes, peer, info))
}

/// The IPv4 or IPv6 socket address in `storage`, by its family.
fn socket_addr(storage: &SockaddrStorage) -> Option<SocketAddr> {
    match storage.as_sockaddr_in() {
        Some(address) => Some(SocketAddr::from(*address)),
        None => storage
            .as_sockaddr_in6()
            .map(|address| SocketAddr::from(*address)),
    }
}

/// What one control message says about a datagram.
pub(super) enum CmsgMetadata {
    /// Its destination, with the arrival interface when the same message
    /// carries it.
    Destination(IpAddr, Option<u32>),
    /// Only the arrival interface (`IP_RECVIF`).
    #[cfg_attr(any(target_os = "linux", target_os = "android"), allow(dead_code))]
    Interface(u32),
}

/// What the control messages of one datagram say about it, taken together:
/// exactly one destination, and the arrival interface and receive stamp
/// where they come.
pub(super) fn packet_info(
    messages: impl IntoIterator<Item = ControlMessageOwned>,
    version: IpVersion,
) -> io::Result<PacketInfo> {
    let mut destination = None;
    let mut interface = None;
    let mut arrival_stamp = None;
    for message in messages {
        if let Some(stamp) = stamps::arrival_stamp_cmsg(&message) {
            arrival_stamp = Some(stamp);
            continue;
        }
        match destination_cmsg(message, version)? {
            Some(CmsgMetadata::Destination(address, index)) => {
                if destination.replace(address).is_some() {
                    return Err(invalid_metadata("duplicate UDP destination metadata"));
                }
                interface = index.or(interface);
            }
            Some(CmsgMetadata::Interface(index)) => interface = Some(index),
            None => {}
        }
    }
    let destination =
        destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))?;
    Ok(PacketInfo {
        destination,
        arrival_index: interface,
        arrival_stamp,
    })
}

/// What one control message says about its datagram's destination, if it
/// is one this receive asked for.
fn destination_cmsg(
    message: ControlMessageOwned,
    version: IpVersion,
) -> io::Result<Option<CmsgMetadata>> {
    Ok(match (version, message) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        (IpVersion::V4, ControlMessageOwned::Ipv4PacketInfo(info)) => {
            let address = IpAddr::V4(Ipv4Addr::from(info.ipi_addr.s_addr.to_ne_bytes()));
            let index = u32::try_from(info.ipi_ifindex).ok();
            Some(CmsgMetadata::Destination(address, index))
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        (IpVersion::V4, ControlMessageOwned::Ipv4RecvDstAddr(address)) => {
            let address = IpAddr::V4(Ipv4Addr::from(address.s_addr.to_ne_bytes()));
            Some(CmsgMetadata::Destination(address, None))
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        (IpVersion::V4, ControlMessageOwned::Ipv4RecvIf(link)) => {
            Some(CmsgMetadata::Interface(u32::from(link.sdl_index)))
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "freebsd",
        ))]
        (IpVersion::V6, ControlMessageOwned::Ipv6PacketInfo(info)) => {
            let address = IpAddr::V6(Ipv6Addr::from(info.ipi6_addr.s6_addr));
            Some(CmsgMetadata::Destination(address, Some(info.ipi6_ifindex)))
        }
        // nix decodes IPv6 packet info only on the targets above; here it
        // comes as raw bytes.
        #[cfg(any(target_os = "dragonfly", target_os = "openbsd", target_os = "netbsd"))]
        (IpVersion::V6, ControlMessageOwned::Unknown(raw))
            if raw.cmsg_header.cmsg_level == libc::IPPROTO_IPV6
                && raw.cmsg_header.cmsg_type == libc::IPV6_PKTINFO =>
        {
            let (address, index) = ipv6_packet_info(&raw.data_bytes)?;
            Some(CmsgMetadata::Destination(IpAddr::V6(address), Some(index)))
        }
        _ => None,
    })
}

/// The destination and arrival interface an `in6_pktinfo` payload holds,
/// read from its bytes; `InvalidData` when it is too short to hold one.
#[cfg(any(
    test,
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
pub(super) fn ipv6_packet_info(payload: &[u8]) -> io::Result<(Ipv6Addr, u32)> {
    use std::mem::{offset_of, size_of};

    let short = || invalid_metadata("short IPv6 destination metadata");
    if payload.len() < size_of::<libc::in6_pktinfo>() {
        return Err(short());
    }
    let address = octets_at::<16>(payload, offset_of!(libc::in6_pktinfo, ipi6_addr));
    let index = octets_at(payload, offset_of!(libc::in6_pktinfo, ipi6_ifindex));
    let (address, index) = address.zip(index).ok_or_else(short)?;
    Ok((Ipv6Addr::from(address), libc::c_uint::from_ne_bytes(index)))
}
