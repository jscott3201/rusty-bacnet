//! A B/IPv6 foreign device's registration target, which the Network Port
//! object reads and changes: FD_BBMD_Address and FD_Subscription_Lifetime
//! (Clause 12.56.37, 12.56.38; Table 12-71 footnote 13 applies them to
//! IPV6 ports), as B/IP's `ForeignTarget` does for B/IP (#1825, #1579).
//!
//! The transport, its receive loop, its resolution driver, its broadcasts,
//! its re-registration task and the object's live view all read one
//! [`ForeignTarget`], so an activated BBMD or time-to-live takes effect
//! everywhere at once: the re-registration task registers with the new
//! values straight away, and a later start uses them too. Only a running
//! transport takes a change, since only it knows its own bound address,
//! which a new BBMD must not be.
//!
//! Lock order: the object database (held by the caller), then this
//! target's lock. The lock is a short synchronous critical section, never
//! held across an await.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::Notify;

use super::port::Bip6ForeignDeviceConfig;

/// What the target's lock guards.
struct State {
    /// The registration in effect, and the one the next start uses.
    config: Bip6ForeignDeviceConfig,
    /// This node's bound address while the transport runs.
    own: Option<SocketAddrV6>,
}

/// One transport's foreign-device registration, shared with the Network
/// Port object it is lent to.
pub(crate) struct ForeignTarget {
    state: Mutex<State>,
    /// Wakes the re-registration task when an activation changes the
    /// registration.
    changed: Notify,
    /// Holds a re-registration after it has read the target and before it
    /// sends, so a test can land an activation while one is in flight.
    #[cfg(test)]
    pub(super) gate: tokio::sync::Semaphore,
    /// Signalled when a re-registration reaches [`Self::gate`].
    #[cfg(test)]
    pub(super) reached: Notify,
}

/// Why a [`Bip6ForeignDeviceConfig`] can't register: see
/// [`Bip6ForeignDeviceConfig::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bip6ForeignDeviceRefusal {
    /// The BBMD address is `::`.
    Unspecified,
    /// The BBMD address is a multicast group.
    Multicast,
    /// The BBMD address is link-local (`fe80::/10`).
    LinkLocal,
    /// The BBMD address is IPv4-mapped (`::ffff:a.b.c.d`).
    Ipv4Mapped,
    /// The BBMD address is the deprecated IPv4-compatible form
    /// (`::a.b.c.d`, other than `::` and `::1`).
    Ipv4Compatible,
    /// The BBMD port is 0.
    ZeroPort,
    /// The time-to-live is 0.
    ZeroTtl,
}

impl std::fmt::Display for Bip6ForeignDeviceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unspecified => "the BBMD address is unspecified (::)",
            Self::Multicast => "the BBMD address is a multicast group",
            Self::LinkLocal => "the BBMD address is link-local",
            Self::Ipv4Mapped => "the BBMD address is IPv4-mapped; B/IP serves IPv4 hosts",
            Self::Ipv4Compatible => {
                "the BBMD address is IPv4-compatible (::a.b.c.d); B/IP serves IPv4 hosts"
            }
            Self::ZeroPort => "the BBMD port is 0",
            Self::ZeroTtl => "the time-to-live is 0",
        })
    }
}

impl std::error::Error for Bip6ForeignDeviceRefusal {}

impl Bip6ForeignDeviceConfig {
    /// Check that this registration can work: the BBMD is an IPv6 unicast
    /// host (loopback, unique local or global unicast) with a nonzero port,
    /// and the time-to-live is nonzero.
    ///
    /// A link-local BBMD is refused: no zone index travels with an address,
    /// so its Forwarded-NPDUs could never be trusted. The IPv4-mapped and
    /// IPv4-compatible forms name IPv4 hosts, which B/IP serves. The
    /// transport's start refuses a registration that fails this check, and
    /// so does a Network Port activation.
    pub fn check(&self) -> Result<(), Bip6ForeignDeviceRefusal> {
        let ip = self.bbmd_ip;
        let ipv4_compatible = ip.segments()[..6] == [0; 6] && ip.segments()[6] != 0;
        let refusal = if ip.is_unspecified() {
            Bip6ForeignDeviceRefusal::Unspecified
        } else if ip.is_multicast() {
            Bip6ForeignDeviceRefusal::Multicast
        } else if ip.is_unicast_link_local() {
            Bip6ForeignDeviceRefusal::LinkLocal
        } else if ip.to_ipv4_mapped().is_some() {
            Bip6ForeignDeviceRefusal::Ipv4Mapped
        } else if ipv4_compatible {
            Bip6ForeignDeviceRefusal::Ipv4Compatible
        } else if self.bbmd_port == 0 {
            Bip6ForeignDeviceRefusal::ZeroPort
        } else if self.ttl == 0 {
            Bip6ForeignDeviceRefusal::ZeroTtl
        } else {
            return Ok(());
        };
        Err(refusal)
    }
}

fn property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

impl ForeignTarget {
    pub(super) fn new(config: Bip6ForeignDeviceConfig) -> Self {
        Self {
            state: Mutex::new(State { config, own: None }),
            changed: Notify::new(),
            #[cfg(test)]
            gate: tokio::sync::Semaphore::new(tokio::sync::Semaphore::MAX_PERMITS),
            #[cfg(test)]
            reached: Notify::new(),
        }
    }

    /// In a test, wait at [`Self::gate`] as a re-registration in flight.
    #[cfg(test)]
    pub(super) async fn in_flight(&self) {
        self.reached.notify_one();
        drop(self.gate.acquire().await);
    }

    /// A target for `bbmd`, for a test's receive loop or driver.
    #[cfg(test)]
    pub(super) fn at(bbmd: SocketAddrV6) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::new(Bip6ForeignDeviceConfig {
            bbmd_ip: *bbmd.ip(),
            bbmd_port: bbmd.port(),
            ttl: 60,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The registration in effect.
    pub(super) fn config(&self) -> Bip6ForeignDeviceConfig {
        self.lock().config.clone()
    }

    /// The BBMD's address and port, as the receive path checks a sender.
    pub(super) fn bbmd(&self) -> (Ipv6Addr, u16) {
        let state = self.lock();
        (state.config.bbmd_ip, state.config.bbmd_port)
    }

    /// The BBMD as a destination.
    pub(super) fn bbmd_addr(&self) -> SocketAddrV6 {
        let (ip, port) = self.bbmd();
        SocketAddrV6::new(ip, port, 0, 0)
    }

    /// How long until the next re-registration: half the time-to-live, and
    /// never under 30 s.
    pub(super) fn renewal_interval(&self) -> Duration {
        Duration::from_secs((u64::from(self.lock().config.ttl) / 2).max(30))
    }

    /// The transport runs, bound to `own`: changes are taken from now on.
    pub(super) fn attach(&self, own: SocketAddrV6) {
        self.lock().own = Some(own);
    }

    /// The transport stopped: changes are refused until it runs again.
    pub(super) fn detach(&self) {
        self.lock().own = None;
    }

    /// Resolve when an activation changed the registration.
    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    /// The registration `changes` would make, checked against this node's
    /// own address. Changes nothing.
    ///
    /// A registration passes [`Bip6ForeignDeviceConfig::check`] when its
    /// BBMD is an IPv6 unicast host; that BBMD must also be other than this
    /// node and one this host's routes reach from the socket's bound
    /// address. IPv4 hosts and host names are refused.
    fn changed_config(
        state: &State,
        changes: &ForeignDeviceChanges,
    ) -> Result<Bip6ForeignDeviceConfig, Error> {
        let own = state
            .own
            .ok_or_else(|| property_error(ErrorCode::WRITE_ACCESS_DENIED))?;
        let out_of_range = || property_error(ErrorCode::VALUE_OUT_OF_RANGE);
        let mut config = state.config.clone();
        if let Some(bbmd) = &changes.bbmd {
            // A B/IPv6 address: an IPv6 host and a port. This device
            // resolves no host names, and 'none' names no BBMD.
            let BACnetHostAddress::Ip(IpAddr::V6(ip)) = bbmd.host else {
                return Err(out_of_range());
            };
            config.bbmd_ip = ip;
            config.bbmd_port = bbmd.port;
        }
        if let Some(ttl) = changes.subscription_lifetime {
            config.ttl = ttl;
        }
        let own_address = config.bbmd_ip == *own.ip() && config.bbmd_port == own.port();
        if config.check().is_err() || own_address {
            return Err(out_of_range());
        }
        // The socket stays bound to its address, so the new BBMD must be one
        // this host reaches from it: a local probe, nothing is sent.
        let bbmd = SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0);
        match super::socket::route_source(bbmd) {
            Ok(source) if source == *own.ip() => Ok(config),
            _ => Err(out_of_range()),
        }
    }
}

impl ForeignRegistration for ForeignTarget {
    fn bbmd(&self) -> BACnetHostNPort {
        BACnetHostNPort::from_socket_addr(SocketAddr::V6(self.bbmd_addr()))
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        Self::changed_config(&self.lock(), changes).map(drop)
    }

    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        state.config = Self::changed_config(&state, changes)?;
        drop(state);
        // One permit is stored if the task isn't waiting yet, so the change
        // is never missed.
        self.changed.notify_one();
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}
