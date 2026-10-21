//! The Unix receive path on libc, for the targets nix doesn't cover here
//! (#1882): illumos, Solaris and NTO, where nix has no `IPV6_RECVPKTINFO`
//! option, and L4Re and Emscripten. They report IPv6 packet info only, with
//! no IPv4 destination and no receive stamps. The libc calls themselves are
//! in `sys::libc_msg`.

use super::*;
use std::os::fd::AsFd;

/// Ask for IPv6 packet info; IPv4 destination metadata is unsupported here.
pub(super) fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    match version {
        IpVersion::V4 => Err(unsupported_ipv4()),
        IpVersion::V6 => crate::sys::libc_msg::set_int_option(
            udp_socket.as_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_RECVPKTINFO,
            1,
        ),
    }
}

fn unsupported_ipv4() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    )
}

/// One datagram from `udp_socket` into `buf`: its length, its sender and
/// what its control messages say about it, through a direct `recvmsg`.
pub(super) fn recv(
    udp_socket: &UdpSocket,
    buf: &mut [u8],
    version: IpVersion,
) -> io::Result<(usize, SocketAddr, PacketInfo)> {
    if matches!(version, IpVersion::V4) {
        return Err(unsupported_ipv4());
    }
    let mut control = ControlBuffer::new();
    let (len, peer, destination, index) =
        crate::sys::libc_msg::recv_ipv6_pktinfo(udp_socket.as_fd(), buf, &mut control)?;
    let info = PacketInfo {
        destination: IpAddr::V6(destination),
        arrival_index: Some(index),
        arrival_stamp: None,
    };
    Ok((len, peer, info))
}
