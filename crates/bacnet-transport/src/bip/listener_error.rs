//! The error a B/IP receive-only listener's bind or group join fails with,
//! or a B/IP or B/IPv6 socket's bind of a port another socket holds (#2366,
//! #2385): it names the socket and keeps the OS error as its source (#2231).
//! Both transports take a held port's text from one place here
//! ([`held_port_error`]), so the two can't drift.
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
pub(crate) fn listener_error(context: String, os: io::Error) -> io::Error {
    io::Error::new(os.kind(), ListenerError { context, os })
}

/// Which transport found its port held: its name in the error, and the IP
/// version Windows' excluded port ranges are listed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldTransport {
    /// B/IP, over IPv4.
    Bip,
    /// B/IPv6.
    #[cfg_attr(not(feature = "ipv6"), allow(dead_code))]
    Bip6,
}

impl HeldTransport {
    fn name(self) -> &'static str {
        match self {
            Self::Bip => "B/IP",
            Self::Bip6 => "B/IPv6",
        }
    }

    fn ip_version(self) -> &'static str {
        match self {
            Self::Bip => "ipv4",
            Self::Bip6 => "ipv6",
        }
    }
}

/// How the socket that found its port held was binding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeldBind {
    /// Claiming the port, the default (#2366, #2385).
    Claiming,
    /// Sharing it, as `set_share_wildcard_port` asks.
    Sharing,
    /// B/IP's per-address socket (#1538), which has no opt-in to name.
    Address,
}

/// Whether a bind failed because the address and port are taken. Windows
/// refuses a bind beside a socket that holds the port with
/// `SO_EXCLUSIVEADDRUSE` as WSAEACCES, and so it does a port inside a range
/// it has excluded.
pub(crate) fn port_held(os: &io::Error) -> bool {
    os.kind() == io::ErrorKind::AddrInUse || access_denied(os)
}

/// Whether `os` is Windows' WSAEACCES for a bind.
fn access_denied(os: &io::Error) -> bool {
    cfg!(windows) && os.kind() == io::ErrorKind::PermissionDenied
}

/// The error of a requested port's bind that [`port_held`] says is held:
/// what [`held_port_text`] says of `place`, with `os` as its source.
pub(crate) fn held_port_error(
    transport: HeldTransport,
    place: &str,
    bind: HeldBind,
    os: io::Error,
) -> io::Error {
    let context = held_port_text(transport, place, bind, access_denied(&os));
    listener_error(context, os)
}

/// What a held port's error says, for B/IP and B/IPv6 alike. `place` names
/// the port, and the address when the socket binds one.
///
/// A claiming bind is told how to share. A sharing bind is told the port
/// can't be shared, without saying why: its holder may not have asked to
/// share it, or the OS may never share one address and port, as macOS and
/// the BSDs don't between two sockets that only set `SO_REUSEADDR`.
///
/// `access_denied` is Windows' WSAEACCES, which has two causes the OS
/// doesn't tell apart: another socket holds the port, or the port lies in a
/// range Windows has excluded, where sharing is no help.
fn held_port_text(
    transport: HeldTransport,
    place: &str,
    bind: HeldBind,
    access_denied: bool,
) -> String {
    let name = transport.name();
    const SHARE: &str = "share the port (share_wildcard_port, or --share-port in the CLI), after \
                         which unicast reaches only one of the sockets on it";
    const UNSHARED: &str = "can't be shared: its holder didn't ask to share it, or this OS never \
                            shares one address and port";
    if !access_denied {
        return match bind {
            HeldBind::Claiming => format!(
                "{place} is in use on this host, and a {name} transport doesn't share its port \
                 by default; stop the other program, use another port, or {SHARE}"
            ),
            HeldBind::Sharing => format!("{place} is in use on this host and {UNSHARED}"),
            HeldBind::Address => format!("{place} is in use on this host"),
        };
    }
    let version = transport.ip_version();
    let either = format!(
        "{place} is held by another socket on this host, or is inside a port range Windows \
         has excluded (netsh interface {version} show excludedportrange protocol=udp)"
    );
    match bind {
        HeldBind::Claiming => format!(
            "{either}; a {name} transport doesn't share its port by default: use another \
             port, or, if another program holds this one, stop it or {SHARE}"
        ),
        HeldBind::Sharing => format!(
            "{either}; use another port: held, it {UNSHARED}, and an excluded one can't be \
             bound at all"
        ),
        HeldBind::Address => format!("{either}; use another port"),
    }
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

    use super::{HeldBind, HeldTransport, held_port_text};

    /// A held port's text, by how the socket was binding: a claiming bind
    /// names the opt-in, and a sharing one says the port can't be shared
    /// without blaming its holder, since macOS and the BSDs refuse two
    /// sockets that both asked to share.
    #[test]
    fn a_held_port_says_what_its_bind_can_do_about_it() {
        let text = |bind| held_port_text(HeldTransport::Bip6, "UDP port 47808", bind, false);
        let claiming = text(HeldBind::Claiming);
        assert!(
            claiming.starts_with(
                "UDP port 47808 is in use on this host, and a B/IPv6 transport doesn't share \
                 its port by default; stop the other program, use another port, or share the \
                 port (share_wildcard_port, or --share-port in the CLI)"
            ),
            "{claiming}"
        );
        assert_eq!(
            text(HeldBind::Sharing),
            "UDP port 47808 is in use on this host and can't be shared: its holder didn't ask \
             to share it, or this OS never shares one address and port"
        );
        assert_eq!(
            text(HeldBind::Address),
            "UDP port 47808 is in use on this host"
        );
        let bip = held_port_text(HeldTransport::Bip, "UDP port 1", HeldBind::Claiming, false);
        assert!(bip.contains("a B/IP transport doesn't share"), "{bip}");
        for said in [claiming, text(HeldBind::Sharing), text(HeldBind::Address)] {
            assert!(!said.contains("excluded"), "{said}");
            assert!(!said.contains("refuses"), "{said}");
        }
    }

    /// Windows' WSAEACCES names both of its causes, with the command that
    /// lists the excluded ranges for the transport's IP version, and never
    /// offers sharing as the only way out.
    #[test]
    fn an_access_denied_bind_names_a_held_port_and_an_excluded_range() {
        for (transport, command) in [
            (
                HeldTransport::Bip,
                "netsh interface ipv4 show excludedportrange protocol=udp",
            ),
            (
                HeldTransport::Bip6,
                "netsh interface ipv6 show excludedportrange protocol=udp",
            ),
        ] {
            for bind in [HeldBind::Claiming, HeldBind::Sharing, HeldBind::Address] {
                let text = held_port_text(transport, "UDP port 47808 on ::1", bind, true);
                assert!(
                    text.starts_with("UDP port 47808 on ::1 is held by another socket"),
                    "{text}"
                );
                assert!(text.contains("a port range Windows has excluded"), "{text}");
                assert!(text.contains(command), "{text}");
                assert!(text.contains("use another port"), "{text}");
                assert_eq!(
                    text.contains("share_wildcard_port"),
                    bind == HeldBind::Claiming,
                    "{text}"
                );
                assert_eq!(
                    text.contains("can't be shared"),
                    bind == HeldBind::Sharing,
                    "{text}"
                );
            }
        }
    }

    /// On Windows a bind that asks to share a port another socket claimed
    /// fails with WSAEACCES, the code an excluded range gives too, so the
    /// error names both causes and keeps the code.
    #[cfg(windows)]
    #[test]
    fn a_sharing_bind_beside_a_claimed_port_is_access_denied_on_windows() {
        use std::net::SocketAddr;
        let socket = |claim: bool| {
            let socket =
                socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
            if claim {
                crate::port_ownership::claim_exclusive(&socket).unwrap();
            } else {
                socket.set_reuse_address(true).unwrap();
            }
            socket
        };
        let holder = socket(true);
        holder
            .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())
            .unwrap();
        let held = holder.local_addr().unwrap();
        let os = socket(false)
            .bind(&held)
            .expect_err("a sharing bind beside a claimed port");
        assert_eq!(
            (os.kind(), os.raw_os_error()),
            (std::io::ErrorKind::PermissionDenied, Some(10013)),
            "Windows refused the bind with {os:?}"
        );
        assert!(super::port_held(&os));
        let place = "UDP port 47808";
        let error = super::held_port_error(HeldTransport::Bip, place, HeldBind::Sharing, os);
        let shown = error.to_string();
        assert!(
            shown.contains("a port range Windows has excluded"),
            "{shown}"
        );
        assert!(shown.contains("can't be shared"), "{shown}");
        let source = error.source().expect("the OS error as the source");
        let source = source.downcast_ref::<std::io::Error>().unwrap();
        assert_eq!(source.raw_os_error(), Some(10013));
    }
}
