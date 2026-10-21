//! The crate's operating-system FFI, and the only module in `bacnet-transport`
//! allowed to use `unsafe` (#1923).
//!
//! Everything here is a thin wrapper over one OS call or one read of OS-owned
//! memory: socket options, `recvmsg`/`sendmsg` with hand-built control
//! messages, Winsock's extension functions, the interface lists of
//! `getifaddrs` and `GetAdaptersAddresses`, and the RS-485 ioctl. Each one
//! takes and returns safe types (slices, borrowed sockets and descriptors,
//! socket addresses, plain structs), so the rest of the crate calls them with
//! no unsafe of its own, and each unsafe block keeps its `// SAFETY:`
//! argument beside it.
//!
//! # The rule
//!
//! The workspace denies `unsafe_code`; this module's inner
//! `#![allow(unsafe_code)]` is the crate's only exception. Clippy's
//! `undocumented_unsafe_blocks` and `missing_safety_doc`, and rustc's
//! `unsafe_op_in_unsafe_fn`, still apply inside it. An unsafe fn whose
//! precondition the wrapper can't check stays private to its submodule,
//! behind a safe fn that upholds it. The `unsafe_stays_in_sys` test fails on
//! any `unsafe` token or unsafe-code allow anywhere else in the crate.
//!
//! - `ifaddrs`: the one `getifaddrs` walk (Unix).
//! - `win_adapters`: `GetAdaptersAddresses` (Windows).
//! - `winsock`: Winsock socket options, `WSARecvMsg` and `WSASendMsg`.
//! - `libc_msg`: the libc socket option, `recvmsg` and `sendmsg` paths, for
//!   the Unix targets nix doesn't cover.
//! - `serial`: Linux's `TIOCSRS485` and `TIOCGRS485`.
//! - `sched`: Linux's `pthread_setschedparam` and its read-back, for the MS/TP
//!   transport's dedicated thread.

#![allow(unsafe_code)]

with_getifaddrs! {
    pub(crate) mod ifaddrs;
}

#[cfg(unix)]
pub(crate) mod libc_msg;

#[cfg(all(feature = "serial", any(target_os = "linux", test)))]
pub(crate) mod serial;

#[cfg(target_os = "linux")]
pub(crate) mod sched;

#[cfg(windows)]
pub(crate) mod win_adapters;

#[cfg(windows)]
pub(crate) mod winsock;

/// Room for the control messages one IPv4 or IPv6 datagram carries: packet
/// info on Linux and Windows, or a destination address and a whole
/// link-layer address (`IP_RECVIF`) on the BSDs, whose padding can take 84
/// octets on a 32-bit BSD, plus a receive timestamp where one was asked for.
/// Its alignment covers any `cmsghdr`, which the libc control-message walk
/// relies on.
///
/// It is also what keeps nix's decoding in bounds. nix reads each message
/// it decodes as a whole struct, without checking `cmsg_len`: `IP_RECVIF`,
/// for one, reads a full `sockaddr_dl` (54 octets on FreeBSD) where the
/// kernel sends about 17. So the buffer must be zeroed for each receive and
/// stay large enough for the messages plus the largest struct nix decodes
/// here, never smaller; such a read then lands on zeros inside it, and a
/// malformed message gives wrong values rather than an out-of-bounds read.
/// `the_control_buffer_holds_every_message_a_datagram_carries` checks the
/// size against whole-struct payloads.
#[cfg(any(unix, windows))]
#[repr(C, align(8))]
pub(crate) struct ControlBuffer([u8; 256]);

#[cfg(any(unix, windows))]
impl ControlBuffer {
    /// A zeroed buffer, for one receive. It is the only way to make one, so
    /// every receive starts from zeros.
    pub(crate) fn new() -> Self {
        Self([0; 256])
    }

    /// The buffer's octets, for parsing what a receive wrote.
    #[cfg(windows)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// The buffer's octets, for a receive to write its control messages to.
    /// The slice starts at the buffer's 8-aligned start.
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// `Ok` when `used`, the raw socket handle a call is about to use, is
/// `looked_up`, the one a Winsock extension function was looked up on;
/// `InvalidInput` otherwise.
#[cfg(any(windows, test))]
fn same_socket<Handle: PartialEq>(looked_up: Handle, used: Handle) -> std::io::Result<()> {
    if looked_up == used {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Winsock extension function used on another socket than its own",
        ))
    }
}

#[cfg(test)]
#[test]
fn an_extension_function_runs_only_on_its_own_socket() {
    assert!(same_socket(0x1f4, 0x1f4).is_ok());
    let error = same_socket(0x1f4, 0x1f8).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(test)]
#[path = "fence_tests.rs"]
mod fence_tests;
