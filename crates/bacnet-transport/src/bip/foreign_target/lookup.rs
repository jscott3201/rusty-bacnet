//! FD_BBMD_Address as a host name (Clause 12.56.37, #1852).
//!
//! Annex J.5.2 leaves it to the device how it learns its BBMD's IP address,
//! and names a name server as one way. The node engine in bip-core works on
//! addresses only, so a written host name is looked up here, on the hosted
//! side, and the node moved to what it stands for: once when the row is
//! activated, and again before each automatic registration attempt (the
//! first after a start, each renewal and each retry). Until a lookup
//! succeeds, and whenever one fails, the device keeps registering with the
//! address it has, and the failure is logged and counted in the
//! foreign-registration snapshot.
//!
//! One lookup runs at a time, bounded by [`LOOKUP_TIMEOUT`]. A name that
//! stands for several addresses keeps the device where it is while the
//! address in use is still one of them, so round-robin answers don't move it
//! at every renewal; otherwise it moves to the first one it can register
//! with, exactly as an activated address would move it.
//!
//! Lock order, as in the parent: the target's lock, then the node's. The
//! resolver runs on a task of its own, with no lock held.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use tracing::{debug, info, warn};

use super::super::foreign_device::ForeignDeviceConfig;
use super::super::node::NodeHandle;
use super::{ForeignTarget, TargetState};

/// What a [`BbmdResolver`] returns: in time, the IPv4 addresses a host name
/// stands for, in the order to prefer them, or why it couldn't tell.
pub type BbmdLookup = Pin<Box<dyn Future<Output = io::Result<Vec<Ipv4Addr>>> + Send>>;

/// Looks up the host name a B/IP foreign device's BBMD is known by, when
/// FD_BBMD_Address names one (#1852).
///
/// The transport calls it once when the name is activated and again before
/// each registration attempt, one lookup at a time, and gives up on a lookup
/// after 10 seconds. Without one set
/// ([`BipTransport::set_bbmd_resolver`](super::super::BipTransport::set_bbmd_resolver)),
/// the system's resolver does the work on Tokio's blocking pool, never on a
/// runtime worker thread. A resolver of your own lets a host use another
/// name service, or answer from a table in tests; its future runs on a task
/// of its own, so it may take its time, but `resolve` itself should return
/// at once.
pub trait BbmdResolver: Send + Sync {
    /// The IPv4 addresses `host` stands for, the preferred first.
    fn resolve(&self, host: &str) -> BbmdLookup;
}

/// The system's resolver (`getaddrinfo` and its kin), which
/// [`tokio::net::lookup_host`] runs on Tokio's blocking pool.
pub(in crate::bip) struct SystemResolver;

impl BbmdResolver for SystemResolver {
    fn resolve(&self, host: &str) -> BbmdLookup {
        let host = host.to_owned();
        Box::pin(async move {
            // The port plays no part in the answer.
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addresses
                .filter_map(|address| match address {
                    SocketAddr::V4(v4) => Some(*v4.ip()),
                    SocketAddr::V6(_) => None,
                })
                .collect())
        })
    }
}

/// Why a lookup of the BBMD's host name failed (#1852). The device kept
/// registering with the address it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BbmdLookupFailure {
    /// The resolver failed with an error of this kind.
    Error(io::ErrorKind),
    /// The resolver hadn't answered after 10 seconds.
    TimedOut,
    /// The name stands for no IPv4 address.
    NoIpv4Address,
    /// Every IPv4 address the name stands for is one this device can't
    /// register with, as a start would refuse it: its own address, the
    /// unspecified address, or a broadcast or multicast address. This is
    /// the first of them.
    Unusable(Ipv4Addr),
}

/// How long a lookup may take before it counts as failed
/// ([`BbmdLookupFailure::TimedOut`]): long enough for a system resolver
/// to try a second name server.
pub(super) const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// FD_BBMD_Address as a host name and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BbmdName {
    host: String,
    port: u16,
}

impl BbmdName {
    /// The name `host` at `port`, when a lookup can take the name and the
    /// port isn't zero.
    pub(super) fn new(host: &str, port: u16) -> Option<Self> {
        (port != 0 && is_host_name(host)).then(|| Self {
            host: host.to_owned(),
            port,
        })
    }

    /// The value FD_BBMD_Address reads.
    pub(super) fn value(&self) -> BACnetHostNPort {
        BACnetHostNPort {
            host: BACnetHostAddress::Name(self.host.clone()),
            port: self.port,
        }
    }
}

/// Whether `host` is a host name in the RFC 1123 form: dot-separated labels
/// of 1 to 63 letters, digits and hyphens, none starting or ending with a
/// hyphen, at most 253 octets in all, a trailing dot aside.
fn is_host_name(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|octet| octet.is_ascii_alphanumeric() || octet == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// The target's lookup bookkeeping, under its lock.
pub(super) struct Lookups {
    resolver: Arc<dyn BbmdResolver>,
    /// Counts activations of the BBMD row: a lookup started for an earlier
    /// one is dropped when it ends.
    generation: u64,
    /// The generation of the lookup running, if any.
    in_flight: Option<u64>,
    /// The next registration attempt goes to an address just looked up, so
    /// it starts no lookup of its own.
    skip_next: bool,
    /// The newest lookup failed: a run of failures warns once.
    failing: bool,
}

impl Lookups {
    pub(super) fn new(resolver: Arc<dyn BbmdResolver>) -> Self {
        Self {
            resolver,
            generation: 0,
            in_flight: None,
            skip_next: false,
            failing: false,
        }
    }

    pub(super) fn set_resolver(&mut self, resolver: Arc<dyn BbmdResolver>) {
        self.resolver = resolver;
    }

    /// A node starts: an attempt skipped for an earlier node doesn't carry
    /// over.
    pub(super) fn attached(&mut self) {
        self.skip_next = false;
    }
}

/// A lookup to start once the target's lock is released.
pub(super) struct Lookup {
    target: Weak<ForeignTarget>,
    resolver: Arc<dyn BbmdResolver>,
    host: String,
    generation: u64,
}

/// A failed lookup, with the resolver's error for the log.
struct Failed {
    failure: BbmdLookupFailure,
    error: Option<io::Error>,
}

impl From<BbmdLookupFailure> for Failed {
    fn from(failure: BbmdLookupFailure) -> Self {
        Self {
            failure,
            error: None,
        }
    }
}

impl Lookup {
    /// Run it on a task of its own. With no runtime to run it on, it fails
    /// at once.
    pub(super) fn start(self) {
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(self.run());
            }
            Err(error) => {
                let failed = Failed {
                    failure: BbmdLookupFailure::Error(io::ErrorKind::Other),
                    error: Some(io::Error::other(error)),
                };
                self.end(Err(failed));
            }
        }
    }

    async fn run(self) {
        let lookup = self.resolver.resolve(&self.host);
        let answer = match tokio::time::timeout(LOOKUP_TIMEOUT, lookup).await {
            Ok(Ok(addresses)) => Ok(addresses),
            Ok(Err(error)) => Err(Failed {
                failure: BbmdLookupFailure::Error(error.kind()),
                error: Some(error),
            }),
            Err(_) => Err(BbmdLookupFailure::TimedOut.into()),
        };
        self.end(answer);
    }

    fn end(self, answer: Result<Vec<Ipv4Addr>, Failed>) {
        if let Some(target) = self.target.upgrade() {
            target.looked_up(self.generation, answer);
        }
    }
}

impl ForeignTarget {
    /// The BBMD row was activated as `name`, or as an address (`None`):
    /// a lookup started for an earlier value no longer counts, and a name
    /// is looked up now. `attempt_follows` says whether the activation
    /// starts a registration attempt, which this lookup then covers.
    pub(super) fn renamed(
        &self,
        state: &mut TargetState,
        name: Option<BbmdName>,
        attempt_follows: bool,
    ) -> Option<Lookup> {
        let lookups = &mut state.lookups;
        lookups.generation = lookups.generation.wrapping_add(1);
        lookups.skip_next = attempt_follows && name.is_some();
        lookups.failing = false;
        state.name = name;
        self.lookup(state)
    }

    /// The node is about to send an automatic registration attempt: look
    /// the BBMD's name up again, if it has one (#1852).
    pub(super) fn attempting(&self) {
        let lookup = {
            let mut state = self.lock();
            if std::mem::take(&mut state.lookups.skip_next) {
                None
            } else {
                self.lookup(&mut state)
            }
        };
        if let Some(lookup) = lookup {
            lookup.start();
        }
    }

    /// A lookup of the name in effect, unless there is none, or one is
    /// already running for it.
    fn lookup(&self, state: &mut TargetState) -> Option<Lookup> {
        let host = state.name.as_ref()?.host.clone();
        let lookups = &mut state.lookups;
        if lookups.in_flight == Some(lookups.generation) {
            return None;
        }
        lookups.in_flight = Some(lookups.generation);
        Some(Lookup {
            target: self.me.clone(),
            resolver: Arc::clone(&lookups.resolver),
            host,
            generation: lookups.generation,
        })
    }

    /// A lookup started for `generation` ended. One for an earlier
    /// activation, or one that ends with the transport stopped, changes and
    /// counts nothing: a start looks the name up again.
    fn looked_up(&self, generation: u64, answer: Result<Vec<Ipv4Addr>, Failed>) {
        let mut state = self.lock();
        if state.lookups.in_flight == Some(generation) {
            state.lookups.in_flight = None;
        }
        if state.lookups.generation != generation {
            debug!("A BBMD host-name lookup ended after the row changed; dropped");
            return;
        }
        let (Some(name), Some(node)) = (state.name.clone(), Self::running(&state)) else {
            return;
        };
        let verdict = answer.and_then(|addresses| self.choose(&state, &node, &name, &addresses));
        node.record()
            .bbmd_lookup_ended(verdict.as_ref().err().map(|failed| failed.failure));
        let moved_to = match verdict {
            Ok(moved_to) => moved_to,
            Err(failed) => {
                self.failed(&mut state, &name, &failed);
                return;
            }
        };
        if std::mem::take(&mut state.lookups.failing) {
            info!(host = %name.host, "The BBMD's host name resolves again");
        }
        let Some(config) = moved_to else {
            return;
        };
        let old = state.config.bbmd();
        // `choose` checked the move as a retarget would, under this lock.
        let moved = config
            .interval()
            .and_then(|interval| node.retarget(config.engine(interval)));
        if let Err(error) = moved {
            debug!(%error, "The node refused the address the BBMD's host name resolved to");
            return;
        }
        info!(host = %name.host, bbmd = %config.bbmd(), "The BBMD's host name now resolves elsewhere; registering there");
        state.config = config;
        state.lookups.skip_next = true;
        drop(state);
        if self.release_previous.load(Ordering::Relaxed) {
            node.release_previous_bbmd(old);
        }
    }

    /// Where `addresses` put the BBMD: `None` to stay with the address in
    /// use, which is still one of them, or else the registration at the
    /// first one this device can register with.
    fn choose(
        &self,
        state: &TargetState,
        node: &NodeHandle,
        name: &BbmdName,
        addresses: &[Ipv4Addr],
    ) -> Result<Option<ForeignDeviceConfig>, Failed> {
        let at = |ip: Ipv4Addr| SocketAddrV4::new(ip, name.port);
        let current = state.config.bbmd();
        if addresses.iter().any(|ip| at(*ip) == current) {
            return Ok(None);
        }
        let candidate = |ip: Ipv4Addr| {
            let mut config = state.config.clone();
            config.bbmd_ip = ip;
            config.bbmd_port = name.port;
            config
        };
        let usable = |config: &ForeignDeviceConfig| {
            config.interval().is_ok_and(|interval| {
                config.check_bbmd(self.broadcast, self.interface).is_ok()
                    && node.check_retarget(config.engine(interval)).is_ok()
            })
        };
        if let Some(config) = addresses
            .iter()
            .map(|ip| candidate(*ip))
            .find(|config| usable(config))
        {
            return Ok(Some(config));
        }
        Err(match addresses.first() {
            Some(ip) => BbmdLookupFailure::Unusable(*ip),
            None => BbmdLookupFailure::NoIpv4Address,
        }
        .into())
    }

    /// Log a failed lookup: the first of a run at warn, the rest at debug.
    fn failed(&self, state: &mut TargetState, name: &BbmdName, failed: &Failed) {
        let bbmd = state.config.bbmd();
        let failure = failed.failure;
        let error = failed.error.as_ref().map(ToString::to_string);
        if std::mem::replace(&mut state.lookups.failing, true) {
            debug!(host = %name.host, ?failure, ?error, %bbmd, "The BBMD's host name still doesn't resolve to a usable address");
        } else {
            warn!(host = %name.host, ?failure, ?error, %bbmd, "The BBMD's host name doesn't resolve to a usable address; registering with the last address it had");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BbmdName, is_host_name};

    #[test]
    fn host_names_take_the_rfc_1123_form() {
        let long_label = "a".repeat(63);
        let longest = [long_label.as_str(); 4].join(".")[..253].to_owned();
        for good in [
            "bbmd",
            "bbmd.example",
            "BBMD-1.example.",
            "10.example",
            "192.0.2.1",
            long_label.as_str(),
            longest.as_str(),
        ] {
            assert!(is_host_name(good), "{good}");
        }
        let too_long = format!("{longest}a");
        let label_too_long = "a".repeat(64);
        for bad in [
            "",
            ".",
            "-bbmd",
            "bbmd-",
            "bb..md",
            "bbmd..",
            "bb md",
            "bb_md",
            "bbmd:47808",
            "::1",
            "bbmd.exämple",
            label_too_long.as_str(),
            too_long.as_str(),
        ] {
            assert!(!is_host_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_name_needs_a_port() {
        assert!(BbmdName::new("bbmd.example", 0).is_none());
        assert!(BbmdName::new("bbmd.example", 47808).is_some());
    }
}
