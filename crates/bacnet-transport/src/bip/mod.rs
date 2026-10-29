//! BACnet/IP over UDP transport (Annex J).
//!
//! Wraps a tokio UDP socket with BVLL framing. The recv loop decodes
//! incoming BVLL frames and extracts NPDU bytes + source MAC for the
//! network layer. Optionally acts as a BBMD or foreign device.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use bytes::BytesMut;
#[cfg(test)]
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::bbmd::{self, BbmdState, BdtEntry};
pub use crate::bbmd::{FdtCounters, ForeignDevicePolicy};
#[cfg(test)]
use crate::bvll::BvllMessage;
use crate::bvll::encode_bip_mac;
#[cfg(test)]
use crate::bvll::{
    decode_bip_mac, decode_bvll, forwarded_message, forwarded_wire, origin_of, raw_frame,
};
use crate::port::{BipPort, ReceivedNpdu, TransportPort};
#[cfg(test)]
use crate::udp_metadata::{DestinationReceiver, IpVersion};
use bacnet_bip_codec as codec;
#[cfg(test)]
use bacnet_bip_codec::{BdtEntries, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{Cast, Link, Mode};
use bacnet_npdu::NpduLimits;
use bacnet_types::bip_port::{BbmdTables, BipPortMode, ForeignRegistration};
use bacnet_types::error::Error;

mod access;
mod bbmd_receive;
mod bbmd_start;
mod client_snapshot;
mod exchange;
pub use bacnet_bip_core::node::ExchangeRefusal;
pub use exchange::BvlcExchangeError;
mod foreign_device;
mod foreign_target;
mod mode;
mod node;
mod send;
mod send_counters;
#[cfg(test)]
mod send_sink;
#[cfg(test)]
mod test_support;
pub use client_snapshot::{
    BvlcClientSnapshot, BvlcClientStatus, BvlcRequestCounters, ForeignRegistrationOutcome,
    ForeignRegistrationSnapshot,
};
pub use foreign_device::ForeignDeviceConfig;
pub use mode::{BbmdOptions, BipMode};
mod fanout;
mod groups;
mod multicast;
pub use multicast::BipMulticast;
mod passive;
#[cfg(any(test, feature = "test-support"))]
mod send_observer;
mod socket;
use bbmd_start::{
    BbmdConfig, OwnAddressContext, initial_bbmd_state, refresh_own_address, subnet_mask_of,
    warn_about_peers_on_the_group_subnet, warn_if_broadcast_may_leave_another_interface,
};
use client_snapshot::ClientRecord;
#[cfg(test)]
use exchange::{decode_bvlc_result_code, unexpected_response};
use node::NodeHandle;
pub use passive::PassiveBip;
#[cfg(feature = "test-support")]
pub use send_observer::SendAttemptObserver;
use socket::BipSocket;
mod ingress;
mod io;
mod own_broadcast;
mod rate_limit;
mod receive_counters;
pub use access::AsBip;
pub use fanout::{FanoutCounters, FanoutPolicy};
#[cfg(test)]
use ingress::{Arrival, handle_datagram};
use ingress::{IngressAddresses, Listeners, broadcast_is_own_address, receive_loop};
use io::RecvContext;
use own_broadcast::OwnBroadcastForwarder;
pub use rate_limit::ManagementCounters;
use rate_limit::ManagementRateLimiter;
pub use receive_counters::ReceiveCounters;
use receive_counters::ReceiveDrops;
pub use send_counters::SendCounters;
use send_counters::SendRefusals;
#[cfg(test)]
use test_support::{Delivery, admitted_delivery, handle_bvll_message};

/// The longest datagram the transport sends: a Forwarded-NPDU carrying a
/// 1497-octet NPDU, the largest B/IP carries (Table 6-1). Longer NPDUs are
/// refused, with an error, and counted in [`SendCounters`] (#1622).
const DATAGRAM_BUDGET: u16 = bacnet_bip_core::node::MAX_DATAGRAM as u16;

/// Default BACnet/IP port (0xBAC0 = 47808).
pub const DEFAULT_BACNET_PORT: u16 = 0xBAC0;

/// BACnet/IP transport over UDP.
pub struct BipTransport {
    interface: Ipv4Addr,
    port: u16,
    /// Fixed at construction: a restart rebinds the remembered actual port,
    /// but only an explicitly requested one may be shared.
    share_port: bool,
    /// See [`Self::set_share_port_by_address`].
    share_port_by_address: bool,
    broadcast_address: Ipv4Addr,
    local_mac: [u8; 6],
    socket: Option<Arc<BipSocket>>,
    network_port_lease: Option<Arc<()>>,
    registration_closed: bool,
    recv_task: Option<JoinHandle<()>>,
    /// BBMD configuration before start (consumed by `start()`).
    bbmd_config: Option<BbmdConfig>,
    /// BBMD state (when acting as a BBMD, created in `start()`).
    bbmd: Option<Arc<std::sync::Mutex<BbmdState>>>,
    /// BBMD FDT expiry purge task.
    bbmd_fdt_purge_task: Option<JoinHandle<()>>,
    /// Foreign device registration (when registered as a foreign device),
    /// shared with the Network Port object that can change it.
    foreign_device: Option<Arc<foreign_target::ForeignTarget>>,
    /// The node engine of the running transport: its link, its own
    /// management exchanges and a foreign device's registration.
    node: Option<Arc<NodeHandle>>,
    /// The task that sends what the node asks for and runs its timers.
    node_task: Option<JoinHandle<()>>,
    /// This transport's own management exchanges, across restarts.
    client_record: Arc<ClientRecord>,
    /// Optional persisted-BDT file (wire format, 10 bytes per entry): loaded
    /// at startup, and rewritten when a Network Port activation changes the
    /// BDT. Inbound Write-BDT does not update it.
    bdt_file: Option<Arc<bbmd::BdtFile>>,
    /// Management request and response rate limiter.
    management_limiter: Arc<std::sync::Mutex<ManagementRateLimiter>>,
    /// Broadcast forwarding fanout policy.
    fanout_policy: FanoutPolicy,
    /// Background worker task for broadcast forwarding.
    fanout_task: Option<JoinHandle<()>>,
    /// Operational counters for broadcast forwarding fanout.
    fanout_counters: Arc<fanout::AtomicFanoutCounters>,
    /// See [`Self::group_source_drops`] and
    /// [`Self::forwarded_group_origin_drops`].
    group_drops: groups::GroupDrops,
    /// See [`Self::receive_counters`].
    receive_drops: Arc<ReceiveDrops>,
    /// See [`Self::frame_tap`].
    frame_tap: crate::tap::FrameTap,
    /// See [`Self::send_counters`].
    send_refusals: Arc<SendRefusals>,
    /// Rate limiter for broadcast forwarding fanout.
    fanout_limiter: Arc<std::sync::Mutex<fanout::FanoutRateLimiter>>,
    /// Forwards this BBMD's own broadcasts to BDT peers and foreign devices
    /// (BBMD mode only, created in `start()`).
    own_broadcast: Option<OwnBroadcastForwarder>,
    /// The B/IP-M group, if any (#941), shared with the Network Port object
    /// that can change it.
    multicast: Arc<multicast::MulticastControl>,
    /// Replaces the result of listing the host's local IPv4 addresses (and
    /// its default-route address) that a wildcard bind reads in `start()`.
    #[cfg(test)]
    local_ipv4_for_test: Option<std::io::Result<(Vec<Ipv4Addr>, Option<Ipv4Addr>)>>,
    /// Runs in a stop between stopping the node and clearing the client
    /// status, where a call racing the stop can land.
    #[cfg(test)]
    between_stops: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl BipTransport {
    /// Create a new BACnet/IP transport.
    ///
    /// - `interface`: Local IP to bind (use `0.0.0.0` for all interfaces; see
    ///   [`enable_bbmd`](Self::enable_bbmd) for how a BBMD then picks its own address).
    ///   With `0.0.0.0`, `start()` lists the host's IPv4 addresses and accepts
    ///   unicast only to one of them; it fails when they cannot be listed or
    ///   none is usable. The list is read at each start, so an address added
    ///   later is accepted after the next restart. The socket binds the
    ///   wildcard address either way, unless
    ///   [`set_share_port_by_address`](Self::set_share_port_by_address) asks
    ///   for an explicit address's own socket.
    /// - `port`: UDP port (default 47808 / 0xBAC0). Port zero asks for a
    ///   private ephemeral port.
    /// - `broadcast_address`: Directed broadcast address (e.g., `255.255.255.255`)
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        let fanout_policy = FanoutPolicy::default();
        let fanout_limiter = Arc::new(std::sync::Mutex::new(fanout::FanoutRateLimiter::new(
            fanout_policy.clone(),
        )));
        let fanout_counters = Arc::new(fanout::AtomicFanoutCounters::default());
        Self {
            interface,
            port,
            share_port: port != 0,
            share_port_by_address: false,
            broadcast_address,
            local_mac: [0; 6],
            socket: None,
            network_port_lease: None,
            registration_closed: false,
            recv_task: None,
            bbmd_config: None,
            bbmd: None,
            bbmd_fdt_purge_task: None,
            foreign_device: None,
            node: None,
            node_task: None,
            client_record: Arc::default(),
            bdt_file: None,
            management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
            fanout_policy,
            fanout_task: None,
            fanout_counters,
            group_drops: groups::GroupDrops::default(),
            receive_drops: Arc::default(),
            frame_tap: crate::tap::FrameTap::new(),
            send_refusals: Arc::default(),
            fanout_limiter,
            own_broadcast: None,
            multicast: Arc::new(multicast::MulticastControl::new(interface, port)),
            #[cfg(test)]
            local_ipv4_for_test: None,
            #[cfg(test)]
            between_stops: None,
        }
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
    /// socket already bound to the wildcard address on that port doesn't stop
    /// it, and under Windows' strong host model a multihomed BBMD reaches a
    /// peer only through the interface it is bound to. See
    /// `docs/rust-api.md` (BIP section) for each OS.
    pub fn set_share_port_by_address(&mut self, enabled: bool) {
        self.share_port_by_address = enabled;
    }

    /// Enable BBMD mode with the given initial BDT.
    /// Must be called before `start()`.
    ///
    /// The BBMD's own B/IP address is the interface address and bound port.
    /// With a `0.0.0.0` interface, `start()` takes it from the BDT it starts
    /// with: the one row whose IP is a local IPv4 address and whose port is
    /// the bound port. With no such row it uses the local address toward the
    /// default route, if that is one of the host's addresses and not loopback.
    /// Several such rows, or no usable address, fail `start()`. A persisted
    /// BDT that loads is the one used, and a failure to choose from it fails
    /// `start()` without trying the configured BDT. Each start of a `0.0.0.0`
    /// BBMD repeats the choice, and the self row the BBMD appended follows it.
    /// With broadcast address 255.255.255.255 and an own address that is not
    /// the default-route address, `start()` warns that the kernel may send
    /// broadcasts from another interface; bind an explicit interface and its
    /// subnet's broadcast address instead.
    ///
    /// A BBMD can't also register as a foreign device: `start()` fails when
    /// [`register_as_foreign_device`](Self::register_as_foreign_device) was
    /// called too, even after an earlier start as a BBMD.
    pub fn enable_bbmd(&mut self, bdt: Vec<BdtEntry>) {
        self.bbmd_config = Some(BbmdConfig {
            initial_bdt: bdt,
            management_acl: Vec::new(),
            foreign_device_policy: None,
            alone_on_subnet: false,
            nat: None,
        });
    }

    /// Set foreign device registration policy on this BBMD.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn set_foreign_device_policy(&mut self, policy: ForeignDevicePolicy) {
        self.enable_foreign_device_registration(policy);
    }

    /// Set the path of a persisted BDT (wire format, 10 bytes per entry) to
    /// load at startup. Must be called before `start()`. A BDT written to
    /// the registered Network Port object and activated by
    /// ReinitializeDevice is saved there (#1826), replacing the file whole,
    /// so a restart keeps it; a failed save is logged and counted
    /// ([`bdt_persist_failures`](Self::bdt_persist_failures)) and leaves
    /// the new BDT in use. Inbound Write-BDT never updates it.
    pub fn set_bdt_persist_path(&mut self, path: std::path::PathBuf) {
        self.bdt_file = Some(Arc::new(bbmd::BdtFile::new(path)));
    }

    /// Set the management ACL for BBMD Delete-FDT-Entry.
    /// Must be called after `enable_bbmd()` and before `start()`.
    /// An empty ACL denies all Delete-FDT-Entry senders (fail closed).
    pub fn set_bbmd_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        if let Some(config) = &mut self.bbmd_config {
            config.management_acl = acl;
        } else {
            // Log a warning if called before `enable_bbmd()` so misconfiguration
            // does not fail silently.
            warn!("set_bbmd_management_acl called before enable_bbmd(); ACL will be ignored");
        }
    }

    /// Say whether this BBMD is the only device of its B/IP network on its
    /// subnet, as a BBMD behind a NAT router often is (Annex J.7.5). It then
    /// sends a peer's Forwarded-NPDU and a foreign device's
    /// Distribute-Broadcast-To-Network only to its peers and foreign
    /// devices, skipping the local rebroadcast (Annex J.4.5 permits it for
    /// the Forwarded-NPDU; this extends it to the other). Off by default;
    /// turn it on only when no other device of the network shares the
    /// subnet, since such a device would stop receiving remote broadcasts.
    /// Must be called after `enable_bbmd()` and before `start()`.
    pub fn set_bbmd_alone_on_subnet(&mut self, alone: bool) {
        if let Some(config) = &mut self.bbmd_config {
            config.alone_on_subnet = alone;
        } else {
            warn!("set_bbmd_alone_on_subnet called before enable_bbmd(); setting will be ignored");
        }
    }

    /// Configure this transport as a foreign device.
    /// Call before `start()`, which validates, before any I/O, the TTL and
    /// renewal settings and the BBMD's address (not the unspecified address,
    /// a group address or port 0), refuses a BBMD at the transport's own
    /// address once bound, and starts an automatic worker. Startup is local
    /// readiness, not a BBMD receipt; use [`Self::bvlc_client_snapshot`] to
    /// observe outcomes.
    /// `start()` fails on a transport that is also a BBMD
    /// ([`enable_bbmd`](Self::enable_bbmd)).
    ///
    /// A registered Network Port object can move it to another BBMD or
    /// time-to-live while it runs (FD_BBMD_Address and
    /// FD_Subscription_Lifetime, activated by ReinitializeDevice); a later
    /// start keeps the values it moved to.
    pub fn register_as_foreign_device(&mut self, config: ForeignDeviceConfig) {
        let target = foreign_target::ForeignTarget::new(config, self);
        self.foreign_device = Some(Arc::new(target));
    }

    /// Get the BBMD state (if BBMD mode is enabled). It appears when
    /// `start()` creates it.
    ///
    /// The lock is a synchronous [`std::sync::Mutex`]: the transport holds it
    /// only for short, synchronous critical sections, and so must every
    /// caller. Holding its guard across an await, or across a call such as
    /// [`send_broadcast`](TransportPort::send_broadcast) or a restart of a
    /// `0.0.0.0`-bound BBMD that locks it too, can stall or deadlock the
    /// transport.
    pub fn bbmd_state(&self) -> Option<&Arc<std::sync::Mutex<BbmdState>>> {
        self.bbmd.as_ref()
    }

    /// The broadcast address this transport was configured with, which its
    /// local broadcasts go to.
    pub fn broadcast_address(&self) -> Ipv4Addr {
        self.broadcast_address
    }

    /// Return the operational BBMD management counters.
    pub fn management_counters(&self) -> ManagementCounters {
        match self.management_limiter.lock() {
            Ok(limiter) => limiter.counters(),
            Err(poison) => poison.into_inner().counters(),
        }
    }

    /// Return operational Foreign Device Table counters if BBMD mode is enabled.
    pub fn fdt_counters(&self) -> Option<FdtCounters> {
        self.bbmd
            .as_ref()
            .map(|bbmd| bbmd::lock(bbmd).fdt_counters())
    }

    /// Return the operational broadcast forwarding fanout counters.
    pub fn fanout_counters(&self) -> FanoutCounters {
        self.fanout_counters.snapshot()
    }

    /// Return the datagrams the receive path dropped since this transport
    /// was created, by reason (see [`ReceiveCounters`]).
    pub fn receive_counters(&self) -> ReceiveCounters {
        self.receive_drops.snapshot()
    }

    /// Return the NPDUs this transport refused to send since it was
    /// created, by reason (see [`SendCounters`]).
    pub fn send_counters(&self) -> SendCounters {
        self.send_refusals.snapshot()
    }

    /// Set the broadcast forwarding fanout policy and rate limits.
    pub fn set_fanout_policy(&mut self, policy: FanoutPolicy) {
        let policy = policy.sanitized();
        if let Ok(mut limiter) = self.fanout_limiter.lock() {
            limiter.set_policy(policy.clone());
        }
        self.fanout_policy = policy;
    }

    /// Timeout for BVLC management response waiting.
    const BVLC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

    #[cfg(not(test))]
    const BBMD_FDT_PURGE_INTERVAL: Duration = Duration::from_secs(1);

    #[cfg(test)]
    const BBMD_FDT_PURGE_INTERVAL: Duration = Duration::from_millis(20);

    /// Get the socket, returning an error if not started.
    fn require_socket(&self) -> Result<&Arc<BipSocket>, Error> {
        self.socket.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })
    }

    fn spawn_bbmd_fdt_purge_task(bbmd: Arc<std::sync::Mutex<BbmdState>>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Self::BBMD_FDT_PURGE_INTERVAL);
            loop {
                ticker.tick().await;
                let purged = bbmd::lock(&bbmd).purge_expired();
                if purged > 0 {
                    debug!(purged, "Purged expired BBMD FDT entries");
                }
            }
        })
    }

    fn abort_background_tasks(&mut self) -> Vec<JoinHandle<()>> {
        // The node first: once it is stopped, under its own lock, a call
        // still running (the receive loop, the driver, a request given up)
        // counts nothing more, so the status cleared next stays clear.
        if let Some(node) = self.node.take() {
            node.stop();
        }
        #[cfg(test)]
        if let Some(between) = self.between_stops.take() {
            between();
        }
        self.client_record.stop();
        let mut tasks = Vec::new();
        if let Some(task) = self.node_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.bbmd_fdt_purge_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.fanout_task.take() {
            task.abort();
            tasks.push(task);
        }
        if let Some(task) = self.recv_task.take() {
            task.abort();
            tasks.push(task);
        }
        self.own_broadcast = None;
        self.socket = None;
        self.multicast.detach();
        tasks
    }

    /// The running node, or the error for a transport not started.
    fn require_node(&self) -> Result<&Arc<NodeHandle>, Error> {
        self.require_socket()?;
        self.node.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            ))
        })
    }
}

impl TransportPort for BipTransport {
    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        if self.registration_closed
            || self.socket.is_some()
            || self.recv_task.is_some()
            || self.network_port_lease.is_some()
            || self.bip_port().is_none()
        {
            return Err(Error::Encoding(
                "registered B/IP lease requires an unstarted transport in one B/IP mode".into(),
            ));
        }
        self.network_port_lease = Some(lease);
        Ok(())
    }
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }
    fn bip_port(&self) -> Option<BipPort> {
        let staged_bbmd = self.bbmd_config.is_some();
        let mode = match (&self.foreign_device, staged_bbmd || self.bbmd.is_some()) {
            // A BBMD that also registers elsewhere has no single mode.
            (Some(_), true) => return None,
            (Some(target), false) => BipPortMode::Foreign {
                registration: Arc::clone(target) as Arc<dyn ForeignRegistration>,
            },
            // Staged configuration replaces any earlier state at the next
            // start, so only started, current state is lent.
            (None, true) => BipPortMode::Bbmd {
                tables: self.bbmd.as_ref().filter(|_| !staged_bbmd).map(|state| {
                    Arc::new(bbmd::LiveTables {
                        state: Arc::clone(state),
                        file: self.bdt_file.clone(),
                        group: self.supports_multicast().then(|| self.group_change()),
                    }) as Arc<dyn BbmdTables>
                }),
            },
            (None, false) => BipPortMode::Normal,
        };
        let ip = if self.socket.is_some() {
            Ipv4Addr::new(
                self.local_mac[0],
                self.local_mac[1],
                self.local_mac[2],
                self.local_mac[3],
            )
        } else {
            self.interface
        };
        // A BBMD's group is read-only, started or not (multicast.rs).
        self.multicast.set_bbmd(staged_bbmd || self.bbmd.is_some());
        Some(BipPort {
            endpoint: SocketAddrV4::new(ip, self.port),
            mode,
            multicast: self.supports_multicast().then(|| {
                Arc::clone(&self.multicast) as Arc<dyn bacnet_types::bip_port::IpMulticast>
            }),
        })
    }
    /// The configured broadcast address at the link's port, or a B/IP-M
    /// node's group and its port (#941).
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        Some(self.broadcast_endpoint())
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.registration_closed = true;
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP transport already started",
            )));
        }

        // Validate what the configuration alone decides before probing
        // interfaces, binding sockets or sending bytes. The BDT's own checks,
        // a group BBMD's two-hop own row among them (Annex J.8.3), need the
        // own address, so they run after the bind. A node is in one B/IP mode: a BBMD distributes its own broadcasts,
        // and a foreign device hands them to its BBMD (#938).
        if self.foreign_device.is_some() && (self.bbmd_config.is_some() || self.bbmd.is_some()) {
            return Err(Error::Encoding(
                "a B/IP transport can't start as a BBMD that also registers as a foreign \
                 device; run the two on separate transports"
                    .into(),
            ));
        }
        let foreign_config = self.foreign_device_config();
        let renewal_interval = foreign_config
            .as_ref()
            .map(ForeignDeviceConfig::interval)
            .transpose()?;
        if let Some(foreign) = &foreign_config {
            foreign.check_bbmd(self.broadcast_address, self.interface)?;
        }
        let multicast = self.multicast.start_config();
        if let Some(group) = &multicast {
            self.check_multicast_start(group, foreign_config.is_some())?;
        }

        socket::probe_interface(self.interface)?;
        // A wildcard socket, or in per-address mode one bound to the interface
        // address plus, on Unix, broadcast listeners: see socket.rs (#1538).
        let plan = self.bind_plan_for_start().await;
        let listener_interface = plan.local.and_then(|local| local.index);
        let bound = socket::bind(plan).map_err(Error::Transport)?;
        let lease = self.network_port_lease.clone();
        let listeners = Listeners::open(bound, lease, &self.frame_tap).map_err(Error::Transport)?;
        let socket = Arc::clone(listeners.primary());

        let wildcard_bind = self.interface.is_unspecified();
        let (local_unicast_ips, route_ip) = if wildcard_bind {
            self.wildcard_local_ipv4().await?
        } else {
            (vec![self.interface], None)
        };

        let local_port = socket.local_addr().map_err(Error::Transport)?.port();

        // A wildcard BBMD's own address comes from its BDT (bbmd_start.rs), on
        // every start. This runs before `self` changes, so a failed start
        // keeps the BBMD configuration for a retry.
        let own_address = OwnAddressContext {
            interface: self.interface,
            port: local_port,
            host: &local_unicast_ips,
            route_ip,
        };
        // With the own address goes the mask of the subnet it is on, looked
        // up at each start: a peer's forward sent from that subnet isn't
        // broadcast there again (Annex J.4.5). A first start looks it up
        // before it consumes the configuration, so a start cancelled at the
        // lookup is retried whole.
        let bbmd_ip = if let Some(config) = &self.bbmd_config {
            let path = self.bdt_file.as_deref().map(bbmd::BdtFile::path);
            // A BBMD with a group serves it (Annex J.8.3): its own row must
            // be two-hop.
            let mut state = initial_bbmd_state(config, multicast.is_some(), path, &own_address)?;
            let ip = state.local_address().ip().octets();
            state.set_subnet_mask(subnet_mask_of(Ipv4Addr::from(ip)).await);
            self.bbmd_config = None;
            self.bbmd = Some(Arc::new(std::sync::Mutex::new(state)));
            self.multicast.set_bbmd_serves_group(multicast.is_some());
            Some(ip)
        } else if let Some(bbmd) = self.bbmd.as_ref().filter(|_| wildcard_bind) {
            // An explicit interface keeps its IP and port, and its mask,
            // across restarts. No lock is held across the lookup.
            let ip = {
                let mut state = bbmd::lock(bbmd);
                refresh_own_address(&mut state, &own_address)?;
                state.local_address().ip().octets()
            };
            let mask = subnet_mask_of(Ipv4Addr::from(ip)).await;
            bbmd::lock(bbmd).set_subnet_mask(mask);
            Some(ip)
        } else {
            // A restart may come with another group, or none: only then is
            // the state locked. The rule it holds is tracked beside it, a
            // run-time activation of the group included (#1963).
            let group = multicast.is_some();
            if let Some(bbmd) = self
                .bbmd
                .as_ref()
                .filter(|_| self.multicast.bbmd_serves_group() != group)
            {
                bbmd::lock(bbmd).set_serves_group(group)?;
                self.multicast.set_bbmd_serves_group(group);
            }
            None
        };
        if let Some(bbmd) = self.bbmd.as_ref().filter(|_| multicast.is_some()) {
            let state = bbmd::lock(bbmd);
            warn_about_peers_on_the_group_subnet(&state, state.subnet_mask());
        }
        if let Some(ip) = bbmd_ip {
            warn_if_broadcast_may_leave_another_interface(
                &own_address,
                Ipv4Addr::from(ip),
                self.broadcast_address,
            );
        }
        let local_ip = match bbmd_ip {
            Some(ip) => Ipv4Addr::from(ip),
            None if wildcard_bind => route_ip.unwrap_or(Ipv4Addr::LOCALHOST),
            None => self.interface,
        };
        if multicast.is_none()
            && broadcast_is_own_address(self.broadcast_address, local_ip, &local_unicast_ips)
        {
            warn!(
                broadcast = %self.broadcast_address,
                "B/IP broadcast address is one of this host's own addresses; \
                 broadcasts sent to it reach no other node"
            );
        }

        // The node engine: its link decides receives and sends, and it runs
        // this transport's own exchanges and a foreign device's registration.
        let mode = match (&foreign_config, &self.bbmd) {
            (Some(fd), _) => Mode::Foreign(
                fd.engine(renewal_interval.expect("validated foreign-device interval")),
            ),
            (None, Some(_)) => Mode::Bbmd,
            (None, None) => Mode::Normal,
        };
        // A B/IP-M node's group stands in for its subnet (Annex J.8.1).
        let subnet_broadcast = SocketAddrV4::new(self.broadcast_address, local_port);
        let subnet = multicast::subnet(multicast, subnet_broadcast);
        let link =
            Link::new(SocketAddrV4::new(local_ip, local_port), subnet, mode).map(|mut link| {
                // NPDUs up to the 1497 octets B/IP carries (Table 6-1, #1622).
                link.budget = Some(DATAGRAM_BUDGET);
                link
            });
        self.client_record.start();
        let node = match link.map_err(node::config_error).and_then(|link| {
            NodeHandle::new(
                link,
                Arc::clone(&socket),
                Arc::clone(&self.client_record),
                Arc::clone(&self.send_refusals),
            )
        }) {
            Ok(node) => Arc::new(node),
            Err(error) => {
                self.client_record.stop();
                return Err(error);
            }
        };

        if let (Some(target), Some(used)) = (&self.foreign_device, &foreign_config)
            && let Err(error) = target.attach(&node, used)
        {
            node.stop();
            self.client_record.stop();
            return Err(error);
        }
        self.client_record.attach(&node);
        self.port = local_port;
        self.local_mac = encode_bip_mac(local_ip.octets(), local_port);
        self.socket = Some(Arc::clone(&socket));
        self.node = Some(Arc::clone(&node));
        let group_swap = self.attach_multicast(&node, subnet_broadcast);

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (npdu_tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let (fanout_tx, fanout_rx) = mpsc::channel(self.fanout_policy.queue_capacity.max(1));
        let fanout_task = tokio::spawn(fanout::run_fanout_worker(
            Arc::clone(&socket),
            fanout_rx,
            Arc::clone(&self.fanout_counters),
        ));
        self.fanout_task = Some(fanout_task);

        let fanout_dispatcher = fanout::FanoutDispatcher::new(
            fanout_tx,
            Arc::clone(&self.fanout_limiter),
            Arc::clone(&self.fanout_counters),
        );
        // The send path shares the receive loop's dispatcher, budgets and counters.
        self.own_broadcast = self
            .bbmd
            .clone()
            .map(|bbmd| OwnBroadcastForwarder::new(bbmd, fanout_dispatcher.clone()));

        let recv_ctx = RecvContext {
            node: Arc::clone(&node),
            npdu_tx,
            bbmd: self.bbmd.clone(),
            management_limiter: Arc::clone(&self.management_limiter),
            fanout: Some(fanout_dispatcher),
            group_drops: self.group_drops.clone(),
            drops: Arc::clone(&self.receive_drops),
            #[cfg(test)]
            force_dbtn_forward_failure: false,
        };

        let ingress = IngressAddresses {
            unicast_ips: local_unicast_ips,
            wildcard_bind,
            listener_interface,
            interface_mismatch_seen: Default::default(),
        };
        self.recv_task = Some(tokio::spawn(receive_loop(
            listeners, ingress, recv_ctx, group_swap,
        )));

        if let Some(bbmd) = self.bbmd.clone() {
            self.bbmd_fdt_purge_task = Some(Self::spawn_bbmd_fdt_purge_task(bbmd));
        }

        self.node_task = Some(tokio::spawn(node.drive()));

        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        for task in self.abort_background_tasks() {
            let _ = task.await;
        }
        self.network_port_lease = None;
        Ok(())
    }

    fn abort(&mut self) {
        let _ = self.abort_background_tasks();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.require_node()?.send_unicast(npdu, mac).await
    }

    /// Broadcast `npdu` on this B/IP network.
    ///
    /// A foreign device sends it to its BBMD as Distribute-Broadcast-To-Network.
    /// Otherwise it goes out as an Original-Broadcast-NPDU, and a BBMD first
    /// queues it as a Forwarded-NPDU for its BDT peers and foreign devices
    /// (Annex J.4.5). That forward does not depend on the local send, so in
    /// BBMD mode an `Err` here can follow a forward that was already queued.
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let node = self.require_node()?;
        node.send_broadcast(npdu, self.own_broadcast.as_ref()).await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    /// The running node's [`Link::max_npdu`] for each kind of send: 1497
    /// octets each way, within the datagram budget (#1622). Before start,
    /// the default, which is the same.
    fn max_npdu(&self) -> NpduLimits {
        let Some(node) = &self.node else {
            return NpduLimits::default();
        };
        let octets = |cast| u16::try_from(node.link().max_npdu(cast)).unwrap_or(u16::MAX);
        NpduLimits {
            unicast: octets(Cast::Unicast),
            broadcast: octets(Cast::Broadcast),
        }
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        // Clause J.1.2's B/IP broadcast address is the configured broadcast
        // IP (every host bit set) together with this port's UDP
        // port — a broadcast IP at a different port belongs to a different
        // B/IP network and must not be folded into this link's broadcast.
        // A B/IP-M node's is its group and the group's port (J.8.1).
        matches!(codec::address_from_slice(mac), Ok(address)
            if address == self.broadcast_endpoint())
    }

    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.groups().contains(mac)
    }

    fn group_destinations(&self) -> crate::port::GroupDestinations {
        let groups = self.groups();
        crate::port::GroupDestinations::new(move |mac| groups.contains(mac))
    }

    /// The group destinations at this link's own UDP port (#1677): a group
    /// address at another port belongs to another B/IP network, so a send
    /// there is refused rather than sent as this link's broadcast.
    fn local_broadcast_destinations(&self) -> crate::port::GroupDestinations {
        let (groups, port) = (self.groups(), self.port);
        crate::port::GroupDestinations::new(move |mac| {
            groups.contains(mac) && codec::address_from_slice(mac).is_ok_and(|a| a.port() == port)
        })
    }
}

impl Drop for BipTransport {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod acl_tests;
#[cfg(test)]
mod arrival_order_tests;

#[cfg(test)]
mod alone_tests;
#[cfg(test)]
mod bbmd_start_tests;
#[cfg(test)]
mod bdt_persistence_tests;
#[cfg(test)]
mod dbtn_tests;
#[cfg(test)]
mod exchange_error_tests;
#[cfg(test)]
mod exchange_tests;
#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod fdt_tests;
#[cfg(test)]
mod foreign_trust_tests;
#[cfg(test)]
mod forwarded_tests;
#[cfg(test)]
mod forwarding_warn_tests;
#[cfg(test)]
mod group_delivery_tests;
#[cfg(test)]
mod group_source_tests;
#[cfg(test)]
mod group_unicast_tests;
#[cfg(test)]
mod held_broadcast_tests;
#[cfg(test)]
mod malformed_bvll_tests;
#[cfg(test)]
mod management_ack_tests;
#[cfg(test)]
mod mode_tests;
#[cfg(test)]
mod multicast_bbmd_tests;
#[cfg(test)]
mod multicast_tests;
#[cfg(test)]
mod nat_persist_tests;
#[cfg(test)]
mod npdu_addressing_tests;
#[cfg(test)]
mod original_tests;
#[cfg(test)]
mod own_broadcast_tests;
#[cfg(test)]
mod rate_limit_tests;
#[cfg(test)]
mod receive_counters_tests;
#[cfg(test)]
mod response_amplification_tests;
#[cfg(test)]
mod send_budget_tests;
#[cfg(test)]
mod send_to_tests;
#[cfg(test)]
mod shared_port_tests;
#[cfg(test)]
mod socket_tests;
#[cfg(test)]
mod tap_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unanswerable_tests;
#[cfg(test)]
mod wildcard_ingress_tests;

#[cfg(test)]
mod bdt_activation_persist_tests;
#[cfg(test)]
mod client_management_tests;
#[cfg(test)]
mod foreign_lifecycle_tests;
#[cfg(test)]
mod foreign_retarget_tests;
#[cfg(test)]
mod registration_schedule_tests;
#[cfg(test)]
mod registration_status_tests;
#[cfg(test)]
mod registration_tests;
