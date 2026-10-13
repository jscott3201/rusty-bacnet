//! Helpers the table suites share.
//!
//! The suites are ported from `bacnet-network`'s `router_table_tests.rs`,
//! `router_table_reject_tests.rs`, `router_table_corroboration_tests.rs`,
//! the table cases of `router/rb06_convergence_tests.rs` and
//! `router/busy_scope_tests.rs`, and the wire Port ID case of
//! `router/init_routing_table_tests.rs`. Where those read the clock, these
//! pass times on one timeline from [`BASE`]; where they read the private
//! maps, these use the table's accessors.

#![allow(dead_code)]

use bacnet_router_core::table::{ClaimCounters, Convergence, Table, Timing};
use bacnet_router_core::{Duration, Instant, Mac, PortId};

/// The hosted router's sizes: 256 routes, 1,024 hold-down records.
pub type Hosted = Table<256, 1024>;

/// The hosted router's timers, which the suites pass to each call.
pub const TIMING: Timing = Timing::DEFAULT;

/// An instant well after the epoch, so tests can step back from it.
pub const BASE: Instant = Instant::from_secs(1_000);

/// The default hold-down.
pub const HOLD_DOWN: Duration = Duration::from_secs(30);

/// The default corroboration window.
pub const CORROBORATION_WINDOW: Duration = Duration::from_secs(60);

/// `n` seconds.
pub const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// One nanosecond.
pub const NANO: Duration = Duration::from_nanos(1);

/// The table's claim counts: zero but for what `set` changes.
/// `ClaimCounters` is `#[non_exhaustive]`, so a test can't spell it out.
pub fn claim_counts(set: impl FnOnce(&mut ClaimCounters)) -> ClaimCounters {
    let mut counts = ClaimCounters::default();
    set(&mut counts);
    counts
}

/// A MAC of these octets.
pub fn mac(octets: &[u8]) -> Mac {
    Mac::from_slice(octets).unwrap()
}

/// Port `n`.
pub const fn port(n: u8) -> PortId {
    PortId(n)
}

/// Network 3000 learned through port 0 from MAC `[1]` at [`BASE`].
pub fn learned_table() -> Hosted {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table
}

/// An empty table under hardened convergence.
pub fn hardened() -> Hosted {
    let mut table = Hosted::new();
    table.set_convergence(Convergence::Hardened);
    table
}

/// The same, under hardened convergence.
pub fn learned_hardened() -> Hosted {
    let mut table = hardened();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table
}

/// Every network [`Table::purge_stale`] removes, in its order.
pub fn purge(table: &mut Hosted, now: Instant, max_age: Duration) -> Vec<u16> {
    let mut purged = Vec::new();
    let count = table.purge_stale(now, max_age, |network| purged.push(network));
    assert_eq!(count, purged.len());
    purged
}

/// Every pending claim [`Table::expire_pending`] drops, in its order.
pub fn expire_pending(table: &mut Hosted, now: Instant) -> Vec<u16> {
    let mut expired = Vec::new();
    let count = table.expire_pending(now, &TIMING, |network| expired.push(network));
    assert_eq!(count, expired.len());
    expired
}
