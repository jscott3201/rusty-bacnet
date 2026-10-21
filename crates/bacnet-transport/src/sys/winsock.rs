//! Winsock: integer socket options, and the `WSARecvMsg` and `WSASendMsg`
//! extension functions B/IP and B/IPv6 receive and send packet info with.
//!
//! An extension function comes from `WSAIoctl` on a socket, and only
//! `RecvMsg::lookup` and `SendMsg::lookup` (with `ipv6`) make one. Each keeps
//! the raw handle it was looked up on, and refuses with `InvalidInput` a call
//! on any other, so the function a call runs is always the one Winsock
//! returned for that socket. The transports look it up on a `socket2::Socket`
//! and use it on the tokio socket made from it, which keeps the handle.

use std::io;
use std::mem::{size_of, zeroed};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::windows::io::{AsRawSocket, BorrowedSocket, RawSocket};

use windows_sys::Win32::Networking::WinSock::*;

use crate::sys::ControlBuffer;

/// This thread's last Winsock error.
fn last_error() -> io::Error {
    // SAFETY: WSAGetLastError takes no arguments and only reads this
    // thread's last Winsock error code.
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

fn raw(socket: BorrowedSocket<'_>) -> SOCKET {
    socket.as_raw_socket() as SOCKET
}

/// Set the integer (or BOOL) socket option `option` at `level` on `socket`
/// to `value`.
pub(crate) fn set_int_option(
    socket: BorrowedSocket<'_>,
    level: i32,
    option: i32,
    value: i32,
) -> io::Result<()> {
    // SAFETY: the socket handle is open for the call's duration, the option
    // value points to `value`, a live i32, and the length passed is exactly
    // an i32's; setsockopt only reads it.
    let result = unsafe {
        setsockopt(
            raw(socket),
            level,
            option,
            (&value as *const i32).cast(),
            size_of::<i32>() as i32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(last_error())
    }
}

/// `WSARecvMsg`, as `WSAIoctl` returned it for `socket`.
pub(crate) struct RecvMsg {
    function: LPFN_WSARECVMSG,
    socket: RawSocket,
}

/// What one [`RecvMsg::recv`] got.
pub(crate) struct WsaReceived {
    /// The datagram's length in the buffer.
    pub(crate) len: usize,
    /// The message flags Winsock set (`MSG_TRUNC`, `MSG_BCAST` and so on).
    pub(crate) flags: u32,
    /// How many octets of the control buffer the control messages fill.
    pub(crate) control_len: usize,
    /// The sender, when its address is IPv4 or IPv6.
    pub(crate) peer: Option<SocketAddr>,
}

impl RecvMsg {
    /// Look `WSARecvMsg` up on `socket`; `Unsupported` when Winsock has none.
    pub(crate) fn lookup(socket: BorrowedSocket<'_>) -> io::Result<Self> {
        let mut function: LPFN_WSARECVMSG = None;
        let mut returned = 0u32;
        // SAFETY: the input points to the WSARecvMsg GUID with its exact size,
        // and the output to `function`, a live LPFN_WSARECVMSG (an
        // Option<fn>, which is pointer-sized and null for None) with its exact
        // size; null OVERLAPPED and callback make the call synchronous.
        let result = unsafe {
            WSAIoctl(
                raw(socket),
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
            return Err(last_error());
        }
        function
            .map(|function| Self {
                function: Some(function),
                socket: socket.as_raw_socket(),
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))
    }

    /// One synchronous `WSARecvMsg` on `socket` into `buf`, with `control`
    /// for the control messages. A Winsock failure is its OS error, which
    /// the caller classifies; `socket` must be the one this was looked up on
    /// (`InvalidInput` otherwise).
    pub(crate) fn recv(
        &self,
        socket: BorrowedSocket<'_>,
        buf: &mut [u8],
        control: &mut ControlBuffer,
    ) -> io::Result<WsaReceived> {
        super::same_socket(self.socket, socket.as_raw_socket())?;
        let recv_msg = self
            .function
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSARecvMsg unavailable"))?;
        // SAFETY: SOCKADDR_STORAGE is a plain C struct of integers, so all
        // zeros is a valid value.
        let mut peer_storage: SOCKADDR_STORAGE = unsafe { zeroed() };
        let mut data = WSABUF {
            len: u32::try_from(buf.len()).unwrap_or(u32::MAX),
            buf: buf.as_mut_ptr(),
        };
        let control = control.bytes_mut();
        let mut message = WSAMSG {
            name: (&mut peer_storage as *mut SOCKADDR_STORAGE).cast(),
            namelen: size_of::<SOCKADDR_STORAGE>() as i32,
            lpBuffers: &mut data,
            dwBufferCount: 1,
            Control: WSABUF {
                len: control.len() as u32,
                buf: control.as_mut_ptr(),
            },
            dwFlags: 0,
        };
        let mut received = 0u32;
        // SAFETY: `recv_msg` is the WSARecvMsg that WSAIoctl returned for
        // this very socket handle: only `lookup` makes a RecvMsg, and the
        // check above refused any other handle. Every pointer in `message` points to live
        // storage of the length given beside it: `peer_storage` (namelen),
        // `data` over `buf` (len, clamped to u32) and `control` (Control.len).
        // Null OVERLAPPED and completion routine make the call synchronous, so
        // no pointer outlives it.
        let result = unsafe {
            recv_msg(
                raw(socket),
                &mut message,
                &mut received,
                std::ptr::null_mut(),
                None,
            )
        };
        if result == SOCKET_ERROR {
            return Err(last_error());
        }
        Ok(WsaReceived {
            len: received as usize,
            flags: message.dwFlags,
            control_len: message.Control.len as usize,
            // SAFETY: `peer_storage` is an initialized SOCKADDR_STORAGE,
            // which is all windows_socket_addr asks.
            peer: unsafe { windows_socket_addr(&peer_storage) },
        })
    }
}

/// The IPv4 or IPv6 socket address in `storage`, by its family.
///
/// # Safety
///
/// Any initialized `SOCKADDR_STORAGE` is sound: it is larger than and
/// aligned for both `SOCKADDR_IN` and `SOCKADDR_IN6`, whose fields and
/// unions are all integers, so reading either over it is defined whatever
/// the bytes. The fn is unsafe for the casts alone.
unsafe fn windows_socket_addr(storage: &SOCKADDR_STORAGE) -> Option<SocketAddr> {
    match storage.ss_family {
        AF_INET => {
            // SAFETY: SOCKADDR_STORAGE is at least as large and as aligned as
            // SOCKADDR_IN, it is initialized, and SOCKADDR_IN is integers,
            // valid for any bytes. The family says it holds one.
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN) };
            // SAFETY: every variant of the IN_ADDR union is integers over the
            // same four initialized octets, so reading S_addr is defined.
            let bytes = unsafe { address.sin_addr.S_un.S_addr.to_ne_bytes() };
            Some(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::from(bytes),
                u16::from_be(address.sin_port),
            )))
        }
        AF_INET6 => {
            // SAFETY: as above, for SOCKADDR_IN6, which SOCKADDR_STORAGE is
            // also large and aligned enough to hold.
            let address = unsafe { &*(storage as *const _ as *const SOCKADDR_IN6) };
            // SAFETY: both IN6_ADDR union variants are integer arrays over the
            // same sixteen initialized octets.
            let bytes = unsafe { address.sin6_addr.u.Byte };
            Some(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(bytes),
                u16::from_be(address.sin6_port),
                address.sin6_flowinfo,
                // SAFETY: both variants of this union (a u32 and the
                // SCOPE_ID struct over it) are integers of the same size.
                unsafe { address.Anonymous.sin6_scope_id },
            )))
        }
        _ => None,
    }
}

/// `WSASendMsg`, as `WSAIoctl` returned it for `socket`.
#[cfg(feature = "ipv6")]
pub(crate) struct SendMsg {
    function: LPFN_WSASENDMSG,
    socket: RawSocket,
}

#[cfg(feature = "ipv6")]
impl SendMsg {
    /// Look `WSASendMsg` up on `socket`; `Unsupported` when Winsock has none.
    pub(crate) fn lookup(socket: BorrowedSocket<'_>) -> io::Result<Self> {
        let mut function: LPFN_WSASENDMSG = None;
        let mut returned = 0;
        // SAFETY: synchronous extension lookup writes only the function-pointer
        // storage of the declared size. This socket owns the returned provider API.
        let result = unsafe {
            WSAIoctl(
                raw(socket),
                SIO_GET_EXTENSION_FUNCTION_POINTER,
                (&WSAID_WSASENDMSG as *const windows_sys::core::GUID).cast(),
                size_of::<windows_sys::core::GUID>() as u32,
                (&mut function as *mut LPFN_WSASENDMSG).cast(),
                size_of::<LPFN_WSASENDMSG>() as u32,
                &mut returned,
                std::ptr::null_mut(),
                None,
            )
        };
        if result != 0 {
            return Err(last_error());
        }
        function
            .map(|function| Self {
                function: Some(function),
                socket: socket.as_raw_socket(),
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSASendMsg unavailable"))
    }

    /// One synchronous `WSASendMsg` of `bytes` to `peer` on `socket`, with
    /// `source` and the interface `index` in a hand-built `IN6_PKTINFO`
    /// control message. `socket` must be the one this was looked up on
    /// (`InvalidInput` otherwise).
    pub(crate) fn send_ipv6_pktinfo(
        &self,
        socket: BorrowedSocket<'_>,
        bytes: &[u8],
        peer: SocketAddrV6,
        source: Ipv6Addr,
        index: u32,
    ) -> io::Result<usize> {
        super::same_socket(self.socket, socket.as_raw_socket())?;
        let function = self
            .function
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "WSASendMsg unavailable"))?;
        let destination = socket2::SockAddr::from(peer);
        let mut control = [0usize; 16];
        let align = |len: usize| len.next_multiple_of(size_of::<usize>());
        let offset = align(size_of::<CMSGHDR>());
        let length = offset + size_of::<IN6_PKTINFO>();
        let space = align(length);
        assert!(space <= size_of_val(&control));
        let mut info = IN6_PKTINFO::default();
        info.ipi6_addr.u.Byte = source.octets();
        info.ipi6_ifindex = index;
        // SAFETY: `control` is a usize array, so it is aligned for CMSGHDR, and
        // for IN6_PKTINFO at `offset`, which is rounded up to usize alignment.
        // The assert keeps `space` (header, padding and payload) inside it, so
        // both writes are in bounds; cmsg_len covers the whole payload.
        unsafe {
            std::ptr::write(
                control.as_mut_ptr().cast::<CMSGHDR>(),
                CMSGHDR {
                    cmsg_len: length,
                    cmsg_level: IPPROTO_IPV6,
                    cmsg_type: IPV6_PKTINFO,
                },
            );
            std::ptr::write(
                control
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<IN6_PKTINFO>(),
                info,
            );
        }
        let mut data = WSABUF {
            len: u32::try_from(bytes.len())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "datagram too large"))?,
            buf: bytes.as_ptr().cast_mut(),
        };
        let message = WSAMSG {
            name: destination.as_ptr().cast_mut().cast(),
            namelen: destination.len(),
            lpBuffers: &mut data,
            dwBufferCount: 1,
            Control: WSABUF {
                len: space as u32,
                buf: control.as_mut_ptr().cast(),
            },
            dwFlags: 0,
        };
        let mut sent = 0;
        // SAFETY: `function` is the WSASendMsg that WSAIoctl returned for this
        // very socket handle: only `lookup` makes a SendMsg, and the check
        // above refused any other handle. Null OVERLAPPED and callback request a
        // synchronous attempt, including on an overlapped-created socket. All
        // message/payload pointers remain live until it returns; WouldBlock is
        // retried by Tokio readiness.
        let result = unsafe {
            function(
                raw(socket),
                &message,
                0,
                &mut sent,
                std::ptr::null_mut(),
                None,
            )
        };
        if result == SOCKET_ERROR {
            Err(last_error())
        } else {
            Ok(sent as usize)
        }
    }
}
