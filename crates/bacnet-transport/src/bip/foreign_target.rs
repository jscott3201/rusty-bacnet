//! A foreign device's registration target, which the Network Port object
//! reads and changes: FD_BBMD_Address and FD_Subscription_Lifetime (Clause
//! 12.56.37, 12.56.38, #1825).
//!
//! The transport, the object's live view and the running node share one
//! [`ForeignTarget`]. A written BBMD or time-to-live is checked when it is
//! written and again before a ReinitializeDevice handler runs; once
//! activated, the running node registers with the new BBMD from then on
//! ([`NodeHandle::retarget`]), and a later start uses the new values too.
//! Only a running transport takes a change: its node knows its own bound
//! address, which a new BBMD must not be, and a stopped one doesn't.
//!
//! FD_BBMD_Address may name the BBMD by host name (#1852): the node then
//! registers with the address the name last resolved to, and [`lookup`]
//! looks it up again at each activation and alongside the registration
//! attempts, at most once per 30 seconds, on the policy
//! [`crate::bbmd_lookup`] shares with B/IPv6.
//!
//! Lock order: the object database (held by the caller), then this
//! target's lock, then the node's. No lock here is held across an await.

mod lookup;
#[cfg(test)]
mod name_tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_bip_core::node::Foreign;
use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::BipTransport;
use super::ForeignBbmd;
use super::foreign_device::ForeignDeviceConfig;
use super::node::NodeHandle;
use crate::bbmd_lookup::{BbmdName, BbmdResolver, Lookups};

/// What the target's lock guards.
struct TargetState {
    /// The registration in effect, and the one the next start uses.
    config: ForeignDeviceConfig,
    /// The renewal interval the owner configured, from which each new
    /// lifetime's effective interval is worked out, so a short lifetime
    /// doesn't shorten it for good.
    configured_renewal: Option<Duration>,
    /// The running node, while one runs.
    node: Weak<NodeHandle>,
    /// FD_BBMD_Address as a host name, when it names one, configured or
    /// written, and its lookups; `config` then holds the address in use,
    /// the one the name last resolved to, or the BBMD before it, until it
    /// first does. A configured name that hasn't resolved yet leaves
    /// `config` naming it, and the node waiting for a BBMD (#2308).
    lookups: Lookups,
    /// A lookup is to run again 30 seconds after one failed while the
    /// node waits for its BBMD ([`ForeignTarget::retry_later`]).
    retry_pending: bool,
}

/// One transport's foreign-device registration, shared with the Network
/// Port object it is lent to.
pub(crate) struct ForeignTarget {
    state: Mutex<TargetState>,
    /// The transport's broadcast address and interface, against which a new
    /// BBMD is checked as at start ([`ForeignDeviceConfig::check_bbmd`]).
    broadcast: Ipv4Addr,
    interface: Ipv4Addr,
    /// After a move to another BBMD, ask the old one to delete this
    /// device's entry ([`BipTransport::set_release_previous_bbmd`]): set on
    /// each registration handed to the node.
    release_previous: AtomicBool,
    /// This target, for the lookups it starts.
    me: Weak<ForeignTarget>,
}

/// What written values would put into effect on the running node.
struct Planned {
    node: Arc<NodeHandle>,
    /// The registration to run with now.
    config: ForeignDeviceConfig,
    /// The BBMD row's new value, when it is among the changes: a host name
    /// to look up, or `None` for an address, which `config` holds.
    name: Option<Option<BbmdName>>,
}

/// The refusal of a change while no node runs: nothing could check it
/// against the device's own address, or put it into effect.
fn not_running() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
}

/// The WriteProperty refusal of a value the foreign device can't register
/// with.
fn out_of_range() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
}

impl ForeignTarget {
    /// The target `transport` registers with `config`, keeping the
    /// release setting of the target it replaces, if any, and looking a
    /// BBMD host name up with the transport's resolver.
    pub(super) fn new(config: ForeignDeviceConfig, transport: &BipTransport) -> Arc<Self> {
        let release_previous = transport
            .foreign_device
            .as_ref()
            .is_some_and(|target| target.release_previous.load(Ordering::Relaxed));
        let name = config.bbmd.name();
        Arc::new_cyclic(|me| Self {
            state: Mutex::new(TargetState {
                configured_renewal: config.renewal_interval,
                config,
                node: Weak::new(),
                lookups: Lookups::new(Arc::clone(&transport.bbmd_resolver), name),
                retry_pending: false,
            }),
            broadcast: transport.broadcast_address,
            interface: transport.interface,
            release_previous: AtomicBool::new(release_previous),
            me: me.clone(),
        })
    }

    /// Look BBMD host names up with `resolver` from now on.
    pub(super) fn set_resolver(&self, resolver: Arc<dyn BbmdResolver>) {
        self.lock().lookups.set_resolver(resolver);
    }

    fn lock(&self) -> MutexGuard<'_, TargetState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registration in effect.
    pub(super) fn config(&self) -> ForeignDeviceConfig {
        self.lock().config.clone()
    }

    /// `config` as the node engine's registration, which, with
    /// [`BipTransport::set_release_previous_bbmd`] on, releases the BBMD a
    /// move to it leaves (#2307).
    fn engine(&self, config: &ForeignDeviceConfig) -> Result<Foreign, Error> {
        let mut foreign = config.engine(config.interval()?);
        foreign.release_previous = self.release_previous.load(Ordering::Relaxed);
        Ok(foreign)
    }

    /// Change the configuration in a test, as a caller with the transport
    /// could before the target existed.
    #[cfg(test)]
    pub(super) fn edit(&self, edit: impl FnOnce(&mut ForeignDeviceConfig)) {
        edit(&mut self.lock().config);
    }

    /// Hand the target the node a start built from `used`. A registration
    /// activated while the start ran goes to the node now, so it never runs
    /// on values the object no longer reports. Each automatic registration
    /// attempt the node sends looks a BBMD host name up again; a node
    /// waiting for a BBMD whose name hasn't resolved yet sends none, so its
    /// name is looked up now (#2308).
    pub(super) fn attach(
        &self,
        node: &Arc<NodeHandle>,
        used: &ForeignDeviceConfig,
    ) -> Result<(), Error> {
        let mut state = self.lock();
        if state.config != *used {
            node.retarget(self.engine(&state.config)?)?;
        }
        state.node = Arc::downgrade(node);
        state.lookups.attached();
        let target = self.me.clone();
        node.on_registration_attempt(Box::new(move || {
            if let Some(target) = target.upgrade() {
                target.attempting();
            }
        }));
        let lookup = state
            .config
            .bbmd()
            .is_none()
            .then(|| state.lookups.lookup(false, &self.name_target()))
            .flatten();
        drop(state);
        if let Some(lookup) = lookup {
            lookup.start();
        }
        Ok(())
    }

    /// The running node: attached, and not stopped.
    fn running(state: &TargetState) -> Option<Arc<NodeHandle>> {
        state.node.upgrade().filter(|node| !node.is_stopped())
    }

    /// What `changes` would put into effect on the running node, checked
    /// as a start checks a registration, and against the node's own
    /// address. Changes nothing. A host name is checked for its form only:
    /// it is looked up once activated.
    fn changed(
        &self,
        state: &TargetState,
        changes: &ForeignDeviceChanges,
    ) -> Result<Planned, Error> {
        let node = Self::running(state).ok_or_else(not_running)?;
        let mut config = state.config.clone();
        let mut name = None;
        if let Some(bbmd) = &changes.bbmd {
            match &bbmd.host {
                BACnetHostAddress::Ip(IpAddr::V4(ip)) => {
                    config.bbmd = ForeignBbmd::Address(SocketAddrV4::new(*ip, bbmd.port));
                    name = Some(None);
                }
                // Until it resolves, the device keeps registering with the
                // address it has (#1852).
                BACnetHostAddress::Name(host) => {
                    let named = BbmdName::new(host, bbmd.port).ok_or_else(out_of_range)?;
                    name = Some(Some(named));
                }
                // A B/IP address is an IPv4 host and a port; 'none' names
                // no BBMD.
                BACnetHostAddress::Ip(IpAddr::V6(_)) | BACnetHostAddress::None => {
                    return Err(out_of_range());
                }
            }
        }
        if let Some(ttl) = changes.subscription_lifetime {
            config.ttl = ttl;
            // A client can't see the configured renewal interval, so a new
            // lifetime cuts it to half the lifetime rather than being
            // refused for it, working from the configured value each time.
            let half = Duration::from_secs(ttl.into()) / 2;
            config.renewal_interval = state.configured_renewal.map(|renewal| renewal.min(half));
        }
        // A zero time-to-live is refused, as at start.
        let foreign = self.engine(&config).map_err(|_| out_of_range())?;
        config
            .check_bbmd(self.broadcast, self.interface)
            .map_err(|_| out_of_range())?;
        node.check_retarget(foreign).map_err(|_| out_of_range())?;
        Ok(Planned { node, config, name })
    }
}

impl ForeignRegistration for ForeignTarget {
    /// The host name and port written, while the BBMD is known by name;
    /// the address otherwise.
    fn bbmd(&self) -> BACnetHostNPort {
        let state = self.lock();
        match (state.lookups.name(), state.config.bbmd()) {
            (Some(name), _) => name.value(),
            (None, Some(bbmd)) => BACnetHostNPort::from_socket_addr(SocketAddr::V4(bbmd)),
            // A configured name always has its lookup's; this can't be.
            (None, None) => BACnetHostNPort {
                host: BACnetHostAddress::None,
                port: 0,
            },
        }
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let state = self.lock();
        self.changed(&state, changes).map(drop)
    }

    /// With [`BipTransport::set_release_previous_bbmd`] on, a move to
    /// another BBMD also has the node ask the old one to delete this
    /// device's entry, after the first registration with the new one:
    /// whatever comes of that, the move stands (#1852, #2307).
    ///
    /// A host name is looked up on the side (#1852), and the device moves
    /// once it resolves; a name alone, with the time-to-live as it was,
    /// sends nothing to the BBMD in use before then.
    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        let Planned { node, config, name } = self.changed(&state, changes)?;
        // Every activation registers again at once, unless it only names
        // the BBMD by a host name, which registers once looked up.
        let reregister = !matches!(name, Some(Some(_))) || config != state.config;
        if reregister {
            let foreign = self.engine(&config).map_err(|_| out_of_range())?;
            node.retarget(foreign).map_err(|_| out_of_range())?;
        }
        state.config = config;
        let lookup = name.and_then(|name| self.renamed(&mut state, name));
        drop(state);
        if let Some(lookup) = lookup {
            lookup.start();
        }
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}

impl BipTransport {
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
    /// start keeps the values it moved to. FD_BBMD_Address, and the
    /// configured BBMD ([`ForeignBbmd::Name`], #2308), may name the BBMD by
    /// host name ([`set_bbmd_resolver`](Self::set_bbmd_resolver)); a
    /// configured name that hasn't resolved yet leaves the transport running
    /// with registration waiting, as [`ForeignBbmd`] says.
    pub fn register_as_foreign_device(&mut self, config: ForeignDeviceConfig) {
        self.foreign_device = Some(ForeignTarget::new(config, self));
    }

    /// Look up the host name a foreign device's FD_BBMD_Address, or its
    /// configured BBMD ([`ForeignBbmd::Name`], #2308), names with
    /// `resolver`, instead of the system's resolver on Tokio's blocking
    /// pool, the default (#1852). Call it before or after
    /// [`register_as_foreign_device`](Self::register_as_foreign_device).
    ///
    /// A host name written to FD_BBMD_Address is checked for its form, and
    /// looked up whenever ReinitializeDevice activates it, on the side: the
    /// activation never waits for it. The device moves to the first address
    /// it can register with, as an activated address would move it. The
    /// name is looked up again alongside the registration attempts (the
    /// first after a start, renewals and retries), at most once every 30
    /// seconds: such an attempt still goes to the address in use, and a
    /// lookup that finds the BBMD elsewhere moves the device, which
    /// registers there at once. So a writer setting a 1-second lifetime
    /// drives one query every 30 seconds, not two a second. One resolver
    /// call runs at a time: a name activated while one runs is looked up
    /// when it ends. While the address in use is still among the answers
    /// the device stays put, so a name with several addresses doesn't move
    /// it. A lookup that fails, takes longer than 10 seconds, finds only
    /// addresses the device can't register with, or ends without an
    /// answer leaves it registering with the address it had (before the
    /// first success, the BBMD it registered with before the name), and is
    /// logged and counted in the snapshot's `bbmd_lookups`,
    /// `bbmd_lookup_failures` and `last_bbmd_lookup_failure`.
    pub fn set_bbmd_resolver(&mut self, resolver: Arc<dyn BbmdResolver>) {
        if let Some(target) = &self.foreign_device {
            target.set_resolver(Arc::clone(&resolver));
        }
        self.bbmd_resolver = resolver;
    }

    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. While FD_BBMD_Address
    /// names the BBMD by host name, its address is the one the name last
    /// resolved to, or the BBMD before it until it first does (#1852). A
    /// configured name reads back as the name until it first resolves, and
    /// as the address in use from then on (#2308). `None` outside
    /// foreign-device mode.
    pub fn foreign_device_config(&self) -> Option<ForeignDeviceConfig> {
        self.foreign_device.as_ref().map(|target| target.config())
    }

    /// After a Network Port activation moves this foreign device to another
    /// BBMD, or the BBMD's host name comes to resolve elsewhere (#1852),
    /// also ask the BBMD it left to delete its entry
    /// (Delete-Foreign-Device-Table-Entry, Annex J.2.9) rather than leave it
    /// until its time-to-live and grace period run out (J.5.2.3, #1852).
    /// The node engine runs it
    /// ([`Foreign::release_previous`](bacnet_bip_core::node::Foreign::release_previous),
    /// #2307): one request, sent after the first registration with the new
    /// BBMD, and given up after the old registration's response timeout (3
    /// seconds, or its renewal interval when shorter). Best effort: it
    /// never holds up or fails the move, and its outcome is counted in the
    /// snapshot's `previous_release*` fields.
    ///
    /// The request names this device's own local address (on a wildcard
    /// bind, the default-route address). Behind a NAT router, or on a
    /// multi-homed host whose registration left from another address, the
    /// old BBMD knows the device by another address and will usually refuse
    /// (counted as refused); in a rare case it could delete another
    /// device's entry registered from the same private address and port on
    /// the BBMD's side. That, a BBMD refusing deletions it doesn't trust
    /// (this stack's does unless its management access list names the
    /// sender), and the entry lapsing on its own anyway are why it is off
    /// by default.
    ///
    /// Call after [`register_as_foreign_device`](Self::register_as_foreign_device),
    /// which keeps the setting when called again ([`set_mode`](Self::set_mode)
    /// turns it off); outside foreign-device mode it is ignored, with a
    /// warning.
    pub fn set_release_previous_bbmd(&mut self, release: bool) {
        match &self.foreign_device {
            Some(target) => target.release_previous.store(release, Ordering::Relaxed),
            None => tracing::warn!(
                "set_release_previous_bbmd called before register_as_foreign_device(); \
                 setting will be ignored"
            ),
        }
    }
}
