//! libc socket calls for what nix doesn't cover (#1882): an integer socket
//! option nix has no setter for, the IPv6 packet-info receive on illumos,
//! Solaris, NTO, L4Re and Emscripten, and the IPv6 packet-info send on the
//! Unix targets where nix can't send one (OpenBSD, DragonFly and others).

/// Set the integer socket option `option` at `level` on `fd` to `value`, for
/// the options nix has no safe setter for: Apple's `SO_TIMESTAMP_MONOTONIC`,
/// and `IPV6_RECVPKTINFO` on the targets that receive through libc.
#[cfg(any(
    target_vendor = "apple",
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
pub(crate) fn set_int_option(
    fd: std::os::fd::BorrowedFd<'_>,
    level: libc::c_int,
    option: libc::c_int,
    value: libc::c_int,
) -> std::io::Result<()> {
    use std::mem::size_of;
    use std::os::fd::AsRawFd;

    // SAFETY: the option value points to `value`, a live c_int, and the
    // length passed is exactly a c_int's; setsockopt only reads it. A wrong
    // level or option for this socket fails with an error, not UB.
    let result = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            level,
            option,
            (&value as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
#[path = "libc_recv.rs"]
mod recv;
#[cfg(any(
    target_os = "l4re",
    target_os = "emscripten",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "nto",
))]
pub(crate) use recv::recv_ipv6_pktinfo;

/// Send `bytes` to `peer` in one `sendmsg`, with `source` and the interface
/// `index` in an `IPV6_PKTINFO` control message, built by hand where nix
/// can't send one: OpenBSD, DragonFly and the other Unix targets.
#[cfg(all(
    feature = "ipv6",
    not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
    ))
))]
pub(crate) fn send_ipv6_pktinfo(
    fd: std::os::fd::BorrowedFd<'_>,
    bytes: &[u8],
    peer: std::net::SocketAddrV6,
    source: std::net::Ipv6Addr,
    index: u32,
) -> std::io::Result<usize> {
    use std::mem::{size_of, zeroed};
    use std::os::fd::AsRawFd;

    let destination = socket2::SockAddr::from(peer);
    let mut control = [0usize; 16];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: msghdr is a plain C struct of integers and raw pointers, so
    // all zeros is a valid value (null pointers, zero lengths). Every field
    // set below points to storage that outlives the synchronous sendmsg.
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_name = destination.as_ptr().cast_mut().cast();
    message.msg_namelen = destination.len();
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    // SAFETY: `control` is a usize array, so it meets cmsghdr's alignment,
    // and the assert keeps CMSG_SPACE of one in6_pktinfo inside it. With
    // msg_controllen that large, CMSG_FIRSTHDR returns the non-null start
    // of `control`; CMSG_DATA pads to cmsghdr alignment, which covers
    // in6_pktinfo's, so the header and payload writes are aligned and in
    // bounds. sendmsg only reads `message`, the iovec, the destination and
    // `control`, all live for the call, and keeps no pointer to them.
    let count = unsafe {
        let space = libc::CMSG_SPACE(size_of::<libc::in6_pktinfo>() as _) as usize;
        assert!(space <= size_of_val(&control));
        message.msg_controllen = space as _;
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::IPPROTO_IPV6;
        (*header).cmsg_type = libc::IPV6_PKTINFO;
        (*header).cmsg_len = libc::CMSG_LEN(size_of::<libc::in6_pktinfo>() as _) as _;
        std::ptr::write(
            libc::CMSG_DATA(header).cast::<libc::in6_pktinfo>(),
            libc::in6_pktinfo {
                ipi6_addr: libc::in6_addr {
                    s6_addr: source.octets(),
                },
                ipi6_ifindex: index,
            },
        );
        libc::sendmsg(fd.as_raw_fd(), &message, 0)
    };
    if count < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(count as usize)
    }
}

/// `CMSG_SPACE(length)`: the room a control message with a `length`-octet
/// payload takes, padding included. The tests size the control buffer with
/// it.
#[cfg(all(
    test,
    any(
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
    )
))]
pub(crate) fn cmsg_space(length: usize) -> usize {
    // SAFETY: CMSG_SPACE is arithmetic on its argument; it touches no memory.
    unsafe { libc::CMSG_SPACE(length as _) as usize }
}
