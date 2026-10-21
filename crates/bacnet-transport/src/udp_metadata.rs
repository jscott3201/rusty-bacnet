//! Destination-aware UDP receive support for BACnet/IP transports.
//!
//! Most Unix targets receive through nix's safe `recvmsg` and socket options
//! (`udp_metadata_nix.rs`, #1882); the few nix doesn't cover keep the libc
//! code (`udp_metadata_libc.rs`). Windows goes through Winsock. The FFI those
//! paths still need lives in `crate::sys`.

use std::io;
use std::net::{IpAddr, SocketAddr};
// The nix and Winsock parsers and the tests build these.
#[cfg_attr(
    not(any(
        windows,
        test,
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    )),
    allow(unused_imports)
)]
use std::net::{Ipv4Addr, Ipv6Addr};
// The tests build these.
#[cfg(test)]
use std::net::{SocketAddrV4, SocketAddrV6};

use socket2::Socket;
use tokio::io::Interest;
use tokio::net::UdpSocket;

macro_rules! supported_unix_item {
    ($item:item) => {
        #[cfg(any(
            target_os = "linux",
            target_os = "l4re",
            target_os = "android",
            target_os = "emscripten",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "nto",
        ))]
        $item
    };
}

macro_rules! unsupported_udp_metadata_item {
    ($item:item) => {
        #[cfg(not(any(
            windows,
            target_os = "linux",
            target_os = "l4re",
            target_os = "android",
            target_os = "emscripten",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "nto",
        )))]
        $item
    };
}

#[path = "udp_metadata_stamps.rs"]
mod stamps;
pub(crate) use stamps::request_arrival_stamps;

/// The Unix receive path on nix (#1882), where nix sets every socket option
/// it needs and decodes or hands over its control messages.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[path = "udp_metadata_nix.rs"]
mod unix;

/// The Unix receive path on libc, where nix has no IPv6 packet-info socket
/// option.
#[cfg(any(
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
#[path = "udp_metadata_libc.rs"]
mod unix;

// The control buffer's alignment is part of the libc walk's safety
// argument, so it lives with the FFI.
#[cfg(any(unix, windows))]
use crate::sys::ControlBuffer;

#[derive(Clone, Copy)]
pub(crate) enum IpVersion {
    V4,
    #[cfg_attr(not(feature = "ipv6"), allow(dead_code))]
    V6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceivedDatagram {
    pub len: usize,
    pub peer: SocketAddr,
    pub destination: IpAddr,
    /// The index of the interface the datagram arrived on, where the OS
    /// reports it: IPv6 packet info everywhere, and for IPv4 the packet info
    /// on Linux and Windows and `IP_RECVIF` on macOS and the BSDs. It is
    /// reported as given: zero names no interface, and consumers treat it so.
    pub arrival_index: Option<u32>,
    pub os_group_delivery: Option<bool>,
    /// When the kernel received the datagram, where
    /// [`request_arrival_stamps`] was set on its socket and the OS reports
    /// it. Its unit depends on the OS, so it only orders datagrams that
    /// sockets of one host received (#1546).
    pub arrival_stamp: Option<u128>,
}

supported_unix_item! {
/// What a datagram's control messages say about it, taken together.
#[derive(Debug, PartialEq, Eq)]
struct PacketInfo {
    destination: IpAddr,
    arrival_index: Option<u32>,
    arrival_stamp: Option<u128>,
}
}

pub(crate) struct DestinationReceiver {
    version: IpVersion,
    #[cfg(windows)]
    recv_msg: crate::sys::winsock::RecvMsg,
}

impl DestinationReceiver {
    pub fn configure(socket: &Socket, version: IpVersion) -> io::Result<Self> {
        configure_packet_info(socket, version)?;
        Ok(Self {
            version,
            #[cfg(windows)]
            recv_msg: crate::sys::winsock::RecvMsg::lookup(
                std::os::windows::io::AsSocket::as_socket(socket),
            )?,
        })
    }

    pub async fn recv_from(
        &self,
        socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        socket
            .async_io(Interest::READABLE, || self.try_recv_from(socket, buf))
            .await
    }

    /// The datagram already queued on `socket`, or `WouldBlock`, without
    /// waiting. On Unix this asks the kernel directly rather than tokio's
    /// cached readiness, which can lag behind a datagram the kernel already
    /// holds; `WouldBlock` leaves that readiness as it was, so a later
    /// [`recv_from`](Self::recv_from) still waits correctly. Windows goes
    /// through tokio, which re-arms its poll there on `WouldBlock`.
    pub fn recv_queued(&self, socket: &UdpSocket, buf: &mut [u8]) -> io::Result<ReceivedDatagram> {
        #[cfg(windows)]
        return socket.try_io(Interest::READABLE, || self.try_recv_from(socket, buf));
        #[cfg(not(windows))]
        self.try_recv_from(socket, buf)
    }

    supported_unix_item! {
    /// One direct `recvmsg` on `udp_socket`, never through tokio's readiness.
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        let (len, peer, info) = unix::recv(udp_socket, buf, self.version)?;
        Ok(ReceivedDatagram {
            len,
            peer,
            destination: info.destination,
            arrival_index: info.arrival_index,
            os_group_delivery: None,
            arrival_stamp: info.arrival_stamp,
        })
    }
    }

    #[cfg(windows)]
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::os::windows::io::AsSocket;
        use windows_sys::Win32::Networking::WinSock::*;

        let mut control = ControlBuffer::new();
        let received = self
            .recv_msg
            .recv(udp_socket.as_socket(), buf, &mut control)
            .map_err(|error| match error.raw_os_error() {
                Some(code) => {
                    classify_recv_error(code, &[WSAEMSGSIZE, WSAECONNRESET, WSAENETRESET])
                }
                None => error,
            })?;
        if received.flags & (MSG_TRUNC | MSG_CTRUNC) != 0 {
            return Err(invalid_metadata("truncated UDP payload or packet metadata"));
        }

        let control = control
            .bytes()
            .get(..received.control_len)
            .ok_or_else(|| invalid_metadata("oversized Windows UDP control buffer"))?;
        let peer = received
            .peer
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
        let (destination, arrival_index) = windows_destination(control, self.version)?;
        Ok(ReceivedDatagram {
            len: received.len,
            peer,
            destination,
            arrival_index,
            os_group_delivery: Some(received.flags & (MSG_BCAST | MSG_MCAST) != 0),
            arrival_stamp: None,
        })
    }

    unsupported_udp_metadata_item! {
    fn try_recv_from(
        &self,
        _udp_socket: &UdpSocket,
        _buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "UDP destination metadata is unsupported on this platform",
        ))
    }
    }
}

pub(crate) fn invalid_metadata(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn classify_recv_error(code: i32, droppable_codes: &[i32]) -> io::Error {
    if droppable_codes.contains(&code) {
        invalid_metadata("droppable UDP receive error")
    } else {
        io::Error::from_raw_os_error(code)
    }
}

supported_unix_item! {
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    unix::configure_packet_info(udp_socket, version)
}
}

#[cfg(windows)]
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    use std::os::windows::io::AsSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let (level, option) = match version {
        IpVersion::V4 => (IPPROTO_IP, IP_PKTINFO),
        IpVersion::V6 => (IPPROTO_IPV6, IPV6_PKTINFO),
    };
    crate::sys::winsock::set_int_option(udp_socket.as_socket(), level, option, 1)
}

unsupported_udp_metadata_item! {
fn configure_packet_info(_udp_socket: &Socket, _version: IpVersion) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "UDP destination metadata is unsupported on this platform",
    ))
}
}

/// The `N` octets of `bytes` from `offset`, if they are all there.
#[cfg(any(
    windows,
    test,
    target_vendor = "apple",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
))]
fn octets_at<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}

/// The Winsock control-message parser, built on every host for its tests.
#[cfg(any(windows, test))]
#[path = "udp_metadata_winsock.rs"]
mod winsock;
#[cfg(windows)]
use winsock::windows_destination;

#[cfg(test)]
#[path = "udp_metadata_tests.rs"]
mod tests;

#[cfg(all(
    test,
    any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    )
))]
#[path = "udp_metadata_ipv6_tests.rs"]
mod ipv6_tests;
