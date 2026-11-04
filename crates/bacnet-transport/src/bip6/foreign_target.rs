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
//! FD_BBMD_Address may name the BBMD by host name (#2308): the device then
//! registers with the IPv6 address the name last resolved to, looked up at
//! each activation and alongside the registration attempts, at most once
//! per 30 seconds, on the policy [`crate::bbmd_lookup`] shares with B/IP
//! ([`lookup`]).
//!
//! Lock order: the object database (held by the caller), then this
//! target's lock. The lock is a short synchronous critical section, never
//! held across an await.

mod lookup;
#[cfg(test)]
mod name_tests;
mod release;

pub use release::Bip6ReleaseCounters;

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_types::bip_port::{ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use tokio::sync::Notify;

use super::port::Bip6ForeignDeviceConfig;
use crate::bbmd_lookup::{BbmdLookupCounters, BbmdName, BbmdResolver, Lookups};

/// What the target's lock guards.
struct State {
    /// The registration in effect, and the one the next start uses. While
    /// FD_BBMD_Address names the BBMD by host name, its address is the one
    /// the name last resolved to, or the BBMD before it until it first
    /// does.
    config: Bip6ForeignDeviceConfig,
    /// This node's bound address while the transport runs.
    own: Option<SocketAddrV6>,
    /// FD_BBMD_Address as a host name, when it names one, and its lookups.
    lookups: Lookups,
    /// What came of those lookups.
    counters: BbmdLookupCounters,
    /// Release the BBMD a move leaves (#2338, [`release`]).
    release_previous: bool,
    /// The BBMD left by the latest move, for the next re-registration pass
    /// to release.
    queued: Option<SocketAddrV6>,
    /// The release sent and not yet answered.
    awaiting: Option<release::Awaiting>,
    /// What came of the releases.
    releases: Bip6ReleaseCounters,
}

/// One transport's foreign-device registration, shared with the Network
/// Port object it is lent to.
pub(crate) struct ForeignTarget {
    state: Mutex<State>,
    /// Wakes the re-registration task when an activation, or a lookup of
    /// the BBMD's host name, changes the registration.
    changed: Notify,
    /// This target, for the lookups it starts.
    me: Weak<ForeignTarget>,
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
    /// The target that registers with `config`, looking a BBMD host name
    /// up with `resolver`.
    pub(super) fn new(
        config: Bip6ForeignDeviceConfig,
        resolver: Arc<dyn BbmdResolver>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            state: Mutex::new(State {
                config,
                own: None,
                lookups: Lookups::new(resolver, None),
                counters: BbmdLookupCounters::default(),
                release_previous: false,
                queued: None,
                awaiting: None,
                releases: Bip6ReleaseCounters::default(),
            }),
            changed: Notify::new(),
            me: me.clone(),
            #[cfg(test)]
            gate: tokio::sync::Semaphore::new(tokio::sync::Semaphore::MAX_PERMITS),
            #[cfg(test)]
            reached: Notify::new(),
        })
    }

    /// Look BBMD host names up with `resolver` from now on.
    pub(super) fn set_resolver(&self, resolver: Arc<dyn BbmdResolver>) {
        self.lock().lookups.set_resolver(resolver);
    }

    /// What came of the lookups of the BBMD's host name.
    pub(super) fn lookup_counters(&self) -> BbmdLookupCounters {
        self.lock().counters
    }

    /// In a test, wait at [`Self::gate`] as a re-registration in flight.
    #[cfg(test)]
    pub(super) async fn in_flight(&self) {
        self.reached.notify_one();
        drop(self.gate.acquire().await);
    }

    /// A target for `bbmd`, for a test's receive loop or driver.
    #[cfg(test)]
    pub(super) fn at(bbmd: SocketAddrV6) -> Arc<Self> {
        let config = Bip6ForeignDeviceConfig {
            bbmd_ip: *bbmd.ip(),
            bbmd_port: bbmd.port(),
            ttl: 60,
        };
        Self::new(config, Arc::new(crate::bbmd_lookup::SystemResolver))
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

    /// The transport runs, bound to `own`: changes are taken from now on,
    /// and the next registration attempt looks a BBMD host name up again,
    /// whenever the last lookup was.
    pub(super) fn attach(&self, own: SocketAddrV6) {
        let mut state = self.lock();
        state.own = Some(own);
        state.lookups.attached();
        state.counters.last_failure = None;
    }

    /// The transport stopped: changes are refused until it runs again, a
    /// lookup that ends meanwhile counts nothing, and a release still out
    /// ends unanswered.
    pub(super) fn detach(&self) {
        let mut state = self.lock();
        state.own = None;
        state.counters.last_failure = None;
        state.end_releases();
    }

    /// Resolve when an activation changed the registration.
    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    /// The registration `changes` would make, checked against this node's
    /// own address, and the BBMD row's new value when it is among them: a
    /// host name to look up, or `None` for an address, which the
    /// registration holds. Changes nothing.
    ///
    /// A registration passes [`Bip6ForeignDeviceConfig::check`] when its
    /// BBMD is an IPv6 unicast host; that BBMD must also be other than this
    /// node and one this host's routes reach from the socket's bound
    /// address ([`Self::usable`]). IPv4 hosts are refused; a host name is
    /// checked for its form only, and looked up once activated (#2308).
    fn changed_config(
        state: &State,
        changes: &ForeignDeviceChanges,
    ) -> Result<(Bip6ForeignDeviceConfig, Option<Option<BbmdName>>), Error> {
        let own = state
            .own
            .ok_or_else(|| property_error(ErrorCode::WRITE_ACCESS_DENIED))?;
        let out_of_range = || property_error(ErrorCode::VALUE_OUT_OF_RANGE);
        let mut config = state.config.clone();
        let mut name = None;
        if let Some(bbmd) = &changes.bbmd {
            match &bbmd.host {
                // A B/IPv6 address: an IPv6 host and a port.
                BACnetHostAddress::Ip(IpAddr::V6(ip)) => {
                    config.bbmd_ip = *ip;
                    config.bbmd_port = bbmd.port;
                    name = Some(None);
                }
                // Until it resolves, the device keeps registering with the
                // address it has.
                BACnetHostAddress::Name(host) => {
                    let named = BbmdName::new(host, bbmd.port).ok_or_else(out_of_range)?;
                    name = Some(Some(named));
                }
                // 'none' names no BBMD.
                BACnetHostAddress::Ip(IpAddr::V4(_)) | BACnetHostAddress::None => {
                    return Err(out_of_range());
                }
            }
        }
        if let Some(ttl) = changes.subscription_lifetime {
            config.ttl = ttl;
        }
        if !Self::usable(own, &config) {
            return Err(out_of_range());
        }
        Ok((config, name))
    }

    /// Whether a device bound to `own` can register as `config` says: its
    /// BBMD passes [`Bip6ForeignDeviceConfig::check`], isn't the device
    /// itself, and is one this host's routes reach from `own`. The socket
    /// stays bound to its address, so the route is checked by a local
    /// probe, which sends nothing.
    fn usable(own: SocketAddrV6, config: &Bip6ForeignDeviceConfig) -> bool {
        let own_address = config.bbmd_ip == *own.ip() && config.bbmd_port == own.port();
        if config.check().is_err() || own_address {
            return false;
        }
        let bbmd = SocketAddrV6::new(config.bbmd_ip, config.bbmd_port, 0, 0);
        matches!(super::socket::route_source(bbmd), Ok(source) if source == *own.ip())
    }
}

impl ForeignRegistration for ForeignTarget {
    /// The host name and port written, while the BBMD is known by name;
    /// the address otherwise.
    fn bbmd(&self) -> BACnetHostNPort {
        let state = self.lock();
        match state.lookups.name() {
            Some(name) => name.value(),
            None => {
                let (ip, port) = (state.config.bbmd_ip, state.config.bbmd_port);
                BACnetHostNPort::from_socket_addr(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)))
            }
        }
    }

    fn subscription_lifetime(&self) -> u16 {
        self.lock().config.ttl
    }

    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        Self::changed_config(&self.lock(), changes).map(drop)
    }

    /// A host name is looked up on the side (#2308), and the device moves
    /// once it resolves; a name alone, with the time-to-live as it was,
    /// sends nothing to the BBMD in use before then.
    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let mut state = self.lock();
        let (config, name) = Self::changed_config(&state, changes)?;
        // Every activation registers again at once, unless it only names
        // the BBMD by a host name, which registers once looked up.
        let reregister = !matches!(name, Some(Some(_))) || config != state.config;
        state.take(config);
        let lookup = name.and_then(|name| state.lookups.renamed(name, &self.name_target()));
        drop(state);
        if reregister {
            // One permit is stored if the task isn't waiting yet, so the
            // change is never missed.
            self.changed.notify_one();
        }
        if let Some(lookup) = lookup {
            lookup.start();
        }
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}
