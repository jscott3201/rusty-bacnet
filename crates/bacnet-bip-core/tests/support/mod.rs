//! Helpers the integration tests share.

#![allow(dead_code)]

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry};
use bacnet_bip_core::bbmd::{Context, RegistrationLimits, Reservation, Tables, Target, Via};
use bacnet_bip_core::LocalSubnet;
use bacnet_bip_core::{Duration, Instant};

/// The default B/IP port.
pub const PORT: u16 = 0xBAC0;
/// A subnet mask of /24.
pub const MASK_24: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);
/// The two-hop mask.
pub const MASK_32: Ipv4Addr = Ipv4Addr::BROADCAST;

/// `a.b.c.d:port`.
pub const fn addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// A row for `address` with `mask`.
pub const fn row(address: SocketAddrV4, mask: Ipv4Addr) -> BdtEntry {
    BdtEntry::new(address, mask)
}

/// The BBMD under test: 192.168.1.1, at the default port.
pub const OWN: SocketAddrV4 = addr(192, 168, 1, 1, PORT);
/// Its subnet's broadcast address.
pub const LOCAL_BROADCAST: SocketAddrV4 = addr(192, 168, 1, 255, PORT);
/// Its subnet, its mask unknown.
pub const SUBNET: LocalSubnet = LocalSubnet::new(LOCAL_BROADCAST);

/// Small tables, as firmware would size them.
pub type Small = Tables<8, 8>;

/// Storage for one decision over [`Small`].
pub fn storage() -> [Target; Small::MAX_TARGETS] {
    [Target::UNUSED; Small::MAX_TARGETS]
}

/// Tables for [`OWN`] with `bdt` set.
pub fn tables(bdt: &[BdtEntry]) -> Small {
    let mut tables = Small::new(OWN);
    tables.bdt.set(BdtEntries::from_entries(bdt)).unwrap();
    tables
}

/// Limits that admit anything with a nonzero time-to-live.
pub const OPEN: RegistrationLimits = limits(1, u16::MAX, usize::MAX, 0);

/// Limits of a time-to-live from `min_ttl` to `max_ttl` seconds,
/// `max_entries_per_source` registrations per IPv4 address and
/// `reserved_capacity` slots reserved.
pub const fn limits(
    min_ttl: u16,
    max_ttl: u16,
    max_entries_per_source: usize,
    reserved_capacity: usize,
) -> RegistrationLimits {
    let mut limits = RegistrationLimits::DEFAULT;
    limits.min_ttl = min_ttl;
    limits.max_ttl = max_ttl;
    limits.max_entries_per_source = max_entries_per_source;
    limits.reserved_capacity = reserved_capacity;
    limits
}

/// Register each of `devices` for `ttl` seconds at `now`, which must succeed.
pub fn register_all(tables: &mut Small, devices: &[SocketAddrV4], ttl: u16, now: Instant) {
    for device in devices {
        let outcome = tables
            .fdt
            .register(*device, ttl, now, &OPEN, Reservation::Ordinary, || true);
        assert!(outcome.result.is_ok(), "{device}: {outcome:?}");
    }
}

/// A decision context at `now` with the default fan-out budget.
pub fn context(now: Instant) -> Context {
    Context::new(now, 32)
}

/// `secs` seconds.
pub const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

/// The destinations of `targets`.
pub fn destinations(targets: &[Target]) -> Vec<SocketAddrV4> {
    targets.iter().map(|target| target.destination).collect()
}

/// A destination a test expects. [`Target`] is `#[non_exhaustive]`, so a
/// test outside the crate can't build one to compare with: a `Target`, or
/// a slice of them, compares with this instead.
#[derive(Debug, Clone, Copy)]
pub struct ExpectedTarget {
    pub destination: SocketAddrV4,
    pub via: Via,
}

impl PartialEq<ExpectedTarget> for Target {
    fn eq(&self, expected: &ExpectedTarget) -> bool {
        self.destination == expected.destination && self.via == expected.via
    }
}

/// A [`Target`] at `destination` of kind `via`, as a test expects it.
pub const fn target(destination: SocketAddrV4, via: Via) -> ExpectedTarget {
    ExpectedTarget { destination, via }
}
