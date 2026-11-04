//! A foreign device's BBMD named by host name (#1852, #2308), for B/IP and
//! B/IPv6 alike.
//!
//! FD_BBMD_Address is a BACnetHostNPort (Clause 12.56.37), so a Network
//! Port write may name the BBMD by host name rather than by address. Annex
//! J.5.2 leaves it to the device how it learns the BBMD's IP address, and
//! names a name server as one way; Annex U.4.5 registers a B/IPv6 foreign
//! device the same way. The transports' node engines work on addresses
//! only, so the name is looked up on the hosted side, by a
//! [`BbmdResolver`], and the device moved to what it stands for.
//!
//! Both transports share one policy:
//!
//! - the name is looked up whenever the row is activated, and again
//!   alongside the automatic registration attempts (the first after a
//!   start, renewals and retries), at most once per 30 seconds outside an
//!   activation. Such an attempt still goes to the address in use; a
//!   lookup that finds the BBMD elsewhere moves the device, which registers
//!   there at once;
//! - one resolver call runs at a time, whichever activation it serves, and
//!   each is given up after 10 seconds. A name activated while one runs is
//!   looked up when it ends, and the stale answer is dropped. A lookup that
//!   ends without an answer, its resolver panicking or its task dropped,
//!   frees the slot all the same;
//! - while the address in use is still among the answers the device stays,
//!   so round-robin answers don't move it; otherwise it moves to the first
//!   answer of its transport's family (IPv4 for B/IP, IPv6 for B/IPv6) it
//!   can register with, as an activated address would move it;
//! - until a lookup succeeds, and whenever one fails, the device keeps
//!   registering with the address it has, and the failure is logged (at
//!   warn once per run of failures) and counted. A B/IP device whose
//!   configured BBMD is a name not resolved yet has no address: it waits,
//!   registering nowhere, and tries the name again 30 seconds after each
//!   failure (#2308).
//!
//! With the floor between attempt-driven lookups, a peer that can write the
//! row and reinitialize the device can't stack resolver threads, and a short
//! time-to-live can't drive a query per renewal.

mod lookups;
#[cfg(test)]
pub(crate) mod table;

use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::time::Duration;

use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};

pub(crate) use lookups::{Ended, Failed, Lookup, Lookups, NameTarget, choose};

/// What a [`BbmdResolver`] returns: in time, the addresses a host name
/// stands for, in the order to prefer them, or why it couldn't tell. Each
/// transport takes the answers of its own family.
pub type BbmdLookup = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

/// Looks up the host name a foreign device's BBMD is known by, when
/// FD_BBMD_Address names one (#1852, #2308).
///
/// The transport calls it whenever the name is activated, and again
/// alongside the automatic registration attempts, at most once every 30
/// seconds; one call runs at a time, and the transport gives up on a call
/// after 10 seconds. Without one set
/// ([`BipTransport::set_bbmd_resolver`](crate::bip::BipTransport::set_bbmd_resolver),
/// and on B/IPv6 `Bip6Transport::set_bbmd_resolver`), the system's
/// resolver does the work on Tokio's blocking pool, never on a runtime
/// worker thread. A resolver of your own lets a host use another name
/// service, or answer from a table in tests; its future runs on a task of
/// its own, so it may take its time, but `resolve` itself should return at
/// once. It may answer with addresses of both families: B/IP takes the IPv4
/// ones, B/IPv6 the IPv6 ones.
pub trait BbmdResolver: Send + Sync {
    /// The addresses `host` stands for, the preferred first.
    fn resolve(&self, host: &str) -> BbmdLookup;
}

/// The system's resolver (`getaddrinfo` and its kin), which
/// [`tokio::net::lookup_host`] runs on Tokio's blocking pool.
pub(crate) struct SystemResolver;

impl BbmdResolver for SystemResolver {
    fn resolve(&self, host: &str) -> BbmdLookup {
        let host = host.to_owned();
        Box::pin(async move {
            // The port plays no part in the answer.
            let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addresses.map(|address| address.ip()).collect())
        })
    }
}

/// Why a lookup of the BBMD's host name failed (#1852). The device kept
/// registering with the address it had, or, with none yet, kept waiting
/// for one (#2308).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BbmdLookupFailure {
    /// The resolver failed with an error of this kind.
    Error(io::ErrorKind),
    /// The resolver hadn't answered after 10 seconds.
    TimedOut,
    /// The name stands for no address of the transport's family: no IPv4
    /// address for B/IP, no IPv6 address for B/IPv6.
    NoAddress,
    /// Every address of the transport's family the name stands for is one
    /// this device can't register with, as a start or a Network Port write
    /// would refuse it: on B/IP its own address, the unspecified address,
    /// or a broadcast or multicast address; on B/IPv6 also a link-local or
    /// IPv4-mapped address, or one the bound socket can't reach. This is
    /// the first of them.
    Unusable(IpAddr),
    /// The lookup ended without an answer: the resolver panicked, or its
    /// task was dropped, as when the runtime shuts down.
    Abandoned,
}

/// What came of the lookups of a foreign device's BBMD host name (#2308):
/// B/IPv6's report (`Bip6Transport::bbmd_lookup_counters`).
/// B/IP reports the same three values in its
/// [`ForeignRegistrationSnapshot`](crate::bip::ForeignRegistrationSnapshot).
/// The totals survive a restart; the newest failure is cleared at each
/// start and stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BbmdLookupCounters {
    /// Lookups that ended with the transport running: one at each
    /// activation of a name, one alongside a registration attempt at most
    /// every 30 seconds, and, while the device waits for a configured name,
    /// one 30 seconds after each failure (#2365). A B/IPv6 start on `::`
    /// counts the lookup it makes before binding, whether or not the start
    /// succeeds.
    pub lookups: u64,
    /// Of those, the ones that failed. The device kept registering with
    /// the address it had, or, with none yet, kept waiting for the name.
    pub failures: u64,
    /// Why the newest lookup failed; `None` once one succeeds.
    pub last_failure: Option<BbmdLookupFailure>,
}

impl BbmdLookupCounters {
    /// Count a lookup that ended: failed with `failure`, or succeeded.
    #[cfg(feature = "ipv6")]
    pub(crate) fn ended(&mut self, failure: Option<BbmdLookupFailure>) {
        self.lookups = self.lookups.saturating_add(1);
        if failure.is_some() {
            self.failures = self.failures.saturating_add(1);
        }
        self.last_failure = failure;
    }
}

/// How long a lookup may take before it counts as failed
/// ([`BbmdLookupFailure::TimedOut`]): long enough for a system resolver
/// to try a second name server.
pub(crate) const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The least time between the starts of two lookups the registration
/// attempts make; an attempt inside it keeps the last answer. An activation
/// always looks the name up.
pub(crate) const ATTEMPT_LOOKUP_INTERVAL: Duration = Duration::from_secs(30);

/// FD_BBMD_Address as a host name and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BbmdName {
    host: String,
    port: u16,
}

impl BbmdName {
    /// The name `host` at `port`, when a lookup can take the name and the
    /// port isn't zero.
    pub(crate) fn new(host: &str, port: u16) -> Option<Self> {
        (port != 0 && is_host_name(host)).then(|| Self {
            host: host.to_owned(),
            port,
        })
    }

    /// The host name.
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    /// The UDP port.
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// The value FD_BBMD_Address reads.
    pub(crate) fn value(&self) -> BACnetHostNPort {
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

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv6Addr};

    use super::{BbmdName, BbmdResolver, SystemResolver, is_host_name};

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

    /// The system resolver's wrapper keeps IPv6 answers, which B/IPv6
    /// takes, rather than filtering them out. `::1` is a literal, which
    /// Tokio parses without asking the host's name service, so this covers
    /// the wrapper only; the transports' `localhost` tests go through the
    /// name service (`a_host_name_resolves_through_the_system_resolver`).
    #[tokio::test]
    async fn the_system_resolver_wrapper_keeps_ipv6_answers() {
        let answers = SystemResolver.resolve("::1").await.unwrap();
        assert_eq!(answers, [IpAddr::V6(Ipv6Addr::LOCALHOST)]);
    }
}
