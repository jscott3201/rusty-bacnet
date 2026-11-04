//! The sockets a B/IP transport binds at start (#892, #950, #1538), and one
//! destruction order for a socket and the registered-object lease.
//!
//! A transport binds one of two ways:
//!
//! - **Wildcard** (the default): one socket on `0.0.0.0:port`. It receives
//!   unicast and broadcasts alike, in the order they arrive, and the receive
//!   loop takes unicast only to the interface address. The socket claims its
//!   port, whether the OS picked it (#892) or it was requested (#2366): it
//!   sets no `SO_REUSEADDR`, and on Windows it sets `SO_EXCLUSIVEADDRUSE`
//!   (`port_ownership`, #950). Another socket that later binds the port then
//!   fails, where with sharing it would take the unicast sent to the port.
//!   macOS and the BSDs still let a socket that sets `SO_REUSEADDR` bind a
//!   specific address on the port, and that socket then gets the unicast
//!   sent to its address; they have no option to stop it.
//!   [`set_share_wildcard_port`](super::BipTransport::set_share_wildcard_port)
//!   opts a requested port back into `SO_REUSEADDR`, so other sockets that
//!   set it can bind the port too, and each unicast reaches only one of them.
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
//!     socket of the same user already bound to the wildcard address without
//!     `SO_EXCLUSIVEADDRUSE` (a transport that shares its wildcard port).
//!
//!   Every socket then asks for receive timestamps, and the receive loop
//!   hands up what the address socket and the listeners hold in the order
//!   the kernel received it (#1546, ingress.rs), so a unicast that depends on
//!   a broadcast sent just before it is handled after it. A listener on
//!   255.255.255.255 or `0.0.0.0` hears every interface, so it keeps only
//!   what arrived on the transport's own (the index from `IP_PKTINFO` on
//!   Linux, `IP_RECVIF` on macOS and the BSDs). A listener that fails is
//!   closed, and unicast goes on.
//!
//! A B/IP-M node (#941) adds one receive-only listener on its multicast
//! group (on Windows the primary socket joins the group instead): see
//! multicast.rs. It binds the wildcard way, except on Linux when it starts
//! with its group on its own requested port. There the group's listener
//! binds that port too, which a claimed wildcard socket would refuse, so the
//! node takes the per-address shape instead: the address socket, the
//! broadcast listeners (whose broadcasts the node drops, as a member takes
//! broadcasts only from its group) and the group's listener. Every squatter,
//! on the wildcard or the interface address, with or without
//! `SO_REUSEADDR` or `SO_REUSEPORT`, is then refused. Its listeners hear
//! what the `0.0.0.0` socket heard: 255.255.255.255 and the interface's own
//! subnet broadcast from the reported netmask
//! ([`group_shape_listener_addresses`]). Unlike a node that asks for
//! per-address binding, it doesn't check the configured broadcast address,
//! and a broadcast listener that can't bind is left out with a warning; only
//! the address socket and the group's listener can fail the start. A B/IP-M node always
//! has an explicit interface (`set_multicast` refuses `0.0.0.0`); without
//! one it would bind the wildcard way. A node that shares its wildcard port
//! on request keeps the wildcard way, as do macOS, where the group's
//! listener binds after the claimed socket, and Windows.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::{ops::Deref, sync::Arc};

use bacnet_types::data_link::DataLink;
use bacnet_types::error::Error;
use tokio::net::UdpSocket;

use super::datagram::OutgoingDatagram;
use crate::local_addresses::LocalInterface;
use crate::tap::{FrameTap, TapDirection, TapPeer};

/// What a socket is for, which decides how it shares its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SocketRole {
    /// `0.0.0.0`, claiming its port where the OS has a way to
    /// (`port_ownership`, #950), with no `SO_REUSEADDR`: on a port the OS
    /// picks, since Linux gives an ephemeral port to an `SO_REUSEADDR` socket
    /// even while another `SO_REUSEADDR` socket owns it (#892), and by
    /// default on a requested one, so a later bind of it fails (#2366).
    ClaimedWildcard,
    /// `0.0.0.0` on a requested port, opted into sharing it:
    /// `SO_REUSEADDR`.
    SharedWildcard,
    /// `interface:port` in per-address mode (#1538): shares nothing on Linux,
    /// sits beside a wildcard listener on macOS and the BSDs, and claims the
    /// address on Windows.
    Address,
    /// A receive-only broadcast listener beside an [`Address`](Self::Address)
    /// socket. Not used on Windows.
    BroadcastListener,
    /// A receive-only B/IP-M listener on its multicast group's address and
    /// port (multicast.rs, #941), which every member on the host binds too.
    /// Not used on Windows.
    #[cfg_attr(windows, allow(dead_code))]
    GroupListener,
}

/// An unbound UDP socket set up for `role`.
pub(super) fn udp_socket(role: SocketRole) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    match role {
        SocketRole::ClaimedWildcard => crate::port_ownership::claim_exclusive(&socket)?,
        SocketRole::SharedWildcard => socket.set_reuse_address(true)?,
        #[cfg(windows)]
        SocketRole::Address => crate::port_ownership::claim_exclusive(&socket)?,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        SocketRole::Address => {}
        #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
        SocketRole::Address => socket.set_reuse_address(true)?,
        SocketRole::BroadcastListener | SocketRole::GroupListener => {
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
    pub(super) requested_port: bool,
    /// Whether a requested port is shared in wildcard mode
    /// (`set_share_wildcard_port`).
    pub(super) share_wildcard: bool,
    /// Whether per-address mode was asked for.
    pub(super) by_address: bool,
    /// The configured broadcast address.
    pub(super) broadcast: Ipv4Addr,
    /// What the host reports about the interface, looked up for the
    /// per-address shape only.
    pub(super) local: Option<LocalInterface>,
    /// A B/IP-M node's group (multicast.rs, #941).
    pub(super) multicast: Option<super::BipMulticast>,
}

impl BindPlan {
    /// Whether the start binds the per-address shape: asked for, or a Linux
    /// B/IP-M node starting with its group on its own requested port
    /// (module docs, #2366).
    pub(super) fn per_address(&self) -> bool {
        self.by_address || self.group_by_address()
    }

    fn group_by_address(&self) -> bool {
        cfg!(any(target_os = "linux", target_os = "android"))
            && self.requested_port
            && !self.share_wildcard
            && !self.interface.is_unspecified()
            && self
                .multicast
                .is_some_and(|group| group.endpoint(self.port).port() == self.port)
    }
}

/// The sockets one start binds: the one every send leaves from, in the
/// per-address shape on Unix the listeners broadcasts arrive on, and a
/// B/IP-M node's group listener (not on Windows).
pub(super) struct BoundSockets {
    pub(super) primary: socket2::Socket,
    pub(super) listeners: Vec<socket2::Socket>,
    pub(super) group: Option<socket2::Socket>,
}

impl super::BipTransport {
    /// Share an explicitly requested port with other sockets on this host
    /// that bind it the same way, as before #2366. Off by default: the
    /// wildcard socket claims its port, so a second bind of it fails at once
    /// (on Windows with `SO_EXCLUSIVEADDRUSE`), and a start that finds the
    /// port held fails with an error that says so. Call it before `start()`.
    ///
    /// With sharing on, the socket sets `SO_REUSEADDR`. Broadcasts reach
    /// every socket on the port, but each unicast reaches only one of them:
    /// on Linux the one that bound last. That can be another program that
    /// takes this transport's replies, registrations and forwards. Two stacks
    /// on one host are better served each on its own address with
    /// [`set_share_port_by_address`](Self::set_share_port_by_address), or
    /// each on its own port. Two B/IP-M members on one host at one unicast
    /// port also need sharing on, each; members on different ports don't.
    ///
    /// It changes nothing for a port the OS picks, which is never shared
    /// (#892), or in per-address mode. On macOS and the BSDs the claim stops
    /// another wildcard bind, and a plain bind of any address, but a socket
    /// that sets `SO_REUSEADDR` may still bind a specific address on the port
    /// and then gets the unicast sent there: they have no option to refuse
    /// it. On Linux a B/IP-M node that starts with its group on its own port
    /// binds its interface address instead of `0.0.0.0`, with broadcast
    /// listeners as [`set_share_port_by_address`](Self::set_share_port_by_address)
    /// does, so the group's listener can bind the port too and every
    /// squatter is refused. A group activated later on the port of a node
    /// bound to `0.0.0.0` turns on `SO_REUSEPORT` for it: from then until the
    /// node restarts (which takes the per-address shape), a socket of the
    /// same user that sets `SO_REUSEPORT` can bind the port and share its
    /// unicast. See `docs/rust-api.md` (Claiming the port).
    pub fn set_share_wildcard_port(&mut self, enabled: bool) {
        self.share_wildcard_port = enabled;
    }

    /// Bind the interface address itself, so transports on other addresses
    /// of this host can share the port, each getting only its own unicast
    /// (#1538). Off by default: the transport binds `0.0.0.0`. Call it before
    /// `start()`, which fails unless the interface is an explicit address and
    /// the port is nonzero.
    ///
    /// In this mode every send leaves from the interface address. On Linux,
    /// macOS and the BSDs, broadcasts arrive on separate receive-only
    /// listeners, and datagrams go up in the order the kernel stamped them
    /// on arrival across those sockets (#1546): a unicast that depends on a
    /// broadcast sent just before it is handled after it; datagrams without
    /// distinct stamps go up in the order read. The listeners keep only
    /// broadcasts that arrived on the interface's own link, as looked up at
    /// start: restart the transport after its interface changes. The
    /// configured broadcast address must be the interface's subnet broadcast
    /// or 255.255.255.255 (a loopback interface may also name itself), or
    /// `start()` fails. On
    /// Windows one socket claims the address with `SO_EXCLUSIVEADDRUSE`; a
    /// socket of the same user already bound to the wildcard address on that
    /// port without claiming it
    /// ([`set_share_wildcard_port`](Self::set_share_wildcard_port)) doesn't
    /// stop it, and under Windows' strong host model a multihomed BBMD reaches a
    /// peer only through the interface it is bound to. See
    /// `docs/rust-api.md` (BIP section) for each OS.
    pub fn set_share_port_by_address(&mut self, enabled: bool) {
        self.share_port_by_address = enabled;
    }

    /// This transport's frame tap: every BVLL message it sends and
    /// receives, BVLC results, registrations and forwarded NPDUs included,
    /// for whoever subscribes. See [`crate::tap`].
    pub fn frame_tap(&self) -> &FrameTap {
        &self.frame_tap
    }

    /// Check that this transport could start now: bind the sockets
    /// [`start`](crate::port::TransportPort::start) would, with the same
    /// options, and release them. Another socket can still take the port
    /// before the real start, which stays authoritative.
    /// With per-address mode it lists the host's interfaces, which can take
    /// a moment on a host with many adapters.
    pub fn check_bind(&self) -> Result<(), Error> {
        probe_interface(self.interface)?;
        let mut plan = self.bind_plan();
        if plan.per_address() {
            plan.local = looked_up(
                plan.interface,
                crate::local_addresses::interface_of(plan.interface),
            );
        }
        bind(plan).map(drop).map_err(Error::Transport)
    }

    /// The host's local IPv4 addresses and its local address toward the
    /// default route, as a wildcard bind reads them. A wildcard bind accepts
    /// unicast only to a listed address, so this fails when the addresses
    /// cannot be listed or none is usable. A listing error keeps its kind.
    pub(super) async fn wildcard_local_ipv4(
        &self,
    ) -> Result<(Vec<Ipv4Addr>, Option<Ipv4Addr>), Error> {
        const ADVICE: &str = "bind an explicit interface address instead";
        let (ips, route_ip) = self.host_ipv4().await.map_err(|e| {
            Error::Transport(io::Error::new(
                e.kind(),
                format!(
                    "a B/IP transport bound to 0.0.0.0 could not list the host's IPv4 \
                     addresses ({e}); {ADVICE}"
                ),
            ))
        })?;
        if ips.is_empty() {
            return Err(Error::Transport(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!(
                    "a B/IP transport bound to 0.0.0.0 found no usable IPv4 address on this \
                     host; {ADVICE}"
                ),
            )));
        }
        Ok((ips, route_ip))
    }

    /// The listed IPv4 addresses and the default-route address, or what a
    /// test injected. Listing adapters can be slow on a host with many
    /// virtual adapters, so it runs on a blocking thread.
    async fn host_ipv4(&self) -> io::Result<(Vec<Ipv4Addr>, Option<Ipv4Addr>)> {
        #[cfg(test)]
        if let Some(injected) = &self.local_ipv4_for_test {
            return match injected {
                Ok(local) => Ok(local.clone()),
                Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
            };
        }
        tokio::task::spawn_blocking(|| {
            Ok((
                crate::local_addresses::ipv4()?,
                crate::local_addresses::route_ipv4(),
            ))
        })
        .await
        .map_err(io::Error::other)?
    }

    /// What `start()` binds. In the per-address shape it looks the interface
    /// up on a blocking thread.
    pub(super) async fn bind_plan_for_start(&self) -> BindPlan {
        let mut plan = self.bind_plan();
        if plan.per_address() {
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
            requested_port: self.requested_port,
            share_wildcard: self.share_wildcard_port,
            by_address: self.share_port_by_address,
            broadcast: self.broadcast_address,
            local: None,
            multicast: self.multicast.config(),
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
    if !plan.per_address() {
        let role = wildcard_role(&plan);
        let primary = bound(role, Ipv4Addr::UNSPECIFIED).map_err(|e| held(&plan, role, e))?;
        let group = join_group(&plan, &primary)?;
        return Ok(BoundSockets {
            primary,
            listeners: Vec::new(),
            group,
        });
    }
    if plan.by_address && plan.multicast.is_some() {
        // A member takes no subnet broadcasts, which per-address mode's
        // listeners are for, and its group needs a listener of its own.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a B/IP multicast node can't also share its port by address; give each node on \
             the host its own port and the group a shared one",
        ));
    }
    if plan.interface.is_unspecified() || !plan.requested_port {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a B/IP transport shares its port by address only with an explicit interface \
             address and a nonzero port",
        ));
    }
    if plan.by_address {
        check_broadcast(&plan)?;
    }
    let primary = bound(SocketRole::Address, plan.interface)
        .map_err(|e| held(&plan, SocketRole::Address, e))?;
    let mut listeners = Vec::new();
    if plan.by_address {
        for ip in listener_addresses(plan.interface, plan.broadcast) {
            let listener = bound(SocketRole::BroadcastListener, ip).map_err(|e| {
                let port = plan.port;
                let context = format!("could not bind the B/IP broadcast listener to {ip}:{port}");
                super::listener_error::listener_error(context, e)
            })?;
            listeners.push(listener);
        }
    } else {
        for ip in group_shape_listener_addresses(&plan) {
            match bound(SocketRole::BroadcastListener, ip) {
                Ok(listener) => listeners.push(listener),
                Err(error) => tracing::warn!(
                    %ip,
                    port = plan.port,
                    %error,
                    "Could not bind a B/IP broadcast listener; the multicast node goes on \
                     without it"
                ),
            }
        }
    }
    let group = join_group(&plan, &primary)?;
    Ok(BoundSockets {
        primary,
        listeners,
        group,
    })
}

/// Where a Linux B/IP-M node's group-driven per-address shape listens for
/// broadcasts (module docs, #2366): what its `0.0.0.0` socket heard before,
/// 255.255.255.255 and the interface's own subnet broadcast from the netmask
/// the host reports, whatever broadcast address is configured. Without a
/// netmask it listens on 255.255.255.255 alone. Neither case refuses the
/// start: an unknown netmask, or a configured broadcast address that is
/// neither of those, is logged at warn and the start goes on.
fn group_shape_listener_addresses(plan: &BindPlan) -> Vec<Ipv4Addr> {
    let (interface, broadcast) = (plan.interface, plan.broadcast);
    let subnet = plan
        .local
        .and_then(|local| local.subnet_broadcast(interface));
    if subnet.is_none() {
        tracing::warn!(
            %interface,
            "No netmask reported for the B/IP multicast node's interface; listening for \
             limited broadcasts only"
        );
    }
    let usual = broadcast.is_broadcast()
        || Some(broadcast) == subnet
        || (broadcast == interface && interface.is_loopback());
    if !usual && subnet.is_some() {
        tracing::warn!(
            %broadcast,
            ?subnet,
            "Configured broadcast for this multicast node matches neither its subnet nor \
             255.255.255.255; kept as configured"
        );
    }
    let directed = subnet.filter(|subnet| !subnet.is_broadcast() && *subnet != interface);
    directed.into_iter().chain([Ipv4Addr::BROADCAST]).collect()
}

/// How the wildcard socket binds its port (module docs).
pub(super) fn wildcard_role(plan: &BindPlan) -> SocketRole {
    if !plan.requested_port {
        SocketRole::ClaimedWildcard
    } else if plan.share_wildcard {
        SocketRole::SharedWildcard
    } else {
        SocketRole::ClaimedWildcard
    }
}

/// A primary bind's error: on a requested port that another socket holds,
/// it says so and names the opt-in, and keeps the OS error as its source,
/// so its OS code survives (#2231).
fn held(plan: &BindPlan, role: SocketRole, e: io::Error) -> io::Error {
    // Windows refuses a bind beside a socket that holds the port with
    // `SO_EXCLUSIVEADDRUSE` as WSAEACCES.
    let held = e.kind() == io::ErrorKind::AddrInUse
        || (cfg!(windows) && e.kind() == io::ErrorKind::PermissionDenied);
    if !plan.requested_port || !held {
        return e;
    }
    let port = plan.port;
    let context = if role == SocketRole::SharedWildcard {
        format!("UDP port {port} is in use on this host by a socket that refuses sharing")
    } else if role == SocketRole::Address {
        let ip = plan.interface;
        format!("UDP port {port} on {ip} is in use on this host")
    } else {
        format!(
            "UDP port {port} is in use on this host, and a B/IP transport doesn't share its \
             port by default; stop the other program, use another port, or share the port \
             (share_wildcard_port, or --share-port in the CLI), after which unicast reaches \
             only one of the sockets on it"
        )
    };
    super::listener_error::listener_error(context, e)
}

/// Set up a B/IP-M node's group, if it has one (multicast.rs): the primary
/// socket sends to it from the interface, and a listener joined to it takes
/// its broadcasts, or on Windows the primary joins it itself.
fn join_group(plan: &BindPlan, primary: &socket2::Socket) -> io::Result<Option<socket2::Socket>> {
    let Some(multicast) = plan.multicast else {
        return Ok(None);
    };
    // `set_multicast` and the start checked the group against the interface
    // and port.
    let interface = plan.interface;
    super::multicast::configure_sender(primary.into(), interface, multicast.ttl())?;
    #[cfg(not(windows))]
    return super::multicast::group_listener(multicast.endpoint(plan.port), interface, false)
        .map(Some);
    #[cfg(windows)]
    {
        primary.join_multicast_v4(&multicast.group(), &interface)?;
        Ok(None)
    }
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
    /// The transport's tap, for every datagram this socket sends or reads.
    tap: FrameTap,
    /// Where a test's sends go instead of the network.
    #[cfg(test)]
    sink: Option<super::send_sink::SendSink>,
    /// Makes the next send's first try report `WouldBlock`, so a test can
    /// drive [`SendTo`]'s boxed wait on a real socket.
    #[cfg(test)]
    pub(super) block_next_try: std::sync::atomic::AtomicBool,
}

impl BipSocket {
    pub(super) fn new(socket: UdpSocket, lease: Option<Arc<()>>) -> Self {
        Self {
            socket,
            _network_port_lease: lease,
            tap: FrameTap::default(),
            #[cfg(test)]
            sink: None,
            #[cfg(test)]
            block_next_try: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// This socket, reporting its datagrams to `tap`.
    pub(super) fn tapped(self, tap: FrameTap) -> Self {
        Self { tap, ..self }
    }

    /// Report a datagram this socket read from `peer`.
    pub(super) fn tap_inbound(&self, datagram: &[u8], peer: SocketAddr) {
        let peer = || TapPeer::Udp(peer);
        self.tap
            .record(TapDirection::Inbound, DataLink::Bip, peer, datagram);
    }

    /// A socket whose sends go to `sink`, for tests.
    #[cfg(test)]
    pub(super) fn recording(socket: UdpSocket, sink: super::send_sink::SendSink) -> Self {
        Self {
            sink: Some(sink),
            ..Self::new(socket, None)
        }
    }

    /// Send one datagram. Every send of the transport goes through here,
    /// and the tap sees each one sent.
    #[cfg(not(test))]
    pub(super) fn send_to<'a>(&'a self, buf: &'a [u8], target: SocketAddrV4) -> SendTo<'a> {
        self.send_datagram(OutgoingDatagram::whole(buf), target)
    }

    /// [`Self::send_to`] for a datagram in parts.
    #[cfg(not(test))]
    pub(super) fn send_datagram<'a>(
        &'a self,
        datagram: OutgoingDatagram<'a>,
        target: SocketAddrV4,
    ) -> SendTo<'a> {
        #[cfg(feature = "test-support")]
        super::send_observer::attempted(&self.socket);
        self.send_to_socket(datagram, target)
    }

    /// [`Self::send_to`] for this crate's tests, which may send to a sink
    /// instead, yielding first on request. It is an `async fn`, so a test
    /// build's send futures are larger than a release build's; the future
    /// size that #1713 pins is measured in another crate's tests, against
    /// the [`SendTo`] the non-test build returns. A send without a sink is
    /// that same [`SendTo`].
    #[cfg(test)]
    pub(super) async fn send_to(&self, buf: &[u8], target: SocketAddrV4) -> io::Result<usize> {
        self.send_datagram(OutgoingDatagram::whole(buf), target)
            .await
    }

    /// [`Self::send_to`] for a datagram in parts, in this crate's tests.
    #[cfg(test)]
    pub(super) async fn send_datagram(
        &self,
        datagram: OutgoingDatagram<'_>,
        target: SocketAddrV4,
    ) -> io::Result<usize> {
        super::send_observer::attempted(&self.socket);
        if let Some(sink) = &self.sink {
            if sink.yields() {
                tokio::task::yield_now().await;
            }
            let sent = sink.send(&datagram.joined(), target);
            return self.recorded(sent, &datagram, target);
        }
        self.send_to_socket(datagram, target).await
    }

    fn send_to_socket<'a>(
        &'a self,
        datagram: OutgoingDatagram<'a>,
        target: SocketAddrV4,
    ) -> SendTo<'a> {
        SendTo {
            socket: self,
            datagram,
            target,
            blocked: None,
            done: false,
        }
    }

    /// One try at sending `datagram`, without waiting for the socket.
    fn try_send(&self, datagram: &OutgoingDatagram<'_>, target: SocketAddrV4) -> io::Result<usize> {
        #[cfg(test)]
        if self
            .block_next_try
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        datagram.try_send(&self.socket, target)
    }

    /// `sent`, after the tap sees `datagram` if it was sent. A datagram in
    /// parts is put in one buffer only when the tap has a subscriber.
    fn recorded(
        &self,
        sent: io::Result<usize>,
        datagram: &OutgoingDatagram<'_>,
        target: SocketAddrV4,
    ) -> io::Result<usize> {
        if sent.is_ok() && self.tap.is_active() {
            let peer = || TapPeer::Udp(target.into());
            let record = |bytes: &[u8]| {
                self.tap
                    .record(TapDirection::Outbound, DataLink::Bip, peer, bytes)
            };
            if datagram.head().is_empty() {
                record(datagram.payload());
            } else {
                record(&datagram.joined());
            }
        }
        sent
    }
}

/// Sending one datagram on a [`BipSocket`], then recording it.
///
/// A datagram almost always goes at once, so the first poll tries the send,
/// and only a socket that would block waits, in a boxed `UdpSocket::send_to`.
/// That future resolves the address before it waits and is about 400 bytes;
/// inline, every send future above this one would carry it, up to a
/// server's request handler, which is allocated for each request (#1713).
/// The socket's `poll_send_to` would be as small, but it keeps one waker,
/// and the receive loop, node driver and fanout worker all send on this
/// socket.
pub(super) struct SendTo<'a> {
    socket: &'a BipSocket,
    datagram: OutgoingDatagram<'a>,
    target: SocketAddrV4,
    blocked: Option<BlockedSend<'a>>,
    done: bool,
}

type BlockedSend<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + Send + 'a>>;

impl std::future::Future for SendTo<'_> {
    type Output = io::Result<usize>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = &mut *self;
        assert!(!this.done, "a B/IP send polled after it finished");
        let (socket, datagram, target) = (this.socket, this.datagram, this.target);
        let sent = match &mut this.blocked {
            None => {
                // The try spends the task's cooperative budget as the
                // awaited send did, so a task that only sends (the BBMD
                // fanout worker) still yields to its neighbours.
                let coop = std::task::ready!(tokio::task::coop::poll_proceed(cx));
                match socket.try_send(&datagram, target) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        // The boxed send keeps its own budget.
                        drop(coop);
                        let mut blocked: BlockedSend<'_> = if datagram.head().is_empty() {
                            Box::pin(socket.socket.send_to(datagram.payload(), target))
                        } else {
                            Box::pin(send_when_writable(&socket.socket, datagram, target))
                        };
                        let polled = blocked.as_mut().poll(cx);
                        this.blocked = Some(blocked);
                        std::task::ready!(polled)
                    }
                    sent => {
                        coop.made_progress();
                        sent
                    }
                }
            }
            Some(blocked) => std::task::ready!(blocked.as_mut().poll(cx)),
        };
        this.done = true;
        this.blocked = None;
        std::task::Poll::Ready(socket.recorded(sent, &datagram, target))
    }
}

/// Send a datagram in parts once `socket` is writable.
async fn send_when_writable(
    socket: &UdpSocket,
    datagram: OutgoingDatagram<'_>,
    target: SocketAddrV4,
) -> io::Result<usize> {
    loop {
        socket.writable().await?;
        match datagram.try_send(socket, target) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            sent => return sent,
        }
    }
}

impl Deref for BipSocket {
    type Target = UdpSocket;
    fn deref(&self) -> &Self::Target {
        &self.socket
    }
}
