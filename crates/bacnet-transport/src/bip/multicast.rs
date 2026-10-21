//! B/IP multicast for a node (Annex J.8, #941): a B/IP-M node sends its
//! broadcasts to an IPv4 multicast group, which stands in for its subnet's
//! broadcast address, and takes broadcasts only from that group.
//!
//! **A BBMD for the group.** A BBMD with a group serves it (Annex J.8.3):
//! it joins as a member does, and `bacnet-bip-core` uses the group where a
//! plain BBMD uses its subnet's broadcast address. A broadcast that comes
//! through the group goes on to the BDT and the foreign devices, a peer's
//! Forwarded-NPDU and a foreign device's Distribute-Broadcast-To-Network go
//! to the group too, and its own BDT row must be two-hop, which `start()`
//! checks. A network that mixes B/IP-M and plain B/IP devices has exactly
//! one BBMD for the group, and the two kinds of device don't share an IP
//! subnet (Annex J.8.1); neither is something one node can check. Like a
//! member it keeps its IP subnet's directed broadcast as a group address,
//! so a datagram from it, or a Forwarded-NPDU naming it as the origin,
//! is still dropped: a BBMD that answered one would reach the whole
//! subnet. It may also run behind a NAT router (#940): both want a two-hop
//! own row, and the NAT rules for the origin of what it forwards don't
//! change.
//!
//! **Configuration.** [`BipMulticast`] names the group (class D, outside
//! the 224.0.0.0/24 local network control block), the
//! multicast TTL (1 by default, so the group stays on the local network:
//! raise it for a group that spans IP routers, which must forward
//! multicast) and the group's UDP port (the transport's own by default).
//! A node in multicast mode needs an explicit interface: the group is
//! joined there and multicasts leave from it. There is no wildcard bind and
//! no route lookup to pick one.
//!
//! **Sockets.** The primary socket sends to the group from the interface
//! (`IP_MULTICAST_IF`), with the TTL, and with loop on, so several stacks
//! on one host can share a group: each drops its own looped-back copy, as
//! it drops any datagram from its own address. That needs the copy to come
//! back from the interface address: bind the interface's primary address,
//! since with a secondary (alias) address some systems send multicast from
//! the primary one, and the node would then take its own broadcasts as a
//! peer's. The primary socket never takes the group's traffic itself: on
//! Linux it turns `IP_MULTICAST_ALL` off, so it gets no group another
//! socket joined, and macOS and the BSDs only deliver a group to a socket
//! that joined it. On Linux, macOS and the BSDs a receive-only listener
//! binds the group address and port with `SO_REUSEADDR` (and `SO_REUSEPORT`
//! on macOS and the BSDs) and joins the group on the interface. Each stack
//! on the host has its own listener and the kernel hands each a copy; the
//! receive loop reads it beside the primary socket, in arrival order
//! (#1546). Another program holding the group's port without those options
//! (bound to `0.0.0.0` on it, say) makes the listener's bind fail, and the
//! start or activation fails with it. Windows can't bind a group address,
//! so there the primary socket joins the group itself and the group's port
//! must be the transport's own, which also means two members can't share
//! one Windows host; it reports the group delivery in the receive flags.
//! Windows multicast is untested.
//!
//! The group is left when its socket closes: when the transport stops, or
//! when an activation replaces or ends the group.
//!
//! **Network Port.** A port that can use multicast serves
//! BACnet_IP_Multicast_Address (Clause 12.56.26): the group, or 0.0.0.0
//! while off. A written group waits for a ReinitializeDevice, which joins
//! it at run time through [`MulticastControl`]: a new listener is bound and
//! joined before the link changes (the primary socket's multicast send
//! options may be set first), then the link moves to the group and the
//! receive loop takes the listener in place of the old one. A later start
//! uses the group activated last. A BBMD serves the row read-only, started
//! or not: its group and its own BDT row go together under the all-ones
//! rule, and the Network Port activates its BBMD rows and its group through
//! separate views, so a joint change could take effect half way (one
//! activation for both is #1963).
//!
//! Lock order: the object database (held by the caller), then the
//! control's lock, then the node's. No lock here is held across an await.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use bacnet_bip_core::LocalSubnet;
use bacnet_types::bip_port::IpMulticast;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::Notify;
use tracing::{info, warn};

use super::ingress::Listener;
use super::node::NodeHandle;
use crate::tap::FrameTap;

/// A B/IP-M node's group (Annex J.8): where its broadcasts go and come
/// from. See [`BipTransport::set_multicast`](super::BipTransport::set_multicast).
///
/// ```
/// use std::net::Ipv4Addr;
/// use bacnet_transport::bip::BipMulticast;
///
/// // A group whose members sit on two IP subnets one router apart.
/// let group = BipMulticast::new(Ipv4Addr::new(239, 255, 66, 1))?.with_ttl(2)?;
/// assert_eq!((group.ttl(), group.port()), (2, None));
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BipMulticast {
    group: Ipv4Addr,
    ttl: u8,
    port: Option<u16>,
}

impl BipMulticast {
    /// The default multicast TTL: the group's datagrams stay on the local
    /// network.
    pub const DEFAULT_TTL: u8 = 1;

    /// The group `group` at the transport's own UDP port, with
    /// [`DEFAULT_TTL`](Self::DEFAULT_TTL). Fails unless `group` is a class D
    /// (multicast) address outside 224.0.0.0/24, the block reserved for
    /// local network control.
    pub fn new(group: Ipv4Addr) -> Result<Self, Error> {
        if !group.is_multicast() {
            return Err(config(format!(
                "a B/IP multicast group must be a class D address (224.0.0.0/4), not {group}"
            )));
        }
        if group.octets()[..3] == [224, 0, 0] {
            // 224.0.0.0/24 is the local network control block (RFC 5771):
            // routing and discovery protocols, never forwarded.
            return Err(config(format!(
                "a B/IP multicast group can't be in 224.0.0.0/24, reserved for local network \
                 control; got {group}"
            )));
        }
        Ok(Self {
            group,
            ttl: Self::DEFAULT_TTL,
            port: None,
        })
    }

    /// This group with multicast TTL `ttl`, 1 to 255. Raise it above 1 for
    /// a group whose members are more IP routers apart than that.
    pub fn with_ttl(self, ttl: u8) -> Result<Self, Error> {
        if ttl == 0 {
            return Err(config(
                "a B/IP multicast TTL must be 1 to 255; 0 would keep it on this host".into(),
            ));
        }
        Ok(Self { ttl, ..self })
    }

    /// This group at UDP port `port` rather than the transport's own: the
    /// port every member sends its broadcasts to. Members on one host, at
    /// one IP address, need their own unicast ports and so this one. On
    /// Windows the group's port must equal the transport's port, or
    /// `set_multicast` refuses it.
    pub fn with_port(self, port: u16) -> Result<Self, Error> {
        if port == 0 {
            return Err(config(
                "a B/IP multicast group's port must be nonzero".into(),
            ));
        }
        Ok(Self {
            port: Some(port),
            ..self
        })
    }

    /// The group address.
    pub fn group(&self) -> Ipv4Addr {
        self.group
    }

    /// The multicast TTL.
    pub fn ttl(&self) -> u8 {
        self.ttl
    }

    /// The group's UDP port, or `None` for the transport's own.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// The group's B/IP address for a transport bound to `own_port`.
    pub(super) fn endpoint(&self, own_port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(self.group, self.port.unwrap_or(own_port))
    }
}

impl fmt::Display for BipMulticast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(port) => write!(f, "{}:{port} (TTL {})", self.group, self.ttl),
            None => write!(f, "{} (TTL {})", self.group, self.ttl),
        }
    }
}

/// Where a link sends its broadcasts, given `subnet`, the configured
/// broadcast IP at the bound port: with `multicast`, its group at the
/// group's port, `subnet` still counting as a group address beside it;
/// without, `subnet` itself.
pub(super) fn subnet(multicast: Option<BipMulticast>, subnet: SocketAddrV4) -> LocalSubnet {
    match multicast {
        Some(multicast) => {
            LocalSubnet::member(multicast.endpoint(subnet.port()), Some(*subnet.ip()))
        }
        None => LocalSubnet::new(subnet),
    }
}

fn config(message: String) -> Error {
    Error::Encoding(message)
}

/// Why a node bound to `interface` at `transport_port` (0 for a port the
/// OS picks) can't use `multicast`: checked when it is configured, at
/// start and for a group written to the Network Port.
fn refusal(interface: Ipv4Addr, transport_port: u16, multicast: &BipMulticast) -> Option<String> {
    if interface.is_unspecified() {
        return Some(
            "B/IP multicast needs an explicit interface address to join the group on; \
             0.0.0.0 is refused"
                .into(),
        );
    }
    if cfg!(windows) && multicast.port.is_some_and(|port| port != transport_port) {
        return Some("on Windows a B/IP multicast group's port must be the transport's own".into());
    }
    if multicast.port.is_none() && transport_port == 0 {
        return Some(
            "a B/IP multicast group needs a port: give the transport a nonzero port or the \
             group its own (BipMulticast::with_port)"
                .into(),
        );
    }
    None
}

/// A new group listener for the receive loop, or none: the hand-off from an
/// activation to the loop that reads it. Windows has none: its primary
/// socket joins the group, so its receive loop takes no hand-off.
#[derive(Default)]
pub(super) struct GroupSwap {
    next: Mutex<Option<Option<Listener>>>,
    ready: Notify,
}

impl GroupSwap {
    /// Hand the loop `listener`, opened and configured already, replacing
    /// the one it reads; `None` closes it.
    #[cfg_attr(windows, allow(dead_code))]
    fn offer(&self, listener: Option<Listener>) {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) = Some(listener);
        self.ready.notify_one();
    }

    /// The next listener offered. Cancel-safe: the offer is taken in the
    /// same poll that sees it.
    pub(super) async fn next(&self) -> Option<Listener> {
        loop {
            self.ready.notified().await;
            if let Some(listener) = self
                .next
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                return listener;
            }
        }
    }
}

/// The primary socket's multicast options: send from `interface` with
/// `ttl`, loop on, and on Linux no group it didn't join.
pub(super) fn configure_sender(
    socket: socket2::SockRef<'_>,
    interface: Ipv4Addr,
    ttl: u8,
) -> io::Result<()> {
    socket.set_multicast_if_v4(&interface)?;
    socket.set_multicast_ttl_v4(u32::from(ttl))?;
    socket.set_multicast_loop_v4(true)?;
    #[cfg(target_os = "linux")]
    socket.set_multicast_all_v4(false)?;
    Ok(())
}

/// A receive-only socket on `group`'s address and port, joined to it on
/// `interface`.
#[cfg(not(windows))]
pub(super) fn group_listener(
    group: SocketAddrV4,
    interface: Ipv4Addr,
) -> io::Result<socket2::Socket> {
    let socket = super::socket::udp_socket(super::socket::SocketRole::GroupListener)?;
    socket.bind(&group.into()).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("could not bind the B/IP multicast listener to {group} ({e})"),
        )
    })?;
    socket
        .join_multicast_v4(group.ip(), &interface)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "could not join B/IP multicast group {} on {interface} ({e})",
                    group.ip()
                ),
            )
        })?;
    #[cfg(target_os = "linux")]
    socket.set_multicast_all_v4(false)?;
    Ok(socket)
}

/// What the control's lock guards.
struct ControlState {
    /// The group in effect, and the one the next start uses.
    config: Option<BipMulticast>,
    /// The TTL and port a group activated through the Network Port gets:
    /// the last configured.
    ttl: u8,
    port: Option<u16>,
    /// The running transport, while one runs.
    running: Option<Running>,
}

/// What an activation changes in a running transport. On Windows only the
/// node and the subnet are read: the primary socket joins the group there.
#[cfg_attr(windows, allow(dead_code))]
pub(super) struct Running {
    pub(super) node: Weak<NodeHandle>,
    /// The bound UDP port.
    pub(super) port: u16,
    /// Where broadcasts go without a group: the configured broadcast
    /// address at the bound port.
    pub(super) subnet: SocketAddrV4,
    /// The receive loop's listener hand-off (not on Windows, nor for a
    /// BBMD, whose group doesn't change at run time).
    pub(super) swap: Option<Arc<GroupSwap>>,
    /// The transport is a BBMD: its group is read-only (module docs).
    pub(super) bbmd: bool,
    /// What a new listener keeps and reports to: the Network Port lease and
    /// the transport's frame tap.
    pub(super) lease: Option<Arc<()>>,
    pub(super) tap: FrameTap,
}

/// One transport's multicast group, shared with the Network Port object it
/// is lent to: the group in effect and, while the transport runs, how to
/// move it to another.
pub(crate) struct MulticastControl {
    state: Mutex<ControlState>,
    /// The transport's configured interface: the group is joined there.
    interface: Ipv4Addr,
    /// The transport's configured port, 0 when the OS picks it: a group at
    /// a picked port couldn't share it with the other members.
    configured_port: u16,
    /// The transport is a BBMD, configured or started: its group is
    /// read-only, running or not (module docs). The transport sets it each
    /// time it lends the view.
    bbmd: std::sync::atomic::AtomicBool,
    /// Makes the next listener an activation opens fail, for tests. Not on
    /// Windows, which opens no listener.
    #[cfg(all(test, not(windows)))]
    pub(super) fail_next_listener: std::sync::atomic::AtomicBool,
}

impl MulticastControl {
    pub(super) fn new(interface: Ipv4Addr, configured_port: u16) -> Self {
        Self {
            state: Mutex::new(ControlState {
                config: None,
                ttl: BipMulticast::DEFAULT_TTL,
                port: None,
                running: None,
            }),
            interface,
            configured_port,
            bbmd: Default::default(),
            #[cfg(all(test, not(windows)))]
            fail_next_listener: Default::default(),
        }
    }

    /// Why this transport can't use `multicast`, if it can't.
    pub(super) fn refusal(&self, multicast: &BipMulticast) -> Option<String> {
        refusal(self.interface, self.configured_port, multicast)
    }

    fn lock(&self) -> MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The group in effect.
    pub(super) fn config(&self) -> Option<BipMulticast> {
        self.lock().config
    }

    /// Configure the group, or none, for the next start.
    pub(super) fn set(&self, multicast: Option<BipMulticast>) {
        let mut state = self.lock();
        if let Some(multicast) = multicast {
            state.ttl = multicast.ttl;
            state.port = multicast.port;
        }
        state.config = multicast;
    }

    /// Say whether the transport is a BBMD, configured or started.
    pub(super) fn set_bbmd(&self, bbmd: bool) {
        self.bbmd.store(bbmd, std::sync::atomic::Ordering::Relaxed);
    }

    /// Hand the control the running transport, for activations.
    pub(super) fn attach(&self, running: Running) {
        self.lock().running = Some(running);
    }

    /// Forget the running transport: it stopped.
    pub(super) fn detach(&self) {
        self.lock().running = None;
    }

    /// The running node: attached, not stopped, and not a BBMD's, whose
    /// group stays as it started.
    fn running(state: &ControlState) -> Option<(Arc<NodeHandle>, &Running)> {
        let running = state.running.as_ref().filter(|running| !running.bbmd)?;
        let node = running.node.upgrade().filter(|node| !node.is_stopped())?;
        Some((node, running))
    }

    /// The group a written value names, as it would be activated: `None`
    /// for 0.0.0.0. Refused with the WriteProperty error a client gets.
    fn written(&self, state: &ControlState, group: [u8; 4]) -> Result<Option<BipMulticast>, Error> {
        let group = Ipv4Addr::from(group);
        if group.is_unspecified() {
            return Ok(None);
        }
        let mut multicast =
            BipMulticast::new(group).map_err(|_| protocol(ErrorCode::VALUE_OUT_OF_RANGE))?;
        multicast.ttl = state.ttl;
        multicast.port = state.port;
        if self.refusal(&multicast).is_some() {
            return Err(protocol(ErrorCode::VALUE_OUT_OF_RANGE));
        }
        Ok(Some(multicast))
    }
}

/// The sockets an activation readied for its new group, before the link
/// moves: on Linux, macOS and the BSDs the new group's listener, opened and
/// configured for the receive loop; on Windows the group the primary
/// socket joined.
#[cfg(not(windows))]
type Prepared = Option<Listener>;
#[cfg(windows)]
type Prepared = Option<Ipv4Addr>;

impl MulticastControl {
    /// Ready the sockets for `next`, changing nothing the node reads yet:
    /// every step that can fail runs here. On Linux, macOS and the BSDs the
    /// sender's options and a listener joined to the group, wrapped for the
    /// receive loop; the old group is left when its listener closes. The
    /// sender's options (interface, TTL, loop) may already be set on the
    /// primary socket when a later step fails, and stay set: they only
    /// apply to multicast sends, which a node without a group makes none of.
    #[cfg(not(windows))]
    fn prepare(
        &self,
        node: &NodeHandle,
        running: &Running,
        next: Option<BipMulticast>,
    ) -> io::Result<Prepared> {
        let Some(next) = next else {
            return Ok(None);
        };
        #[cfg(test)]
        if self
            .fail_next_listener
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::Error::other("injected listener failure"));
        }
        // Wrapping the socket registers it with the runtime, which an
        // activation outside one couldn't do.
        tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        configure_sender(
            socket2::SockRef::from(&***node.socket()),
            self.interface,
            next.ttl,
        )?;
        let socket = group_listener(next.endpoint(running.port), self.interface)?;
        Listener::open_group(socket, running.lease.clone(), &running.tap).map(Some)
    }

    /// Ready the sockets for `next`: on Windows the primary socket joins
    /// the new group. The old one is left once the link has moved
    /// ([`commit`](Self::commit)).
    #[cfg(windows)]
    fn prepare(
        &self,
        node: &NodeHandle,
        _running: &Running,
        next: Option<BipMulticast>,
    ) -> io::Result<Prepared> {
        let Some(next) = next else {
            return Ok(None);
        };
        let primary = socket2::SockRef::from(&***node.socket());
        configure_sender(
            socket2::SockRef::from(&***node.socket()),
            self.interface,
            next.ttl,
        )?;
        primary.join_multicast_v4(&next.group, &self.interface)?;
        Ok(Some(next.group))
    }

    /// The link moved: hand the new listener over, or on Windows leave the
    /// old group.
    #[cfg_attr(windows, allow(unused_variables))]
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn commit(
        &self,
        node: &NodeHandle,
        running: &Running,
        prepared: Prepared,
        old: Option<BipMulticast>,
    ) {
        // A listener only comes from a transport whose receive loop takes
        // the hand-off; without one it would close unread.
        #[cfg(not(windows))]
        debug_assert!(
            running.swap.is_some() || prepared.is_none(),
            "a group listener with no receive loop to hand it to"
        );
        #[cfg(not(windows))]
        if let Some(swap) = &running.swap {
            swap.offer(prepared);
        }
        #[cfg(windows)]
        if let Some(old) = old.filter(|old| Some(old.group) != prepared) {
            let primary = socket2::SockRef::from(&***node.socket());
            if let Err(error) = primary.leave_multicast_v4(&old.group, &self.interface) {
                warn!(%error, group = %old.group, "Could not leave the old B/IP multicast group");
            }
        }
    }

    /// The link refused to move: undo what [`prepare`](Self::prepare) did.
    /// A listener closes when dropped; on Windows the new group is left.
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn roll_back(&self, node: &NodeHandle, prepared: Prepared, old: Option<BipMulticast>) {
        #[cfg(windows)]
        if let Some(group) = prepared.filter(|group| old.map(|old| old.group) != Some(*group)) {
            let primary = socket2::SockRef::from(&***node.socket());
            let _ = primary.leave_multicast_v4(&group, &self.interface);
        }
    }
}

fn protocol(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code,
    }
}

impl IpMulticast for MulticastControl {
    fn group(&self) -> [u8; 4] {
        self.lock()
            .config
            .map_or([0; 4], |multicast| multicast.group.octets())
    }

    fn check(&self, group: [u8; 4]) -> Result<(), Error> {
        let state = self.lock();
        if Self::running(&state).is_none() {
            return Err(protocol(ErrorCode::WRITE_ACCESS_DENIED));
        }
        self.written(&state, group).map(drop)
    }

    fn activate(&self, group: [u8; 4]) -> Result<(), Error> {
        let mut state = self.lock();
        let next = self.written(&state, group)?;
        if next == state.config {
            return Ok(());
        }
        let Some((node, running)) = Self::running(&state) else {
            return Err(protocol(ErrorCode::WRITE_ACCESS_DENIED));
        };
        let old = state.config;
        let subnet = subnet(next, running.subnet);
        // Every step that can fail first, then the link, then the hand-off:
        // a failure leaves the old group in use, the link and the listener
        // as they were (the primary's multicast send options may be set).
        let prepared = self.prepare(&node, running, next).map_err(|error| {
            warn!(%error, "B/IP multicast group change failed; the old group stays in use");
            Error::Transport(error)
        })?;
        if let Err(error) = node.set_subnet(subnet) {
            self.roll_back(&node, prepared, old);
            return Err(error);
        }
        self.commit(&node, running, prepared, old);
        info!(group = ?next.map(|multicast| multicast.group), "B/IP multicast group changed");
        state.config = next;
        Ok(())
    }

    /// Not for a BBMD, running or not (module docs).
    fn can_change(&self) -> bool {
        !self.bbmd.load(std::sync::atomic::Ordering::Relaxed)
            && !self
                .lock()
                .running
                .as_ref()
                .is_some_and(|running| running.bbmd)
    }
}

impl super::BipTransport {
    /// Make this node a B/IP-M member of `multicast`'s group (Annex J.8), or,
    /// with `None`, not. Its broadcasts then go to the group as
    /// Original-Broadcast-NPDU, and it takes Original-Broadcast-NPDU and
    /// Forwarded-NPDU only from the group: never a subnet broadcast, since
    /// members and plain B/IP devices don't share a subnet (Annex J.8.1).
    /// Unicast is unchanged. Several stacks on one host can share a group:
    /// each needs its own unicast address or port, and each drops its own
    /// looped-back copies.
    ///
    /// Fails, changing nothing, on a transport bound to 0.0.0.0: the group
    /// is joined on the interface, which must be explicit. With no group
    /// port ([`BipMulticast::with_port`]) the transport's port must be
    /// nonzero. `start()` also refuses a member that is a foreign device
    /// too, or shares its port by address. A BBMD with a group serves it
    /// (Annex J.8.3, module docs): `start()` then refuses a BDT whose own
    /// row isn't two-hop. Call it before `start()`. A registered Network
    /// Port serves the group as BACnet_IP_Multicast_Address, which a
    /// ReinitializeDevice can change while a member runs; a later start
    /// keeps that change. A BBMD's row is read-only.
    pub fn set_multicast(&mut self, multicast: Option<BipMulticast>) -> Result<(), Error> {
        if let Some(refused) = multicast.and_then(|group| self.multicast.refusal(&group)) {
            return Err(config(refused));
        }
        self.multicast.set(multicast);
        Ok(())
    }

    /// The B/IP-M group in effect, if any: the one configured, or the one a
    /// Network Port activation moved to.
    pub fn multicast(&self) -> Option<BipMulticast> {
        self.multicast.config()
    }

    /// Whether this transport can use a B/IP-M group, as a member or as the
    /// BBMD that serves it, so it lends its group to the Network Port: an
    /// explicit interface, not a foreign device, not sharing its port by
    /// address, and a port for the group.
    pub(super) fn supports_multicast(&self) -> bool {
        let group_port = self.multicast.config().and_then(|group| group.port);
        !self.interface.is_unspecified()
            && !self.share_port_by_address
            && self.foreign_device.is_none()
            && (self.multicast.configured_port != 0 || group_port.is_some())
    }

    /// Where this link's broadcasts go: the multicast group at its port, or
    /// the configured broadcast address at the link's port.
    pub(super) fn broadcast_endpoint(&self) -> SocketAddrV4 {
        match self.multicast.config() {
            Some(multicast) => multicast.endpoint(self.port),
            None => SocketAddrV4::new(self.broadcast_address, self.port),
        }
    }

    /// Hand the multicast control the started node, so a Network Port
    /// activation can move its group, and return the listener hand-off for
    /// the receive loop: only for a transport that lends its group
    /// ([`supports_multicast`](Self::supports_multicast)), never for a BBMD,
    /// whose group doesn't change, and never on Windows, whose primary
    /// socket joins the group. Without one the receive loop waits on
    /// nothing extra.
    pub(super) fn attach_multicast(
        &self,
        node: &Arc<NodeHandle>,
        subnet: SocketAddrV4,
    ) -> Option<Arc<GroupSwap>> {
        if !self.supports_multicast() {
            self.multicast.detach();
            return None;
        }
        let bbmd = self.bbmd.is_some();
        let swap = (!cfg!(windows) && !bbmd).then(|| Arc::new(GroupSwap::default()));
        self.multicast.attach(Running {
            node: Arc::downgrade(node),
            port: subnet.port(),
            subnet,
            swap: swap.clone(),
            bbmd,
            lease: self.network_port_lease.clone(),
            tap: self.frame_tap.clone(),
        });
        swap
    }

    /// Refuse, before any I/O, a start of a member that can't be one.
    pub(super) fn check_multicast_start(
        &self,
        multicast: &BipMulticast,
        foreign: bool,
    ) -> Result<(), Error> {
        let refused = if foreign {
            Some(
                "a B/IP foreign device broadcasts through its BBMD, so it can't also be a \
                 multicast group member"
                    .to_string(),
            )
        } else if self.share_port_by_address {
            Some(
                "a B/IP multicast node can't also share its port by address; give each node \
                 on the host its own port and the group a shared one"
                    .to_string(),
            )
        } else {
            self.multicast.refusal(multicast)
        };
        refused.map_or(Ok(()), |refused| Err(config(refused)))
    }
}
