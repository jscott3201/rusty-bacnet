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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

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
}

/// The refusal of a change while no node runs: nothing could check it
/// against the device's own address, or put it into effect.
fn not_running() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::WRITE_ACCESS_DENIED,
    }
}

/// The WriteProperty refusal of a value the foreign device can't register
/// with.
fn out_of_range() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::VALUE_OUT_OF_RANGE,
    }
}

impl ForeignTarget {
    pub(super) fn new(
        config: ForeignDeviceConfig,
        broadcast: Ipv4Addr,
        interface: Ipv4Addr,
    ) -> Self {
        Self {
            state: Mutex::new(TargetState {
                configured_renewal: config.renewal_interval,
                config,
                node: Weak::new(),
            }),
            broadcast,
            interface,
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

    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        let (node, config) = self.changed(&state, changes)?;
        let interval = config.interval().map_err(|_| out_of_range())?;
        node.retarget(config.engine(interval))
            .map_err(|_| out_of_range())?;
        state.config = config;
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}

impl super::BipTransport {
    /// The foreign-device registration in effect: the one configured, or
    /// the one a Network Port activation moved it to. `None` outside
    /// foreign-device mode.
    pub fn foreign_device_config(&self) -> Option<ForeignDeviceConfig> {
        self.foreign_device.as_ref().map(|target| target.config())
    }
}
