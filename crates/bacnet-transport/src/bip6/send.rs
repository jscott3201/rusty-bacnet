//! A synchronous packet-info syscall attempt inside Tokio writable readiness.
//!
//! No native pointer or pending overlapped operation survives a syscall attempt.
//! Most Unix targets send through nix's safe `sendmsg` (#1882); the libc and
//! Winsock paths are FFI, in `crate::sys`.

use super::link::SelectedLink;
use socket2::Socket;
use std::{io, net::SocketAddrV6};
use tokio::{io::Interest, net::UdpSocket};

pub(super) struct SelectedSender {
    #[cfg(windows)]
    function: crate::sys::winsock::SendMsg,
}

impl SelectedSender {
    pub fn new(socket: &Socket) -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                function: crate::sys::winsock::SendMsg::lookup(
                    std::os::windows::io::AsSocket::as_socket(socket),
                )?,
            })
        }
        #[cfg(unix)]
        {
            let _ = socket;
            Ok(Self {})
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = socket;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "IPv6 source selection unsupported",
            ))
        }
    }

    pub async fn send(
        &self,
        socket: &UdpSocket,
        bytes: &[u8],
        mut peer: SocketAddrV6,
        link: SelectedLink,
    ) -> io::Result<usize> {
        if peer.ip().is_unicast_link_local() || peer.ip().is_multicast() {
            peer.set_scope_id(link.index);
        }
        let count = socket
            .async_io(Interest::WRITABLE, || {
                self.try_send(socket, bytes, peer, link)
            })
            .await?;
        if count != bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "partial IPv6 datagram send",
            ));
        }
        Ok(count)
    }

    /// One direct `sendmsg` with the selected source in IPv6 packet info,
    /// through nix (#1882).
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    fn try_send(
        &self,
        socket: &UdpSocket,
        bytes: &[u8],
        peer: SocketAddrV6,
        link: SelectedLink,
    ) -> io::Result<usize> {
        use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags, SockaddrIn6};
        use std::{io::IoSlice, os::fd::AsRawFd};
        let info = libc::in6_pktinfo {
            ipi6_addr: libc::in6_addr {
                s6_addr: link.address.octets(),
            },
            ipi6_ifindex: link.index,
        };
        Ok(sendmsg(
            socket.as_raw_fd(),
            &[IoSlice::new(bytes)],
            &[ControlMessage::Ipv6PacketInfo(&info)],
            MsgFlags::empty(),
            Some(&SockaddrIn6::from(peer)),
        )?)
    }

    /// The same on libc, where nix can't send IPv6 packet info: OpenBSD,
    /// DragonFly and the other Unix targets.
    #[cfg(all(
        unix,
        not(any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
        ))
    ))]
    fn try_send(
        &self,
        socket: &UdpSocket,
        bytes: &[u8],
        peer: SocketAddrV6,
        link: SelectedLink,
    ) -> io::Result<usize> {
        use std::os::fd::AsFd;
        crate::sys::libc_msg::send_ipv6_pktinfo(
            socket.as_fd(),
            bytes,
            peer,
            link.address,
            link.index,
        )
    }

    /// The same through `WSASendMsg`, with a hand-built `IN6_PKTINFO`.
    #[cfg(windows)]
    fn try_send(
        &self,
        udp_socket: &UdpSocket,
        bytes: &[u8],
        peer: SocketAddrV6,
        link: SelectedLink,
    ) -> io::Result<usize> {
        use std::os::windows::io::AsSocket;
        self.function.send_ipv6_pktinfo(
            udp_socket.as_socket(),
            bytes,
            peer,
            link.address,
            link.index,
        )
    }

    #[cfg(not(any(unix, windows)))]
    fn try_send(
        &self,
        _: &UdpSocket,
        _: &[u8],
        _: SocketAddrV6,
        _: SelectedLink,
    ) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "IPv6 source selection unsupported",
        ))
    }
}
