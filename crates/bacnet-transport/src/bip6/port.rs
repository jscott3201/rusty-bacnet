use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bip_port::{BipPortMode, ForeignRegistration};
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::bbmd_lookup::{BbmdLookupCounters, BbmdResolver, SystemResolver};
use crate::port::{Bip6Port, ReceivedNpdu, TransportPort};

use super::collision_probe::Probe;
use super::foreign_bbmd::Bip6ForeignBbmd;
use super::foreign_target::{Bip6ForeignStatus, Bip6ReleaseCounters, Cause, ForeignTarget};
use super::frame::encoded;
use super::receive::Receiver;
use super::receive_counters::{Bip6ReceiveCounters, ReceiveDrops};
use super::reply_limit::ReplyLimiter;
use super::resolver::{Driver, Resolver, UnresolvedSend};
use super::socket::{Bip6Socket, PortUse, Role};
use super::vmac_table::{generate_random_vmac, vmac_from_device_instance};
use super::{Bip6Address, Bip6Vmac, Bvll6, MAX_VMAC_RETRIES, encode_bvll6};

/// BACnet/IPv6 multicast group (link-local): FF02::BAC0.
pub const BACNET_IPV6_MULTICAST_LINK_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group (site-local): FF05::BAC0.
pub const BACNET_IPV6_MULTICAST_SITE_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group (organization-local): FF08::BAC0.
pub const BACNET_IPV6_MULTICAST_ORG_LOCAL: Ipv6Addr =
    Ipv6Addr::new(0xFF08, 0, 0, 0, 0, 0, 0, 0xBAC0);

/// BACnet/IPv6 multicast group -- alias for link-local (backward compatibility).
pub const BACNET_IPV6_MULTICAST: Ipv6Addr = BACNET_IPV6_MULTICAST_LINK_LOCAL;

/// Default BACnet/IPv6 port (same as BIP: 0xBAC0 = 47808).
pub const DEFAULT_BACNET6_PORT: u16 = 0xBAC0;

/// BACnet/IPv6 transport over UDP (Annex U).
///
/// The network layer addresses a B/IPv6 node by its VMAC (U.1.3), so the
/// transport's MAC is three octets: [`local_mac`](TransportPort::local_mac)
/// is this node's VMAC, a received NPDU's source MAC is its sender's (or a
/// Forwarded-NPDU's origin's), and
/// [`send_unicast`](TransportPort::send_unicast) takes the destination's.
/// The transport maps VMACs to B/IPv6 addresses itself, from the frames it
/// receives and by asking when it doesn't know (U.5). To reach a node known
/// only by its address, find its VMAC with
/// [`resolve_virtual_address`](Self::resolve_virtual_address).
pub struct Bip6Transport {
    interface: Ipv6Addr,
    port: u16,
    /// See [`Self::set_share_wildcard_port`].
    share_wildcard_port: bool,
    device_instance: Option<u32>,
    /// This node's VMAC as the network layer's MAC: zero before start and
    /// after stop.
    local_mac: [u8; 3],
    /// See [`Self::local_address`].
    pub(super) local_address: Option<SocketAddrV6>,
    pub(super) source_vmac: Bip6Vmac,
    pub(super) socket: Option<Arc<Bip6Socket>>,
    recv_task: Option<JoinHandle<()>>,
    /// The neighbour table and its resolutions (U.5), shared with the
    /// receive loop and the resolver task.
    pub(super) resolver: Arc<Resolver>,
    /// The task that sends what the resolver schedules.
    resolve_task: Option<JoinHandle<()>>,
    /// See [`Self::set_unresolved_send`].
    pub(super) unresolved: UnresolvedSend,
    /// Broadcast scope for send_broadcast.
    broadcast_scope: Bip6BroadcastScope,
    /// Foreign device BBMD configuration (optional).
    /// The foreign-device registration, shared with the Network Port
    /// object, which can move it ([`ForeignTarget`]).
    foreign_device: Option<Arc<ForeignTarget>>,
    /// Looks up FD_BBMD_Address host names (#2308).
    bbmd_resolver: Arc<dyn BbmdResolver>,
    /// See [`Self::set_release_previous_bbmd`].
    release_previous_bbmd: bool,
    /// Foreign device re-registration task handle.
    registration_task: Option<JoinHandle<()>>,
    /// The registered Network Port's lease, handed to the socket at start
    /// so every task that owns the socket keeps the port registered.
    network_port_lease: Option<Arc<()>>,
    /// Set by the first start: a lease can be retained only before it.
    registration_closed: bool,
    /// See [`Self::forwarded_group_origin_drops`].
    forwarded_group_origin_drops: Arc<std::sync::atomic::AtomicU64>,
    /// See [`Self::receive_counters`].
    receive_drops: Arc<ReceiveDrops>,
    /// See [`Self::frame_tap`].
    frame_tap: crate::tap::FrameTap,
}

/// Configuration for BIPv6 foreign device registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bip6ForeignDeviceConfig {
    /// The BBMD to register with: its B/IPv6 address, or a host name and
    /// port (#2365), which parses from `[ip]:port` or `host:port`
    /// (`"bbmd.example:47808".parse()?`).
    pub bbmd: Bip6ForeignBbmd,
    /// Time-to-live in seconds.
    pub ttl: u16,
}

/// IPv6 multicast scope for BACnet broadcasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bip6BroadcastScope {
    /// FF02::BAC0 — link-local (single link only)
    LinkLocal,
    /// FF05::BAC0 — site-local (building/campus, default)
    SiteLocal,
    /// FF08::BAC0 — organization-local
    OrganizationLocal,
}

impl Bip6BroadcastScope {
    fn multicast_addr(&self) -> Ipv6Addr {
        match self {
            Self::LinkLocal => BACNET_IPV6_MULTICAST_LINK_LOCAL,
            Self::SiteLocal => BACNET_IPV6_MULTICAST_SITE_LOCAL,
            Self::OrganizationLocal => BACNET_IPV6_MULTICAST_ORG_LOCAL,
        }
    }
}

impl Bip6Transport {
    /// Create a new BACnet/IPv6 transport.
    ///
    /// - `interface`: Concrete local IPv6 address selecting its unique usable
    ///   interface, or `::` for unambiguous automatic selection. Automatic mode
    ///   prefers a non-loopback link and a unique non-link-local address on it;
    ///   ambiguity is an error. Loopback is node-local only. Normal operation
    ///   retains the selected source/index on a wildcard receive socket.
    ///   Foreign `::` instead selects a source usable for the configured BBMD.
    /// - `port`: UDP port (default 47808 / 0xBAC0)
    /// - `device_instance`: If `Some(id)`, derive the 3-byte VMAC from the
    ///   valid 22-bit device instance (per Clause H.7.2). Otherwise an
    ///   OS-random Random Device Instance VMAC is generated at startup.
    pub fn new(interface: Ipv6Addr, port: u16, device_instance: Option<u32>) -> Self {
        Self {
            interface,
            port,
            share_wildcard_port: false,
            device_instance,
            local_mac: [0; 3],
            local_address: None,
            source_vmac: Bip6Vmac::new([0; 3]),
            socket: None,
            recv_task: None,
            resolver: Arc::default(),
            resolve_task: None,
            unresolved: UnresolvedSend::default(),
            broadcast_scope: Bip6BroadcastScope::SiteLocal,
            foreign_device: None,
            bbmd_resolver: Arc::new(SystemResolver),
            release_previous_bbmd: false,
            registration_task: None,
            network_port_lease: None,
            registration_closed: false,
            forwarded_group_origin_drops: Arc::default(),
            receive_drops: Arc::default(),
            frame_tap: crate::tap::FrameTap::new(),
        }
    }

    /// Share an explicitly requested port with other sockets on this host
    /// that bind it the same way, as before #2385. Off by default: the
    /// socket claims its port, so a second bind of it fails at once (on
    /// Windows with `SO_EXCLUSIVEADDRUSE`), and a start that finds the port
    /// held fails with an error that says so. Call it before `start()`.
    ///
    /// With sharing on, the socket sets `SO_REUSEADDR`. A datagram sent to
    /// a BACnet group reaches every socket on the port that joined it, but
    /// each unicast reaches only one of the sockets: on Linux the one that
    /// bound last. That can be another program that takes this transport's
    /// replies, address resolutions and registrations. Two stacks on one
    /// host are better served each on its own port.
    ///
    /// It changes nothing for a port the OS picks, which is never shared
    /// (#892). The claim needs no second socket: a normal node's one socket
    /// on `[::]` joins the BACnet groups itself, and takes their datagrams
    /// and its own broadcasts' echoes as before. A foreign device binds
    /// its own address, and claims that address and port.
    ///
    /// By OS: Linux and Windows refuse every later bind of a claimed port
    /// that could take its unicast, and with sharing on let one in. macOS
    /// and the BSDs refuse a second socket on `[::]`, or on a foreign
    /// device's address, with or without sharing; but they let a socket
    /// that sets `SO_REUSEADDR` bind a specific address beside a normal
    /// node's `[::]`, claimed or not, and it then gets the unicast sent
    /// there. They have no option to refuse it. See `docs/rust-api.md`
    /// (BIP6, Claiming the port).
    pub fn set_share_wildcard_port(&mut self, enabled: bool) {
        self.share_wildcard_port = enabled;
    }

    /// Set the broadcast scope for send_broadcast.
    pub fn set_broadcast_scope(&mut self, scope: Bip6BroadcastScope) {
        self.broadcast_scope = scope;
    }

    /// Configure this transport as a foreign device.
    ///
    /// Foreign `::` derives a concrete source from the route to the BBMD and
    /// binds the production socket to it, so a BBMD named by host name
    /// ([`Bip6ForeignBbmd::Name`], #2365) is looked up at start, and the
    /// start fails if the name doesn't stand for an IPv6 address the device
    /// could register with. An explicit address is retained: a start on
    /// one goes ahead with a name that doesn't resolve, and the device
    /// waits for it, registering nothing, refusing its broadcasts (counted
    /// in [`foreign_status`](Self::foreign_status)) and looking the name up
    /// again 30 seconds after each failure, until it resolves.
    /// Foreign startup does not require normal-mode multicast membership.
    /// `start()` refuses a registration that fails
    /// [`Bip6ForeignDeviceConfig::check`] (a link-local BBMD, say).
    /// A foreign device must also have a configured Device instance. Random
    /// VMAC startup is rejected until BBMD-assisted collision resolution is
    /// implemented.
    /// Must be called before `start()`.
    ///
    /// A registered Network Port can move the registration while the
    /// transport runs (FD_BBMD_Address and FD_Subscription_Lifetime, on
    /// ReinitializeDevice ACTIVATE_CHANGES or WARMSTART): the transport
    /// then registers with the new BBMD at once, sends its broadcasts and
    /// resolutions there, and trusts its Forwarded-NPDUs only.
    /// FD_BBMD_Address may name the BBMD by host name
    /// ([`set_bbmd_resolver`](Self::set_bbmd_resolver)).
    pub fn register_as_foreign_device(&mut self, config: Bip6ForeignDeviceConfig) {
        let resolver = Arc::clone(&self.bbmd_resolver);
        let target = ForeignTarget::new(config, resolver);
        target.set_release_previous(self.release_previous_bbmd);
        self.foreign_device = Some(target);
    }

    /// After a Network Port activation moves this foreign device to another
    /// BBMD, or the BBMD's host name comes to resolve elsewhere (#2308),
    /// also ask the BBMD it left to delete its entry
    /// (Delete-Foreign-Device-Table-Entry, Annex U.4.5.4) rather than leave
    /// it until its time-to-live and grace period run out (U.4.5.5, #2338).
    /// It is B/IP's release
    /// ([`BipTransport::set_release_previous_bbmd`](crate::bip::BipTransport::set_release_previous_bbmd)):
    /// one request, naming this device's own bound B/IPv6 address, sent
    /// after the first registration with the new BBMD and given up after 3
    /// seconds. Best effort: it never holds up or fails the move, and its
    /// outcome is counted in [`previous_bbmd_releases`](Self::previous_bbmd_releases).
    /// Off by default, as on B/IP: a BBMD may refuse deletions it doesn't
    /// trust, and the entry lapses on its own anyway.
    ///
    /// Call it before or after
    /// [`register_as_foreign_device`](Self::register_as_foreign_device); it
    /// means nothing outside foreign-device mode.
    pub fn set_release_previous_bbmd(&mut self, release: bool) {
        self.release_previous_bbmd = release;
        if let Some(target) = &self.foreign_device {
            target.set_release_previous(release);
        }
    }

    /// What came of the releases of the BBMDs this foreign device moved
    /// away from ([`set_release_previous_bbmd`](Self::set_release_previous_bbmd),
    /// #2338): all zero outside foreign-device mode, or with the release
    /// off.
    pub fn previous_bbmd_releases(&self) -> Bip6ReleaseCounters {
        self.foreign_device
            .as_ref()
            .map(|target| target.release_counters())
            .unwrap_or_default()
    }

    /// How this foreign device's registration stands (#2365): the BBMD's
    /// address in use, whether it waits for its BBMD's host name, the
    /// BBMD's latest answer to a registration (#2388), and the lookups,
    /// releases, registrations and refused broadcasts so far. `None`
    /// outside foreign-device mode.
    pub fn foreign_status(&self) -> Option<Bip6ForeignStatus> {
        self.foreign_device.as_ref().map(|target| target.status())
    }

    /// Look up the host name a foreign device's FD_BBMD_Address names with
    /// `resolver`, instead of the system's resolver on Tokio's blocking
    /// pool, the default (#2308). Call it before or after
    /// [`register_as_foreign_device`](Self::register_as_foreign_device).
    ///
    /// A host name written to FD_BBMD_Address is checked for its form, and
    /// looked up whenever ReinitializeDevice activates it, on the side,
    /// and again alongside the re-registrations (the first after a start
    /// included), at most once every 30 seconds; the device moves to the
    /// first IPv6 address among the answers it could register with, as an
    /// activated address would move it, and re-registers there at once.
    /// The policy is B/IP's
    /// ([`BipTransport::set_bbmd_resolver`](crate::bip::BipTransport::set_bbmd_resolver)):
    /// one resolver call at a time, given up after 10 seconds; the device
    /// stays put while the address in use is among the answers; and a
    /// failed lookup keeps the address it had, and is logged and counted
    /// in [`bbmd_lookup_counters`](Self::bbmd_lookup_counters).
    pub fn set_bbmd_resolver(&mut self, resolver: Arc<dyn BbmdResolver>) {
        if let Some(target) = &self.foreign_device {
            target.set_resolver(Arc::clone(&resolver));
        }
        self.bbmd_resolver = resolver;
    }

    /// What came of the lookups of a BBMD host name that FD_BBMD_Address
    /// names (#2308): all zero outside foreign-device mode, or while it
    /// holds an address.
    pub fn bbmd_lookup_counters(&self) -> BbmdLookupCounters {
        self.foreign_device
            .as_ref()
            .map(|target| target.lookup_counters())
            .unwrap_or_default()
    }

    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. A BBMD configured by
    /// host name reads back as the name until it first resolves, and as
    /// the address in use from then on. `None` outside foreign-device
    /// mode.
    pub fn foreign_device_config(&self) -> Option<Bip6ForeignDeviceConfig> {
        self.foreign_device.as_ref().map(|target| target.config())
    }

    /// The foreign-device target, for a test's gate.
    #[cfg(test)]
    pub(super) fn foreign_target(&self) -> Option<&Arc<ForeignTarget>> {
        self.foreign_device.as_ref()
    }

    /// Forwarded-NPDUs dropped since this transport was created because
    /// their original source address is an IPv6 multicast group (#1493). No
    /// node sends from one, so such a frame is malformed and never reaches
    /// the network layer. The total survives a restart.
    pub fn forwarded_group_origin_drops(&self) -> u64 {
        self.forwarded_group_origin_drops
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// This transport's frame tap: every BVLL message it sends and
    /// receives, address resolution and registrations included, for
    /// whoever subscribes. See [`crate::tap`].
    pub fn frame_tap(&self) -> &crate::tap::FrameTap {
        &self.frame_tap
    }

    /// The datagrams the receive path dropped since this transport was
    /// created, by reason, and the replies it held back over its quotas
    /// (#1643). The totals survive a restart. A Forwarded-NPDU with a group
    /// origin is counted in [`Self::forwarded_group_origin_drops`] instead.
    pub fn receive_counters(&self) -> Bip6ReceiveCounters {
        self.receive_drops.snapshot()
    }
}

/// Re-register with `target` every half time-to-live, and at once with
/// the new values when an activation, or a lookup of the BBMD's host name,
/// moves the registration. Each pass reads the target afresh, and marks
/// the registration it sends outstanding before the send (#2388); no lock
/// is held across an await. A move's release of the BBMD left (#2338) goes
/// after the registration with the new one, and is given up here when its
/// answer is overdue.
async fn reregister(target: Arc<ForeignTarget>, socket: Arc<Bip6Socket>, source_vmac: Bip6Vmac) {
    let mut renew_at = tokio::time::Instant::now() + target.renewal_interval();
    loop {
        let release_due = target.release_due();
        let retry_due = target.retry_due();
        let cause = tokio::select! {
            () = tokio::time::sleep_until(renew_at) => Cause::Renewal,
            () = target.changed() => Cause::Change,
            () = tokio::time::sleep_until(release_due.unwrap_or(renew_at)), if release_due.is_some() => {
                target.release_expired(tokio::time::Instant::now());
                continue;
            }
            // A device waiting for its BBMD's name tries it again 30 s
            // after the newest failure (#2365).
            () = target.rearmed() => continue,
            () = tokio::time::sleep_until(retry_due.unwrap_or(renew_at)), if retry_due.is_some() => {
                if let Some(due) = retry_due {
                    target.retry(due);
                }
                continue;
            }
        };
        // The name, if the BBMD has one, is looked up alongside at most
        // every 30 s; the attempt goes where it was, if anywhere yet.
        target.attempting();
        let (config, release) = target.pass(tokio::time::Instant::now(), cause);
        #[cfg(test)]
        target.in_flight().await;
        if let Some(bbmd) = config.bbmd.address() {
            if !send_fails(&target)
                && send_register_foreign_device_v6(&socket, bbmd, config.ttl, source_vmac).await
            {
                target.registration_sent();
            } else {
                target.registration_not_sent(bbmd);
            }
        }
        if let Some((left, entry)) = release
            && !send_release_v6(&socket, left, entry, source_vmac).await
        {
            target.release_not_sent(left);
        }
        renew_at = tokio::time::Instant::now() + target.renewal_interval();
    }
}

/// Whether a test made this re-registration's send fail, unsent.
#[cfg(test)]
fn send_fails(target: &ForeignTarget) -> bool {
    target
        .fail_send
        .swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Whether a test made this re-registration's send fail: never outside one.
#[cfg(not(test))]
fn send_fails(_: &ForeignTarget) -> bool {
    false
}

/// Ask `bbmd` to delete the FDT entry `entry` (Clause U.4.5.4); whether it
/// was sent.
async fn send_release_v6(
    socket: &Bip6Socket,
    bbmd: SocketAddrV6,
    entry: Bip6Address,
    source_vmac: Bip6Vmac,
) -> bool {
    let release = Bvll6::DeleteForeignDeviceTableEntry {
        source: source_vmac,
        entry,
    };
    let sent = match encoded(&release) {
        Ok(buf) => socket
            .send_to(&buf, bbmd)
            .await
            .map(drop)
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match &sent {
        Ok(()) => debug!(%bbmd, "BIP6: asked the BBMD left to delete this device's entry"),
        Err(error) => {
            warn!(%error, %bbmd, "BIP6: failed to send Delete-Foreign-Device-Table-Entry")
        }
    }
    sent.is_ok()
}

/// Send a Register-Foreign-Device message (Clause U.4.5).
async fn send_register_foreign_device_v6(
    socket: &Bip6Socket,
    bbmd_addr: SocketAddrV6,
    ttl: u16,
    source_vmac: Bip6Vmac,
) -> bool {
    let registration = Bvll6::RegisterForeignDevice {
        source: source_vmac,
        ttl,
    };
    let buf = match encoded(&registration) {
        Ok(buf) => buf,
        Err(e) => {
            warn!(error = %e, "BIP6: failed to encode Register-Foreign-Device");
            return false;
        }
    };
    if let Err(e) = socket.send_to(&buf, bbmd_addr).await {
        warn!(error = %e, "BIP6: failed to send Register-Foreign-Device");
        false
    } else {
        debug!(bbmd = %bbmd_addr, ttl = ttl, "BIP6: sent Register-Foreign-Device");
        true
    }
}

impl TransportPort for Bip6Transport {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }

    /// NORMAL, or FOREIGN with a live view of its registration, which a
    /// Network Port activation can move while the transport runs. This
    /// transport can't act as a BBMD.
    fn bip6_port(&self) -> Option<Bip6Port> {
        let (endpoint, vmac) = match self.local_address {
            Some(bound) => (*bound.ip(), Some(self.source_vmac.octets())),
            None => (
                self.interface,
                self.device_instance
                    .and_then(|instance| vmac_from_device_instance(instance).ok())
                    .map(Bip6Vmac::octets),
            ),
        };
        let port = self.local_address.map_or(self.port, |bound| bound.port());
        let mode = match &self.foreign_device {
            Some(target) => BipPortMode::Foreign {
                registration: Arc::clone(target) as Arc<dyn ForeignRegistration>,
            },
            None => BipPortMode::Normal,
        };
        Some(Bip6Port {
            endpoint: SocketAddrV6::new(endpoint, port, 0, 0),
            vmac,
            multicast_group: self.broadcast_scope.multicast_addr(),
            mode,
        })
    }

    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        if self.registration_closed
            || self.socket.is_some()
            || self.recv_task.is_some()
            || self.network_port_lease.is_some()
        {
            return Err(Error::Encoding(
                "registered B/IPv6 lease requires an unstarted transport".into(),
            ));
        }
        self.network_port_lease = Some(lease);
        Ok(())
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.registration_closed = true;
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP6 transport already started",
            )));
        }
        let configured_vmac = self
            .device_instance
            .map(vmac_from_device_instance)
            .transpose()?;
        if self.foreign_device.is_some() && self.device_instance.is_none() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "BIP6 foreign devices require a configured Device instance until BBMD-assisted random-VMAC resolution is implemented",
            )));
        }
        if let Some(Err(refusal)) = self.foreign_device.as_ref().map(|fd| fd.config().check()) {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("BIP6 foreign-device registration refused: {refusal}"),
            )));
        }

        // A foreign device's BBMD, looked up first if a start on `::` needs
        // a host name's address to bind by (#2365).
        let start_bbmd = match &self.foreign_device {
            Some(fd) => Some(fd.start_bbmd(self.interface).await?),
            None => None,
        };
        let role = start_bbmd
            .as_ref()
            .map_or(Role::Normal, |start| Role::Foreign(start.bbmd));

        let port_use = PortUse::sharing(self.share_wildcard_port);

        // Resolve and prepare privately. No identity, port, VMAC or socket is
        // published until joins and the required collision exchange succeed.
        let socket = Arc::new(
            Bip6Socket::bind(self.interface, self.port, port_use, role)
                .await
                .map_err(Error::Transport)?
                .tapped(self.frame_tap.clone())
                .leased(self.network_port_lease.clone()),
        );
        let bound = socket.local_address().map_err(Error::Transport)?;
        let local_port = bound.port();
        let local_ip = socket.selection.map_or(*bound.ip(), |link| link.address);
        let wildcard_bind = false;
        let local_unicast_ips = vec![local_ip];
        let mut source_vmac = match configured_vmac {
            Some(vmac) => vmac,
            None => generate_random_vmac()?,
        };
        let if_index = socket.selection.map_or(0, |link| link.index);
        let scope_id = match socket.selection {
            _ if !local_ip.is_unicast_link_local() => 0,
            Some(link) => link.index,
            None => bound.scope_id(),
        };
        let local_address = SocketAddrV6::new(local_ip, local_port, 0, scope_id);
        let collision_probe_required = self.device_instance.is_none();
        let collision_group = self.broadcast_scope.multicast_addr();

        // VMAC collision detection and resolution
        if self.foreign_device.is_none() {
            let multicast_dest = SocketAddrV6::new(collision_group, local_port, 0, if_index);
            let mut check_buf = vec![0u8; 64];

            for attempt in 0..=MAX_VMAC_RETRIES {
                let ar_msg = encoded(&Bvll6::AddressResolution {
                    source: source_vmac,
                    target: source_vmac,
                })?;
                if let Err(error) = socket.send_to(&ar_msg, multicast_dest).await {
                    if collision_probe_required {
                        return Err(Error::Transport(error));
                    }
                    debug!(error = %error, "Configured-VMAC collision probe send failed");
                }

                let probe = Probe {
                    vmac: source_vmac,
                    local_ip,
                    local_port,
                    local_unicast_ips: &local_unicast_ips,
                    wildcard_bind,
                    group: collision_group,
                    required: collision_probe_required,
                };
                let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
                let collision = probe.window(&*socket, &mut check_buf, deadline).await?;

                if !collision {
                    break;
                }

                if self.device_instance.is_some() {
                    return Err(Error::Transport(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "configured BACnet Device instance VMAC is already in use",
                    )));
                }

                if attempt < MAX_VMAC_RETRIES {
                    let old_vmac = source_vmac;
                    source_vmac = generate_random_vmac()?;
                    warn!(
                        old_vmac = ?old_vmac,
                        new_vmac = ?source_vmac,
                        attempt = attempt + 1,
                        max_retries = MAX_VMAC_RETRIES,
                        "BIP6 VMAC collision detected, re-deriving new VMAC"
                    );
                } else {
                    return Err(Error::Transport(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!(
                            "random BIP6 VMAC collision persists after {MAX_VMAC_RETRIES} retries"
                        ),
                    )));
                }
            }
        }

        // Do the only foreign-device startup I/O before spawning owned tasks,
        // so cancellation cannot orphan an unpublished receive worker.
        // A device waiting for its BBMD's name registers once it resolves.
        // A start on `::` registers where its lookup found the BBMD.
        if let (Some(fd), Some(bbmd)) = (
            &self.foreign_device,
            start_bbmd.as_ref().and_then(|start| start.bbmd),
        ) {
            // Outstanding before it goes, so its answer finds it (#2388).
            fd.registration_going(bbmd);
            if send_register_foreign_device_v6(&socket, bbmd, fd.config().ttl, source_vmac).await {
                fd.registration_sent();
            } else {
                fd.registration_not_sent(bbmd);
            }
        }
        // The address a start on `::` found is the registration's from here,
        // before the receive loop and the driver read it: nothing after
        // this can fail the start, which keeps the name until now.
        let found = start_bbmd
            .as_ref()
            .filter(|start| start.looked_up)
            .and_then(|start| start.bbmd);
        if let (Some(fd), Some(found)) = (&self.foreign_device, found) {
            fd.take_found(found);
        }

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        self.resolver.set_own(Some(source_vmac));
        let receiver = Receiver {
            socket: Arc::clone(&socket),
            tx,
            local_port,
            vmac: source_vmac,
            local_ip,
            unicast_ips: local_unicast_ips,
            wildcard_bind,
            foreign_bbmd: self.foreign_device.clone(),
            neighbours: Arc::clone(&self.resolver),
            forwarded_group_origin_drops: Arc::clone(&self.forwarded_group_origin_drops),
            drops: Arc::clone(&self.receive_drops),
            nak_quota: ReplyLimiter::default(),
            far_ack_quota: ReplyLimiter::default(),
        };
        let recv_task = tokio::spawn(receiver.run());
        // Address-Resolutions go where a broadcast would: to the BBMD for a
        // foreign device, which passes them on (U.4.4), and otherwise to
        // the scope's group at this node's port.
        let driver = Driver {
            socket: Arc::clone(&socket),
            resolver: Arc::clone(&self.resolver),
            source: source_vmac,
            resolution_to: SocketAddrV6::new(collision_group, local_port, 0, 0),
            foreign: self.foreign_device.clone(),
        };
        let resolve_task = tokio::spawn(driver.run());

        self.local_mac = source_vmac.octets();
        self.local_address = Some(local_address);
        self.source_vmac = source_vmac;
        self.recv_task = Some(recv_task);
        self.resolve_task = Some(resolve_task);
        self.socket = Some(Arc::clone(&socket));

        // Start foreign device registration if configured
        if let Some(fd) = &self.foreign_device {
            fd.attach(local_address);
            // A BBMD named by host name is looked up alongside the
            // registration this start sent, or, waiting, for the first
            // time, unless the start looked it up already.
            if found.is_none() {
                fd.attempting();
            }
            let reg_task = tokio::spawn(reregister(
                Arc::clone(fd),
                Arc::clone(&socket),
                self.source_vmac,
            ));
            self.registration_task = Some(reg_task);
        }

        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        // Refuse changes first, so none is accepted while the tasks stop
        // and leaves a permit no task will take.
        if let Some(fd) = &self.foreign_device {
            fd.detach();
        }
        if let Some(task) = self.registration_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.resolve_task.take() {
            task.abort();
            let _ = task.await;
        }
        // Only now, so no pass or answer in flight leaves one behind.
        if let Some(fd) = &self.foreign_device {
            fd.forget_answers();
        }
        self.socket = None;
        self.local_mac = [0; 3];
        self.local_address = None;
        self.source_vmac = Bip6Vmac::new([0; 3]);
        self.resolver.clear();
        self.resolver.set_own(None);
        // Every task that owned the socket has ended.
        self.network_port_lease = None;
        Ok(())
    }

    /// A router routes through this port: an NPDU for a VMAC that needs
    /// resolving is queued rather than waited for
    /// ([`UnresolvedSend::Queue`]), so one unknown destination, which any
    /// node on any network the router serves can name, doesn't stall the
    /// port's sends.
    fn prepare_for_routing(&mut self) {
        self.unresolved = UnresolvedSend::Queue;
    }

    /// Send `npdu` to the node whose 3-octet VMAC `mac` holds, in an
    /// Original-Unicast-NPDU to the address the transport has for it.
    ///
    /// When the transport has none, it resolves the VMAC first (U.5): an
    /// Address-Resolution goes to the scope's BACnet group, or for a
    /// foreign device to its BBMD, at once and then a second and two seconds
    /// later, and the resolution fails three seconds after the first.
    /// Meanwhile the send waits, or with [`UnresolvedSend::Queue`] it is
    /// queued and the call returns at once. A VMAC that failed fails a send
    /// at once for the next ten seconds, unless the transport hears from it.
    ///
    /// Errors (all [`Error::Transport`]): `NotConnected` before start;
    /// `InvalidInput` for a MAC that isn't three octets, or this node's own
    /// VMAC; `TimedOut` when nothing answered the resolution;
    /// `ConnectionRefused` when the BBMD refused to pass it on and nothing
    /// answered; `HostUnreachable` within ten seconds of a failure; and
    /// `QuotaExceeded` when 64 VMACs are resolving, or 64 sends wait on this
    /// one (512 in all), or 8 NPDUs are queued for it (128 in all).
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })?;
        self.resolver
            .send_unicast(&**socket, self.source_vmac, self.unresolved, npdu, mac)
            .await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })?;

        let source_vmac = self.source_vmac;

        // In foreign device mode, use Distribute-Broadcast-To-Network via BBMD
        if let Some(fd) = &self.foreign_device {
            // Waiting for the BBMD's host name: no hold queue, so refused.
            let Some(bbmd_addr) = fd.bbmd_addr() else {
                fd.broadcast_refused();
                return Err(Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "the foreign device's BBMD isn't known yet: its host name hasn't resolved",
                )));
            };
            let mut buf = BytesMut::new();
            encode_bvll6(
                &mut buf,
                &Bvll6::DistributeBroadcastToNetwork {
                    original_source: source_vmac,
                    npdu,
                },
            )?;
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        let dest = SocketAddrV6::new(
            self.broadcast_scope.multicast_addr(),
            socket.local_port().map_err(Error::Transport)?,
            0,
            0,
        );
        let mut buf = BytesMut::new();
        encode_bvll6(
            &mut buf,
            &Bvll6::OriginalBroadcastNpdu {
                source: source_vmac,
                npdu,
            },
        )?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    /// This node's 3-octet VMAC (U.1.3), zero before start and after stop.
    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// No VMAC names a group: a B/IPv6 broadcast is the network layer's
    /// zero-length MAC, which goes to [`send_broadcast`](Self::send_broadcast).
    fn is_group_destination(&self, _mac: &[u8]) -> bool {
        false
    }

    /// A rule that matches no MAC, as [`Self::is_group_destination`].
    fn group_destinations(&self) -> crate::port::GroupDestinations {
        crate::port::GroupDestinations::new(|_| false)
    }
}

impl Drop for Bip6Transport {
    fn drop(&mut self) {
        // The tasks own socket clones. Abort them so dropping the transport
        // cannot detach a receive/registration lifetime from its owner.
        if let Some(task) = self.registration_task.take() {
            task.abort();
        }
        if let Some(task) = self.recv_task.take() {
            task.abort();
        }
        if let Some(task) = self.resolve_task.take() {
            task.abort();
        }
        // Fails any waiter a leaked send left behind.
        self.resolver.clear();
    }
}
