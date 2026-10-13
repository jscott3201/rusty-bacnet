//! The sockets a B/IP transport binds at start (#892, #950, #1538), and one
//! destruction order for a socket and the registered-object lease.
//!
//! A transport binds one of two ways:
//!
//! - **Wildcard** (the default): one socket on `0.0.0.0:port`. It receives
//!   unicast and broadcasts alike, in the order they arrive, and the receive
//!   loop takes unicast only to the interface address. A port the OS picks is
//!   never shared (#892); an explicitly requested one sets `SO_REUSEADDR`.
//! - **Per address**, opted into with
//!   [`set_share_port_by_address`](super::BipTransport::set_share_port_by_address)
//!   and needing an explicit interface and a nonzero port: the socket binds
//!   `interface:port`, so transports on different addresses of one host can
//!   share one port, each getting only its own unicast (#1538). Every send
//!   leaves from that socket, so its source is the interface address. The
//!   configured broadcast address must be the interface's subnet broadcast
//!   or 255.255.255.255 (or, on a loopback interface, the interface itself,
//!   as loopback tests set it), judged by the netmask the host reports. How
//!   broadcasts arrive differs by OS:
//!   - Linux delivers a broadcast only to sockets bound to the wildcard
//!     address or to the broadcast address itself. Receive-only listeners
//!     bind the configured broadcast address and 255.255.255.255 with
//!     `SO_REUSEADDR`, which several transports on one subnet share, and Linux
//!     hands a broadcast to each. No listener can see a unicast, and the
//!     address socket needs no `SO_REUSEADDR`, so no other socket can bind the
//!     same address and port, or the wildcard address on it.
//!   - macOS and the BSDs refuse to bind 255.255.255.255, so one receive-only
//!     listener binds `0.0.0.0:port` with `SO_REUSEADDR` and `SO_REUSEPORT`,
//!     which several such listeners need, and each gets a broadcast. A unicast
//!     to a local address that no socket on the port is bound to can reach a
//!     listener, which drops it. The address socket sets `SO_REUSEADDR` to
//!     bind beside the listeners; another socket binding the same address and
//!     port would also need `SO_REUSEPORT` on both, so it is refused.
//!   - Windows delivers a broadcast arriving on the interface to a socket bound
//!     to the interface address, so that one socket is enough. Its
//!     `SO_REUSEADDR` would let another socket bind the same address and take
//!     the unicast sent there, so it claims the address with
//!     `SO_EXCLUSIVEADDRUSE` instead, as `port_ownership` does for a private
//!     port (#950). Other addresses can still share the port, and so can a
//!     socket already bound to the wildcard address without
//!     `SO_EXCLUSIVEADDRUSE`.
//!
//!   The receive loop reads the address socket and the listeners fairly, in
//!   no fixed order, so a broadcast and a unicast that arrive together may be
//!   handled in either order; a unicast that depends on a broadcast sent just
//!   before it can be handled first. A listener on 255.255.255.255 or
//!   `0.0.0.0` hears every interface, so it keeps only what arrived on the
//!   transport's own (the index from `IP_PKTINFO` on Linux, `IP_RECVIF` on
//!   macOS and the BSDs). A listener that fails is closed, and unicast goes
//!   on.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::{ops::Deref, sync::Arc};

use bacnet_types::error::Error;
use tokio::net::UdpSocket;

use crate::local_addresses::LocalInterface;

/// What a socket is for, which decides how it shares its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SocketRole {
    /// `0.0.0.0` on a port the OS picks. Linux gives an ephemeral port to an
    /// `SO_REUSEADDR` socket even while another `SO_REUSEADDR` socket owns it,
    /// and unicast to that port then reaches only one of them (#892), so it
    /// never sets the option, and claims the port where the OS has a way to
    /// (`port_ownership`, #950).
    PrivateWildcard,
    /// `0.0.0.0` on an explicitly requested port: `SO_REUSEADDR`, as before.
    SharedWildcard,
    /// `interface:port` in per-address mode (#1538): shares nothing on Linux,
    /// sits beside a wildcard listener on macOS and the BSDs, and claims the
    /// address on Windows.
    Address,
    /// A receive-only broadcast listener beside an [`Address`](Self::Address)
    /// socket. Not used on Windows.
    BroadcastListener,
}

/// An unbound UDP socket set up for `role`.
pub(super) fn udp_socket(role: SocketRole) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    match role {
        SocketRole::PrivateWildcard => crate::port_ownership::claim_exclusive(&socket)?,
        SocketRole::SharedWildcard => socket.set_reuse_address(true)?,
        #[cfg(windows)]
        SocketRole::Address => crate::port_ownership::claim_exclusive(&socket)?,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        SocketRole::Address => {}
        #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
        SocketRole::Address => socket.set_reuse_address(true)?,
        SocketRole::BroadcastListener => {
            socket.set_reuse_address(true)?;
            #[cfg(any(
                target_vendor = "apple",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd",
            ))]
            socket.set_reuse_port(true)?;
        }
    }
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// What one start binds, from the transport's configuration.
#[derive(Clone, Copy, Debug)]
pub(super) struct BindPlan {
    pub(super) interface: Ipv4Addr,
    pub(super) port: u16,
    /// Whether the port was explicitly requested (fixed at construction).
    pub(super) share_port: bool,
    /// Whether per-address mode was asked for.
    pub(super) by_address: bool,
    /// The configured broadcast address.
    pub(super) broadcast: Ipv4Addr,
    /// What the host reports about the interface, looked up for
    /// per-address mode only.
    pub(super) local: Option<LocalInterface>,
}

/// The sockets one start binds: the one every send leaves from, and in
/// per-address mode on Unix, the listeners broadcasts arrive on.
pub(super) struct BoundSockets {
    pub(super) primary: socket2::Socket,
    pub(super) listeners: Vec<socket2::Socket>,
}

impl super::BipTransport {
    /// Check that this transport could start now: bind the sockets
    /// [`start`](crate::port::TransportPort::start) would, with the same
    /// options, and release them. Another socket can still take the port
    /// before the real start, which stays authoritative.
    /// With per-address mode it lists the host's interfaces, which can take
    /// a moment on a host with many adapters.
    pub fn check_bind(&self) -> Result<(), Error> {
        probe_interface(self.interface)?;
        let mut plan = self.bind_plan();
        if plan.by_address {
            plan.local = looked_up(
                plan.interface,
                crate::local_addresses::interface_of(plan.interface),
            );
        }
        bind(plan).map(drop).map_err(Error::Transport)
    }

    /// What `start()` binds. In per-address mode it looks the interface up
    /// on a blocking thread.
    pub(super) async fn bind_plan_for_start(&self) -> BindPlan {
        let mut plan = self.bind_plan();
        if plan.by_address {
            let ip = plan.interface;
            let lookup =
                tokio::task::spawn_blocking(move || crate::local_addresses::interface_of(ip));
            let found = lookup.await.unwrap_or_else(|e| Err(io::Error::other(e)));
            plan.local = looked_up(ip, found);
        }
        plan
    }

    fn bind_plan(&self) -> BindPlan {
        BindPlan {
            interface: self.interface,
            port: self.port,
            share_port: self.share_port,
            by_address: self.share_port_by_address,
            broadcast: self.broadcast_address,
            local: None,
        }
    }
}

/// The interface lookup's answer. A failed lookup turns off the
/// broadcast-address check and the arrival-interface filter, so it is
/// logged.
fn looked_up(ip: Ipv4Addr, found: io::Result<Option<LocalInterface>>) -> Option<LocalInterface> {
    found.unwrap_or_else(|error| {
        tracing::debug!(%ip, %error, "Could not look up the B/IP interface");
        None
    })
}

/// In per-address mode the configured broadcast address must be the
/// interface's subnet broadcast or 255.255.255.255, or, on a loopback
/// interface only, the interface itself (as a loopback test sets it up, with
/// no listener for it). Another address would lose this subnet's directed
/// broadcasts: on Linux a listener binds it, and any local address binds
/// without complaint. Where the host reports no netmask for the interface,
/// the bind decides.
pub(super) fn check_broadcast(plan: &BindPlan) -> io::Result<()> {
    let (interface, broadcast) = (plan.interface, plan.broadcast);
    if broadcast.is_broadcast() || (broadcast == interface && interface.is_loopback()) {
        return Ok(());
    }
    const LOST: &str = "so a transport sharing its port by address would lose this subnet's \
                        broadcasts";
    let refused = |reason: String| {
        let message = format!("the B/IP broadcast address {broadcast} {reason}, {LOST}");
        Err(io::Error::new(io::ErrorKind::InvalidInput, message))
    };
    if broadcast == interface {
        return refused(
            "is the interface's own address, which only a loopback interface may use; set the \
             subnet broadcast or 255.255.255.255"
                .to_string(),
        );
    }
    let Some(local) = plan.local.filter(|local| local.netmask.is_some()) else {
        return Ok(());
    };
    match local.subnet_broadcast(interface) {
        Some(expected) if expected == broadcast => Ok(()),
        Some(expected) => refused(format!(
            "is neither the subnet broadcast of {interface} ({expected}) nor 255.255.255.255"
        )),
        None => refused(format!(
            "is not 255.255.255.255, and the subnet of {interface} has no broadcast address"
        )),
    }
}

/// Fail on an interface address this host doesn't have. A wildcard bind
/// would succeed anyway, so this binds a throwaway socket on an ephemeral
/// port: a misconfigured interface fails fast at startup rather than
/// silently advertising an unowned IP in I-Am replies via the local MAC
/// (tests::start_fails_on_nonlocal_interface).
pub(super) fn probe_interface(interface: Ipv4Addr) -> Result<(), Error> {
    if !interface.is_unspecified() {
        std::net::UdpSocket::bind(SocketAddrV4::new(interface, 0)).map_err(Error::Transport)?;
    }
    Ok(())
}

/// Bind the sockets `plan` asks for, as the module docs describe.
pub(super) fn bind(plan: BindPlan) -> io::Result<BoundSockets> {
    let bound = |role: SocketRole, ip: Ipv4Addr| -> io::Result<socket2::Socket> {
        let socket = udp_socket(role)?;
        socket.bind(&SocketAddrV4::new(ip, plan.port).into())?;
        Ok(socket)
    };
    if !plan.by_address {
        let role = if plan.share_port {
            SocketRole::SharedWildcard
        } else {
            SocketRole::PrivateWildcard
        };
        let primary = bound(role, Ipv4Addr::UNSPECIFIED)?;
        let listeners = Vec::new();
        return Ok(BoundSockets { primary, listeners });
    }
    if plan.interface.is_unspecified() || !plan.share_port {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a B/IP transport shares its port by address only with an explicit interface \
             address and a nonzero port",
        ));
    }
    check_broadcast(&plan)?;
    let primary = bound(SocketRole::Address, plan.interface)?;
    let mut listeners = Vec::new();
    for ip in listener_addresses(plan.interface, plan.broadcast) {
        let listener = bound(SocketRole::BroadcastListener, ip).map_err(|e| {
            let port = plan.port;
            let message =
                format!("could not bind the B/IP broadcast listener to {ip}:{port} ({e})");
            io::Error::new(e.kind(), message)
        })?;
        listeners.push(listener);
    }
    Ok(BoundSockets { primary, listeners })
}

/// Where per-address mode's broadcast listeners bind: on Linux the
/// configured broadcast address, unless it is the limited broadcast or the
/// interface itself, and 255.255.255.255; on macOS and the BSDs the wildcard
/// address; on Windows nowhere.
fn listener_addresses(interface: Ipv4Addr, broadcast: Ipv4Addr) -> Vec<Ipv4Addr> {
    if cfg!(windows) {
        Vec::new()
    } else if cfg!(any(target_os = "linux", target_os = "android")) {
        let directed =
            !broadcast.is_broadcast() && !broadcast.is_unspecified() && broadcast != interface;
        let mut addresses: Vec<_> = directed.then_some(broadcast).into_iter().collect();
        addresses.push(Ipv4Addr::BROADCAST);
        addresses
    } else {
        vec![Ipv4Addr::UNSPECIFIED]
    }
}

/// Every socket-owning worker retains this same allocation. Rust drops fields
/// in declaration order: the socket closes before the final registration token.
pub(super) struct BipSocket {
    socket: UdpSocket,
    _network_port_lease: Option<Arc<()>>,
    /// Where a test's sends go instead of the network.
    #[cfg(test)]
    sink: Option<super::send_sink::SendSink>,
}

impl BipSocket {
    pub(super) fn new(socket: UdpSocket, lease: Option<Arc<()>>) -> Self {
        Self {
            socket,
            _network_port_lease: lease,
            #[cfg(test)]
            sink: None,
        }
    }

    /// A socket whose sends go to `sink`, for tests.
    #[cfg(test)]
    pub(super) fn recording(socket: UdpSocket, sink: super::send_sink::SendSink) -> Self {
        Self {
            sink: Some(sink),
            ..Self::new(socket, None)
        }
    }

    /// Send one datagram. Every send of the transport goes through here.
    pub(super) async fn send_to(&self, buf: &[u8], target: SocketAddrV4) -> io::Result<usize> {
        #[cfg(test)]
        if let Some(sink) = &self.sink {
            if sink.yields() {
                tokio::task::yield_now().await;
            }
            return sink.send(buf, target);
        }
        self.socket.send_to(buf, target).await
    }
}

impl Deref for BipSocket {
    type Target = UdpSocket;
    fn deref(&self) -> &Self::Target {
        &self.socket
    }
}
