use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bip_port::{BipPortMode, ForeignRegistration};
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::port::{Bip6Port, ReceivedNpdu, TransportPort};

use super::collision_probe::Probe;
use super::foreign_target::ForeignTarget;
use super::frame::encoded;
use super::receive::Receiver;
use super::receive_counters::{Bip6ReceiveCounters, ReceiveDrops};
use super::reply_limit::ReplyLimiter;
use super::resolver::{Driver, Resolver, UnresolvedSend};
use super::socket::Bip6Socket;
use super::vmac_table::{generate_random_vmac, vmac_from_device_instance};
use super::{Bip6Vmac, Bvll6, MAX_VMAC_RETRIES, encode_bvll6};

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
    /// BBMD IPv6 address to register with.
    pub bbmd_ip: Ipv6Addr,
    /// BBMD port.
    pub bbmd_port: u16,
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
            registration_task: None,
            network_port_lease: None,
            registration_closed: false,
            forwarded_group_origin_drops: Arc::default(),
            receive_drops: Arc::default(),
            frame_tap: crate::tap::FrameTap::new(),
        }
    }

    /// Set the broadcast scope for send_broadcast.
    pub fn set_broadcast_scope(&mut self, scope: Bip6BroadcastScope) {
        self.broadcast_scope = scope;
    }

    /// Configure this transport as a foreign device.
    ///
    /// Foreign `::` derives a concrete source from the route to the BBMD and
    /// binds the production socket to it. An explicit address is retained.
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
    pub fn register_as_foreign_device(&mut self, config: Bip6ForeignDeviceConfig) {
        self.foreign_device = Some(Arc::new(ForeignTarget::new(config)));
    }

    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. `None` outside
    /// foreign-device mode.
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

/// Send a Register-Foreign-Device message (Clause U.4.5).
async fn send_register_foreign_device_v6(
    socket: &Bip6Socket,
    bbmd_addr: SocketAddrV6,
    ttl: u16,
    source_vmac: Bip6Vmac,
) {
    let registration = Bvll6::RegisterForeignDevice {
        source: source_vmac,
        ttl,
    };
    let buf = match encoded(&registration) {
        Ok(buf) => buf,
        Err(e) => {
            warn!(error = %e, "BIP6: failed to encode Register-Foreign-Device");
            return;
        }
    };
    if let Err(e) = socket.send_to(&buf, bbmd_addr).await {
        warn!(error = %e, "BIP6: failed to send Register-Foreign-Device");
    } else {
        debug!(bbmd = %bbmd_addr, ttl = ttl, "BIP6: sent Register-Foreign-Device");
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

        // Resolve and prepare privately. No identity, port, VMAC or socket is
        // published until joins and the required collision exchange succeed.
        let foreign = self.foreign_device.as_ref().map(|fd| fd.bbmd_addr());
        let socket = Arc::new(
            Bip6Socket::bind(self.interface, self.port, foreign)
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
        if let Some(fd) = &self.foreign_device {
            let config = fd.config();
            send_register_foreign_device_v6(
                &socket,
                SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0),
                config.ttl,
                source_vmac,
            )
            .await;
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
            let target = Arc::clone(fd);
            let sock = Arc::clone(&socket);
            let source_vmac = self.source_vmac;

            // Re-register every half time-to-live, and at once with the new
            // values when an activation moves the registration. Each pass
            // reads the target afresh; no lock is held across an await.
            let reg_task = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(target.renewal_interval()) => {}
                        () = target.changed() => {}
                    }
                    let config = target.config();
                    #[cfg(test)]
                    target.in_flight().await;
                    let bbmd = SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0);
                    send_register_foreign_device_v6(&sock, bbmd, config.ttl, source_vmac).await;
                }
            });
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
            let bbmd_addr = fd.bbmd_addr();
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
