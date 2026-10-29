//! Receive timestamps (#1546): the socket option that asks for them on each
//! OS, and the control message that carries one.

use super::*;

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use nix::sys::socket::ControlMessageOwned;

/// Ask the kernel to stamp each datagram `udp_socket` receives with the time
/// it arrived, which [`ReceivedDatagram::arrival_stamp`] then carries: the
/// realtime clock in nanoseconds on Linux (the software receive stamp of
/// `SO_TIMESTAMPING`), the monotonic
/// clock in Mach time units on macOS (`SO_TIMESTAMP_MONOTONIC`; the
/// microsecond `SO_TIMESTAMP` gives two datagrams queued back to back the
/// same stamp), and the realtime clock in microseconds on the BSDs
/// (`SO_TIMESTAMP`). Elsewhere, Windows included, it is `Unsupported`. Only
/// a transport that reads several sockets asks for it, to hand up their
/// datagrams in arrival order (#1546).
///
/// Linux stamps a packet as it arrives only while some socket on the host
/// wants stamps, and turns that on a moment after the first one asks.
/// `SO_TIMESTAMPNS` stamps a packet that arrived before then with the time
/// it is read, which would order it by the read; `SO_TIMESTAMPING` leaves it
/// unstamped, so it goes up in read order without claiming more.
pub(crate) fn request_arrival_stamps(udp_socket: &Socket) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use nix::sys::socket::{TimestampingFlag, setsockopt, sockopt};
        let flags = TimestampingFlag::SOF_TIMESTAMPING_RX_SOFTWARE
            | TimestampingFlag::SOF_TIMESTAMPING_SOFTWARE;
        Ok(setsockopt(udp_socket, sockopt::Timestamping, &flags)?)
    }
    // nix has no setter for this one.
    #[cfg(target_vendor = "apple")]
    {
        crate::sys::libc_msg::set_int_option(
            std::os::fd::AsFd::as_fd(udp_socket),
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMP_MONOTONIC,
            1,
        )
    }
    #[cfg(any(
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        use nix::sys::socket::{setsockopt, sockopt};
        Ok(setsockopt(udp_socket, sockopt::ReceiveTimestamp, &true)?)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    {
        let _ = udp_socket;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "receive timestamps are unsupported on this platform",
        ))
    }
}

/// The receive stamp a control message carries, if it is one and holds a
/// stamp: the software stamp of `SCM_TIMESTAMPING`, whose zero is no stamp.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn arrival_stamp_cmsg(message: &ControlMessageOwned) -> Option<u128> {
    let ControlMessageOwned::ScmTimestampsns(stamps) = message else {
        return None;
    };
    arrival_stamp(
        stamps.system.tv_sec(),
        stamps.system.tv_nsec(),
        1_000_000_000,
    )
    .filter(|&stamp| stamp != 0)
}

/// The control message type of a `SO_TIMESTAMP_MONOTONIC` stamp, a `u64` of
/// Mach absolute time, from the macOS SDK's `sys/socket.h` (libc lacks it).
#[cfg(target_vendor = "apple")]
pub(super) const SCM_TIMESTAMP_MONOTONIC: libc::c_int = 0x04;

/// The receive stamp a control message carries, if it is one. nix doesn't
/// decode `SCM_TIMESTAMP_MONOTONIC`, so its `u64` is read from the bytes. A
/// short one is no stamp: the datagram still goes up, in read order.
#[cfg(target_vendor = "apple")]
pub(super) fn arrival_stamp_cmsg(message: &ControlMessageOwned) -> Option<u128> {
    let ControlMessageOwned::Unknown(raw) = message else {
        return None;
    };
    if raw.cmsg_header.cmsg_level != libc::SOL_SOCKET
        || raw.cmsg_header.cmsg_type != SCM_TIMESTAMP_MONOTONIC
    {
        return None;
    }
    let ticks = octets_at(&raw.data_bytes, 0).map(u64::from_ne_bytes)?;
    Some(u128::from(ticks))
}

/// The receive stamp a control message carries, if it is one: the
/// `SCM_TIMESTAMP` time of day.
#[cfg(any(
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(super) fn arrival_stamp_cmsg(message: &ControlMessageOwned) -> Option<u128> {
    let ControlMessageOwned::ScmTimestamp(time) = message else {
        return None;
    };
    arrival_stamp(time.tv_sec(), time.tv_usec(), 1_000_000)
}

/// A realtime stamp of `seconds` and a `fraction` of `per_second` parts,
/// counted in those parts; `None` before the epoch or with a fraction out of
/// range, which no kernel stamp is.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub(super) fn arrival_stamp(
    seconds: impl TryInto<u64>,
    fraction: impl TryInto<u64>,
    per_second: u64,
) -> Option<u128> {
    let (seconds, fraction) = (seconds.try_into().ok()?, fraction.try_into().ok()?);
    (fraction < per_second)
        .then(|| u128::from(seconds) * u128::from(per_second) + u128::from(fraction))
}
