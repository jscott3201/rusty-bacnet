//! Duplicate global-broadcast suppression (#1698), off by default.
//!
//! In a topology with a loop, routers pass one global broadcast around it
//! until its hop count runs out, each copy fanned out again. The standard
//! defines no suppression, so a router that wants it keeps its own record:
//! here, the last `B` global broadcasts seen, each by its origin (SNET and
//! SADR, or the arrival network and link source when it has none, the
//! address a relay would add) and a hash of its APDU, for
//! [`Duplicates::ttl`]. A copy that matches one within that time is
//! dropped, neither relayed nor delivered, and counted in
//! [`Counters::duplicate_drops`](super::Counters::duplicate_drops).
//!
//! The record is a ring of `B` entries in the [`Storage`](super::Storage),
//! all zero bits when empty; a full ring gives up its oldest entry, so `B`
//! must cover the global broadcasts the router sees within one time to
//! live, or a looped copy can outlast its record. The hashes are 32-bit
//! FNV-1a: multiplications only, no division. Two different broadcasts
//! from one origin whose hashes collide within the time to live are taken
//! as one, and the second is wrongly dropped; a collision never lets a
//! looped copy through, since a real repeat always hashes the same. A
//! legitimate sender that repeats the same broadcast within the time to
//! live, a client retrying a Who-Is say, loses the repeat too, which is
//! why it is off unless [`Options::duplicates`](super::Options::duplicates)
//! turns it on.

use crate::{Duration, Instant};

/// How long a global broadcast is remembered for duplicate suppression.
///
/// It is `#[non_exhaustive]`: start from [`Duplicates::DEFAULT`] and set
/// the fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Duplicates {
    /// How long after its first copy a repeat is dropped.
    pub ttl: Duration,
}

impl Duplicates {
    /// One second: a looped copy comes back within milliseconds over IP
    /// and within a few hundred over MS/TP, while a client's own retries
    /// are usually a second or more apart.
    pub const DEFAULT: Self = Self {
        ttl: Duration::from_secs(1),
    };
}

impl Default for Duplicates {
    /// [`Duplicates::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One broadcast seen. Network 0 marks an empty slot: no SNET or port
/// network is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    at: Instant,
    payload: u32,
    source: u32,
    network: u16,
}

const EMPTY: Seen = Seen {
    at: Instant::ZERO,
    payload: 0,
    source: 0,
    network: 0,
};

/// The last `B` global broadcasts seen.
#[derive(Debug, Clone)]
pub(super) struct Recent<const B: usize> {
    seen: [Seen; B],
    /// The slot the next new one takes.
    next: usize,
}

/// 32-bit FNV-1a over `octets`.
fn fnv1a(octets: &[u8]) -> u32 {
    octets.iter().fold(0x811c_9dc5, |hash, &octet| {
        (hash ^ u32::from(octet)).wrapping_mul(0x0100_0193)
    })
}

impl<const B: usize> Recent<B> {
    pub(super) const NEW: Self = Self {
        seen: [EMPTY; B],
        next: 0,
    };

    /// Whether a broadcast from `source` on `network` with `payload` was
    /// seen within `ttl` before `now`; if not, it is recorded. A repeat
    /// keeps the first copy's time, so a sender repeating itself every
    /// second isn't suppressed for good.
    pub(super) fn repeat(
        &mut self,
        network: u16,
        source: &[u8],
        payload: &[u8],
        now: Instant,
        ttl: Duration,
    ) -> bool {
        let entry = Seen {
            at: now,
            payload: fnv1a(payload),
            source: fnv1a(source),
            network,
        };
        let fresh = |seen: &Seen| now < seen.at + ttl;
        let matches = |seen: &&Seen| {
            seen.network == network && seen.source == entry.source && seen.payload == entry.payload
        };
        if self.seen.iter().filter(matches).any(fresh) {
            return true;
        }
        if let Some(slot) = self.seen.get_mut(self.next) {
            *slot = entry;
            self.next += 1;
            if self.next == B {
                self.next = 0;
            }
        }
        false
    }
}
