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
//! Lock order: the object database (held by the caller), then this
//! target's lock, then the node's. No lock here is held across an await.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::BipTransport;
use super::foreign_device::ForeignDeviceConfig;
use super::node::NodeHandle;

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
    /// device's entry ([`BipTransport::set_release_previous_bbmd`]).
    release_previous: AtomicBool,
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
    /// release setting of the target it replaces, if any.
    pub(super) fn new(config: ForeignDeviceConfig, transport: &BipTransport) -> Self {
        let release_previous = transport
            .foreign_device
            .as_ref()
            .is_some_and(|target| target.release_previous.load(Ordering::Relaxed));
        Self {
            state: Mutex::new(TargetState {
                configured_renewal: config.renewal_interval,
                config,
                node: Weak::new(),
            }),
            broadcast: transport.broadcast_address,
            interface: transport.interface,
            release_previous: AtomicBool::new(release_previous),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TargetState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registration in effect.
    pub(super) fn config(&self) -> ForeignDeviceConfig {
        self.lock().config.clone()
    }

    /// Change the configuration in a test, as a caller with the transport
    /// could before the target existed.
    #[cfg(test)]
    pub(super) fn edit(&self, edit: impl FnOnce(&mut ForeignDeviceConfig)) {
        edit(&mut self.lock().config);
    }

    /// Hand the target the node a start built from `used`. A registration
    /// activated while the start ran goes to the node now, so it never runs
    /// on values the object no longer reports.
    pub(super) fn attach(
        &self,
        node: &Arc<NodeHandle>,
        used: &ForeignDeviceConfig,
    ) -> Result<(), Error> {
        let mut state = self.lock();
        if state.config != *used {
            let interval = state.config.interval()?;
            node.retarget(state.config.engine(interval))?;
        }
        state.node = Arc::downgrade(node);
        Ok(())
    }

    /// The running node: attached, and not stopped.
    fn running(state: &TargetState) -> Option<Arc<NodeHandle>> {
        state.node.upgrade().filter(|node| !node.is_stopped())
    }

    /// The registration `changes` would make on the running node, checked
    /// as a start checks one, and against the node's own address. Changes
    /// nothing.
    fn changed(
        &self,
        state: &TargetState,
        changes: &ForeignDeviceChanges,
    ) -> Result<(Arc<NodeHandle>, ForeignDeviceConfig), Error> {
        let node = Self::running(state).ok_or_else(not_running)?;
        let mut config = state.config.clone();
        if let Some(bbmd) = &changes.bbmd {
            // A B/IP address: an IPv4 host and a port. This device resolves
            // no host names, as for the BDT rows (#1578), and 'none' names
            // no BBMD.
            let BACnetHostAddress::Ip(IpAddr::V4(ip)) = bbmd.host else {
                return Err(out_of_range());
            };
            config.bbmd_ip = ip;
            config.bbmd_port = bbmd.port;
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
        let interval = config.interval().map_err(|_| out_of_range())?;
        config
            .check_bbmd(self.broadcast, self.interface)
            .map_err(|_| out_of_range())?;
        node.check_retarget(config.engine(interval))
            .map_err(|_| out_of_range())?;
        Ok((node, config))
    }
}

impl ForeignRegistration for ForeignTarget {
    fn bbmd(&self) -> BACnetHostNPort {
        let config = &self.lock().config;
        BACnetHostNPort::from_socket_addr(SocketAddr::V4(SocketAddrV4::new(
            config.bbmd_ip,
            config.bbmd_port,
        )))
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let state = self.lock();
        self.changed(&state, changes).map(drop)
    }

    /// With [`BipTransport::set_release_previous_bbmd`] on, a move to
    /// another BBMD also asks the old one to delete this device's entry,
    /// on the side: whatever comes of that, the move stands (#1852).
    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        let (node, config) = self.changed(&state, changes)?;
        let interval = config.interval().map_err(|_| out_of_range())?;
        node.retarget(config.engine(interval))
            .map_err(|_| out_of_range())?;
        let old = SocketAddrV4::new(state.config.bbmd_ip, state.config.bbmd_port);
        let moved = old != SocketAddrV4::new(config.bbmd_ip, config.bbmd_port);
        state.config = config;
        drop(state);
        if moved && self.release_previous.load(Ordering::Relaxed) {
            node.release_previous_bbmd(old);
        }
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}

impl BipTransport {
    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. `None` outside
    /// foreign-device mode.
    pub fn foreign_device_config(&self) -> Option<ForeignDeviceConfig> {
        self.foreign_device.as_ref().map(|target| target.config())
    }

    /// After a Network Port activation moves this foreign device to another
    /// BBMD, also ask the BBMD it left to delete its entry
    /// (Delete-Foreign-Device-Table-Entry, Annex J.2.9) rather than leave it
    /// until its time-to-live and grace period run out (J.5.2.3, #1852).
    /// Best effort: the request goes on the side, never holds up or fails
    /// the move, and its outcome is counted in the snapshot's
    /// `previous_release*` fields.
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
