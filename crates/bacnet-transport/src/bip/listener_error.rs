//! The error a B/IP receive-only listener's bind or group join fails with,
//! or a wildcard socket's bind of a port another socket holds (#2366): it
//! names the socket and keeps the OS error as its source (#2231).
//!
//! An `io::Error` built from a message string keeps the kind but drops
//! `raw_os_error()`, so a caller could tell a port held by another socket
//! apart only by the wording. Here the original error stays reachable through
//! [`std::error::Error::source`] on the returned `io::Error` (which forwards
//! to this type's source), with its OS code intact.

use std::fmt;
use std::io;

/// A listener failure: which listener and what it tried, then the OS error.
#[derive(Debug)]
struct ListenerError {
    /// What failed, naming the listener and its address.
    context: String,
    /// The error the OS returned.
    os: io::Error,
}

impl fmt::Display for ListenerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.context, self.os)
    }
}

impl std::error::Error for ListenerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.os)
    }
}

/// `os` as an I/O error of its kind whose message starts with `context` and
/// whose `source()` is `os` itself, so its OS code survives.
pub(super) fn listener_error(context: String, os: io::Error) -> io::Error {
    io::Error::new(os.kind(), ListenerError { context, os })
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::net::{Ipv4Addr, UdpSocket};

    /// A wrapped bind failure keeps its message, its kind and, through its
    /// source, the OS code of the bind it wraps.
    #[test]
    fn a_wrapped_bind_error_keeps_its_os_code() {
        let holder = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let os = UdpSocket::bind(holder.local_addr().unwrap()).unwrap_err();
        let code = os.raw_os_error().expect("a bind error from the OS");
        let kind = os.kind();
        let shown = os.to_string();

        let wrapped = super::listener_error("could not bind the B/IP test listener".into(), os);

        assert_eq!(wrapped.kind(), kind);
        assert_eq!(
            wrapped.to_string(),
            format!("could not bind the B/IP test listener ({shown})")
        );
        let source = wrapped.source().expect("the OS error as the source");
        let source = source.downcast_ref::<std::io::Error>().unwrap();
        assert_eq!(source.raw_os_error(), Some(code));
    }
}
