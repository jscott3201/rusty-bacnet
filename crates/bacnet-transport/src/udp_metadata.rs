//! Destination-aware UDP receive support for BACnet/IP transports.
#![allow(unsafe_code)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

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

#[derive(Clone, Copy)]
pub(crate) enum IpVersion {
    V4,
    #[cfg_attr(not(feature = "ipv6"), allow(dead_code))]
    V6,
}

#[derive(Debug)]
pub(crate) struct ReceivedDatagram {
    pub len: usize,
    pub peer: SocketAddr,
    pub destination: IpAddr,
    pub os_group_delivery: Option<bool>,
}

pub(crate) struct DestinationReceiver {
    version: IpVersion,
    #[cfg(windows)]
    recv_msg: windows_sys::Win32::Networking::WinSock::LPFN_WSARECVMSG,
}

impl DestinationReceiver {
    pub fn configure(socket: &Socket, version: IpVersion) -> io::Result<Self> {
        configure_packet_info(socket, version)?;
        Ok(Self {
            version,
            #[cfg(windows)]
            recv_msg: windows_recv_msg(socket)?,
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

    supported_unix_item! {
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::mem::{size_of, zeroed};
        use std::os::fd::AsRawFd;

        let mut peer_storage: libc::sockaddr_storage = unsafe { zeroed() };
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let mut control = [0usize; 16];
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_name = (&mut peer_storage as *mut libc::sockaddr_storage).cast();
        message.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = size_of::<[usize; 16]>() as _;

        let received = unsafe { libc::recvmsg(udp_socket.as_raw_fd(), &mut message, 0) };
        if received < 0 {
            return Err(io::Error::last_os_error());
        }
        if message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
            return Err(invalid_metadata("truncated UDP payload or packet metadata"));
        }

        let peer = unsafe { unix_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
        let destination = unsafe { unix_destination(&message, self.version) }?;
        Ok(ReceivedDatagram {
            len: received as usize,
            peer,
            destination,
            os_group_delivery: None,
        })
    }
    }

    #[cfg(windows)]
    fn try_recv_from(
        &self,
        udp_socket: &UdpSocket,
        buf: &mut [u8],
    ) -> io::Result<ReceivedDatagram> {
        use std::mem::{size_of, zeroed};
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::*;

        let recv_msg = self
            .recv_msg
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))?;
        let mut peer_storage: SOCKADDR_STORAGE = unsafe { zeroed() };
        let mut data = WSABUF {
            len: u32::try_from(buf.len()).unwrap_or(u32::MAX),
            buf: buf.as_mut_ptr(),
        };
        let mut control = [0usize; 16];
        let mut message = WSAMSG {
            name: (&mut peer_storage as *mut SOCKADDR_STORAGE).cast(),
            namelen: size_of::<SOCKADDR_STORAGE>() as i32,
            lpBuffers: &mut data,
            dwBufferCount: 1,
            Control: WSABUF {
                len: size_of::<[usize; 16]>() as u32,
                buf: control.as_mut_ptr().cast(),
            },
            dwFlags: 0,
        };
        let mut received = 0u32;
        let result = unsafe {
            recv_msg(
                udp_socket.as_raw_socket() as SOCKET,
                &mut message,
                &mut received,
                std::ptr::null_mut(),
                None,
            )
        };
        if result == SOCKET_ERROR {
            return Err(classify_recv_error(
                unsafe { WSAGetLastError() },
                &[WSAEMSGSIZE, WSAECONNRESET, WSAENETRESET],
            ));
        }
        if message.dwFlags & (MSG_TRUNC | MSG_CTRUNC) != 0 {
            return Err(invalid_metadata("truncated UDP payload or packet metadata"));
        }

        let peer = unsafe { windows_socket_addr(&peer_storage) }
            .ok_or_else(|| invalid_metadata("missing or invalid UDP peer address"))?;
        let destination = unsafe {
            windows_destination(
                std::slice::from_raw_parts(
                    control.as_ptr().cast::<u8>(),
                    message.Control.len as usize,
                ),
                self.version,
            )
        }?;
        Ok(ReceivedDatagram {
            len: received as usize,
            peer,
            destination,
            os_group_delivery: Some(message.dwFlags & (MSG_BCAST | MSG_MCAST) != 0),
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

fn invalid_metadata(message: &'static str) -> io::Error {
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
    use std::mem::size_of;
    use std::os::fd::AsRawFd;

    let enabled: libc::c_int = 1;
    let (level, option) = match version {
        IpVersion::V4 => ipv4_packet_info_option()?,
        IpVersion::V6 => (libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO),
    };
    let result = unsafe {
        libc::setsockopt(
            udp_socket.as_raw_fd(),
            level,
            option,
            (&enabled as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Ok((libc::IPPROTO_IP, libc::IP_PKTINFO))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
supported_unix_item! {
unsafe fn unix_ipv4_destination_cmsg(_header: &libc::cmsghdr) -> io::Result<Option<Ipv4Addr>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    ))
}
}

#[cfg(all(
    unix,
    any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )
))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Ok((libc::IPPROTO_IP, libc::IP_RECVDSTADDR))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn ipv4_packet_info_option() -> io::Result<(libc::c_int, libc::c_int)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPv4 destination metadata is unsupported on this platform",
    ))
}

supported_unix_item! {
unsafe fn unix_socket_addr(storage: &libc::sockaddr_storage) -> Option<SocketAddr> {
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            let address = unsafe { &*(storage as *const _ as *const libc::sockaddr_in) };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()),
                u16::from_be(address.sin_port),
            )))
        }
        libc::AF_INET6 => {
            let address = unsafe { &*(storage as *const _ as *const libc::sockaddr_in6) };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(address.sin6_addr.s6_addr),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                address.sin6_scope_id,
            )))
        }
        _ => None,
    }
}
}

supported_unix_item! {
unsafe fn unix_destination(message: &libc::msghdr, version: IpVersion) -> io::Result<IpAddr> {
    let mut destination = None;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        let current = unsafe { &*header };
        let candidate = unsafe { unix_destination_cmsg(current, version) }?;
        if let Some(candidate) = candidate {
            if destination.replace(candidate).is_some() {
                return Err(invalid_metadata("duplicate UDP destination metadata"));
            }
        }
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))
}
}

supported_unix_item! {
unsafe fn unix_destination_cmsg(
    header: &libc::cmsghdr,
    version: IpVersion,
) -> io::Result<Option<IpAddr>> {
    match version {
        IpVersion::V4 => unsafe {
            unix_ipv4_destination_cmsg(header).map(|address| address.map(IpAddr::V4))
        },
        IpVersion::V6
            if header.cmsg_level == libc::IPPROTO_IPV6
                && header.cmsg_type == libc::IPV6_PKTINFO =>
        {
            if !unix_cmsg_has_payload::<libc::in6_pktinfo>(header) {
                return Err(invalid_metadata("short IPv6 destination metadata"));
            }
            let info = unsafe { &*(libc::CMSG_DATA(header) as *const libc::in6_pktinfo) };
            Ok(Some(IpAddr::V6(Ipv6Addr::from(info.ipi6_addr.s6_addr))))
        }
        _ => Ok(None),
    }
}
}

supported_unix_item! {
fn unix_cmsg_has_payload<T>(header: &libc::cmsghdr) -> bool {
    let required = unsafe { libc::CMSG_LEN(std::mem::size_of::<T>() as _) } as usize;
    header.cmsg_len as usize >= required
}
}

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<Ipv4Addr>> {
    if header.cmsg_level != libc::IPPROTO_IP || header.cmsg_type != libc::IP_PKTINFO {
        return Ok(None);
    }
    if !unix_cmsg_has_payload::<libc::in_pktinfo>(header) {
        return Err(invalid_metadata("short IPv4 destination metadata"));
    }
    let info = unsafe { &*(libc::CMSG_DATA(header) as *const libc::in_pktinfo) };
    Ok(Some(Ipv4Addr::from(info.ipi_addr.s_addr.to_ne_bytes())))
}

#[cfg(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
unsafe fn unix_ipv4_destination_cmsg(header: &libc::cmsghdr) -> io::Result<Option<Ipv4Addr>> {
    if header.cmsg_level != libc::IPPROTO_IP || header.cmsg_type != libc::IP_RECVDSTADDR {
        return Ok(None);
    }
    if !unix_cmsg_has_payload::<libc::in_addr>(header) {
        return Err(invalid_metadata("short IPv4 destination metadata"));
    }
    let address = unsafe { &*(libc::CMSG_DATA(header) as *const libc::in_addr) };
    Ok(Some(Ipv4Addr::from(address.s_addr.to_ne_bytes())))
}

#[cfg(windows)]
fn configure_packet_info(udp_socket: &Socket, version: IpVersion) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let enabled = 1u32;
    let (level, option) = match version {
        IpVersion::V4 => (IPPROTO_IP, IP_PKTINFO),
        IpVersion::V6 => (IPPROTO_IPV6, IPV6_PKTINFO),
    };
    let result = unsafe {
        setsockopt(
            udp_socket.as_raw_socket() as SOCKET,
            level,
            option,
            (&enabled as *const u32).cast(),
            size_of::<u32>() as i32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
    }
}

unsupported_udp_metadata_item! {
fn configure_packet_info(_udp_socket: &Socket, _version: IpVersion) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "UDP destination metadata is unsupported on this platform",
    ))
}
}

#[cfg(windows)]
fn windows_recv_msg(
    udp_socket: &Socket,
) -> io::Result<windows_sys::Win32::Networking::WinSock::LPFN_WSARECVMSG> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    let mut function: LPFN_WSARECVMSG = None;
    let mut returned = 0u32;
    let result = unsafe {
        WSAIoctl(
            udp_socket.as_raw_socket() as SOCKET,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            (&WSAID_WSARECVMSG as *const windows_sys::core::GUID).cast(),
            size_of::<windows_sys::core::GUID>() as u32,
            (&mut function as *mut LPFN_WSARECVMSG).cast(),
            size_of::<LPFN_WSARECVMSG>() as u32,
            &mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    function
        .map(Some)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))
}

#[cfg(windows)]
unsafe fn windows_socket_addr(
    storage: &windows_sys::Win32::Networking::WinSock::SOCKADDR_STORAGE,
) -> Option<SocketAddr> {
    use windows_sys::Win32::Networking::WinSock::*;

    match storage.ss_family {
        AF_INET => {
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN) };
            let bytes = unsafe { address.sin_addr.S_un.S_addr.to_ne_bytes() };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(bytes),
                u16::from_be(address.sin_port),
            )))
        }
        AF_INET6 => {
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN6) };
            let bytes = unsafe { address.sin6_addr.u.Byte };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(bytes),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                unsafe { address.Anonymous.sin6_scope_id },
            )))
        }
        _ => None,
    }
}

#[cfg(windows)]
unsafe fn windows_destination(control: &[u8], version: IpVersion) -> io::Result<IpAddr> {
    use std::mem::size_of;
    use windows_sys::Win32::Networking::WinSock::*;

    let align = |length: usize| (length + size_of::<usize>() - 1) & !(size_of::<usize>() - 1);
    let data_offset = align(size_of::<CMSGHDR>());
    let mut offset = 0usize;
    let mut destination = None;
    while offset + size_of::<CMSGHDR>() <= control.len() {
        let header: CMSGHDR =
            unsafe { std::ptr::read_unaligned(control.as_ptr().add(offset).cast()) };
        if header.cmsg_len < data_offset || offset + header.cmsg_len > control.len() {
            return Err(invalid_metadata("invalid Windows UDP control message"));
        }
        let data = unsafe { control.as_ptr().add(offset + data_offset) };
        let candidate = match version {
            IpVersion::V4
                if header.cmsg_level == IPPROTO_IP
                    && header.cmsg_type == IP_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN_PKTINFO>() =>
            {
                let info: IN_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                let bytes = unsafe { info.ipi_addr.S_un.S_addr.to_ne_bytes() };
                Some(IpAddr::V4(Ipv4Addr::from(bytes)))
            }
            IpVersion::V6
                if header.cmsg_level == IPPROTO_IPV6
                    && header.cmsg_type == IPV6_PKTINFO
                    && header.cmsg_len >= data_offset + size_of::<IN6_PKTINFO>() =>
            {
                let info: IN6_PKTINFO = unsafe { std::ptr::read_unaligned(data.cast()) };
                Some(IpAddr::V6(Ipv6Addr::from(unsafe { info.ipi6_addr.u.Byte })))
            }
            _ => None,
        };
        if let Some(candidate) = candidate {
            if destination.replace(candidate).is_some() {
                return Err(invalid_metadata("duplicate UDP destination metadata"));
            }
        }
        offset = offset.saturating_add(align(header.cmsg_len));
    }
    destination.ok_or_else(|| invalid_metadata("missing UDP destination metadata"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ipv4_loopback_destination_is_reported() {
        let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
        socket.set_nonblocking(true).unwrap();
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
        socket
            .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
            .unwrap();
        let socket = UdpSocket::from_std(socket.into()).unwrap();
        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        sender
            .send_to(b"v4", socket.local_addr().unwrap())
            .await
            .unwrap();

        let mut buf = [0u8; 8];
        let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
        assert_eq!(&buf[..received.len], b"v4");
        assert_eq!(received.destination, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn ipv6_loopback_destination_is_reported() {
        let socket = Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None).unwrap();
        socket.set_only_v6(true).unwrap();
        socket.set_nonblocking(true).unwrap();
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V6).unwrap();
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
        let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
        assert_eq!(&buf[..received.len], b"v6");
        assert_eq!(received.destination, IpAddr::V6(Ipv6Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn oversized_datagram_is_dropped_without_stopping_receive() {
        let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
        socket.set_nonblocking(true).unwrap();
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V4).unwrap();
        socket
            .bind(&SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into())
            .unwrap();
        let socket = UdpSocket::from_std(socket.into()).unwrap();
        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let destination = socket.local_addr().unwrap();

        sender.send_to(&[0xAA; 64], destination).await.unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(
            receiver
                .recv_from(&socket, &mut buf)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        sender.send_to(b"ok", destination).await.unwrap();
        let received = receiver.recv_from(&socket, &mut buf).await.unwrap();
        assert_eq!(&buf[..received.len], b"ok");
    }

    #[test]
    fn windows_recoverable_receive_errors_are_droppable_invalid_data() {
        assert_eq!(
            classify_recv_error(10040, &[10040, 10054, 10052]).kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            classify_recv_error(10054, &[10040, 10054, 10052]).kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            classify_recv_error(10052, &[10040, 10054, 10052]).kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            classify_recv_error(12345, &[10040, 10054, 10052]).raw_os_error(),
            Some(12345)
        );
    }
}
