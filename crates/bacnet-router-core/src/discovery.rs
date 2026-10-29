//! Looking for routes to unknown networks, in bounded storage.
//!
//! A router asked to reach a network it has no route to sends a
//! Who-Is-Router for it before refusing the message (Clause 6.6.3.1).
//! [`Discovery`] decides when to send one: at most once per network per
//! cooldown, for up to `D` networks at a time, each forgotten past its age
//! at the next sweep; the router's
//! [`Options`](crate::router::Options) set both. Messages that wait for an
//! answer are refused at once, not buffered, and the hosted router refuses
//! them with reject reason 1; once the answer has taught the table a route,
//! the next message takes it. Network 0 and 0xFFFF are never looked for,
//! and a cancelled `Discovery` looks for nothing more.
//!
//! The cooldown and the age come with each call, from the router's options,
//! so [`Discovery::new`] is all zero bits.

use core::fmt;

use crate::{Duration, Instant, swap_apart};

/// When a network was last looked for.
#[derive(Debug, Clone, Copy)]
struct Entry {
    network: u16,
    solicited: Instant,
}

/// Unused storage: all zero bits.
const NO_ENTRY: Entry = Entry {
    network: 0,
    solicited: Instant::ZERO,
};

/// The networks a router is looking for: up to `D`, sorted by number.
///
/// [`new`](Self::new) is a `const fn` whose result is all zero bits, so it
/// can live in a zero-initialized static (`.bss`).
#[derive(Clone)]
pub struct Discovery<const D: usize> {
    entries: [Entry; D],
    len: usize,
    cancelled: bool,
}

/// The searches remembered, not the unused storage.
impl<const D: usize> fmt::Debug for Discovery<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Discovery")
            .field("entries", &self.stored())
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

impl<const D: usize> Default for Discovery<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const D: usize> Discovery<D> {
    /// Nothing looked for yet: all zero bits.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [NO_ENTRY; D],
            len: 0,
            cancelled: false,
        }
    }

    /// Whether to send a Who-Is-Router for `network` now; when it says so,
    /// the search is recorded at `now`.
    ///
    /// No: when cancelled, for network 0 or 0xFFFF, within `cooldown` of the
    /// last search for it, or when `D` other networks are being looked for
    /// already.
    pub fn should_solicit(&mut self, network: u16, now: Instant, cooldown: Duration) -> bool {
        if self.cancelled || network == 0 || network == 0xFFFF {
            return false;
        }
        let entry = Entry {
            network,
            solicited: now,
        };
        match self.find(network) {
            Ok(index) => {
                let last = self.entries[index].solicited;
                if now.saturating_duration_since(last) < cooldown {
                    return false;
                }
                self.entries[index] = entry;
            }
            Err(_) if self.len == D => return false,
            Err(index) => {
                // Into the free slot at the end, then back to its place one
                // swap at a time, with no memmove (`swap_apart`).
                self.entries[self.len] = entry;
                for at in (index..self.len).rev() {
                    swap_apart(&mut self.entries, at, at + 1);
                }
                self.len += 1;
            }
        }
        true
    }

    /// Forget every search more than `max_age` old at `now`, calling
    /// `expired` with each network in ascending order, and return how many.
    ///
    /// Every call comes before anything is forgotten, so an `expired` that
    /// panics leaves the searches as they were.
    pub fn expire(
        &mut self,
        now: Instant,
        max_age: Duration,
        mut expired: impl FnMut(u16),
    ) -> usize {
        let old = |entry: &Entry| now.saturating_duration_since(entry.solicited) > max_age;
        for entry in self.stored().iter().filter(|entry| old(entry)) {
            expired(entry.network);
        }
        // Each kept search is swapped forward, never copied over another,
        // with no memmove (`swap_apart`).
        let mut kept = 0;
        for index in 0..self.len {
            if !old(&self.entries[index]) {
                swap_apart(&mut self.entries, kept, index);
                kept += 1;
            }
        }
        let removed = self.len - kept;
        self.entries[kept..self.len].fill(NO_ENTRY);
        self.len = kept;
        removed
    }

    /// Forget every search and look again: what [`new`](Self::new) gives.
    pub(crate) fn clear(&mut self) {
        self.entries[..self.len].fill(NO_ENTRY);
        self.len = 0;
        self.cancelled = false;
    }

    /// Forget every search and look for nothing more: the router stopped.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.entries[..self.len].fill(NO_ENTRY);
        self.len = 0;
    }

    /// Whether [`cancel`](Self::cancel) was called.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// When `network` was last looked for, while that is remembered.
    #[must_use]
    pub fn last_solicited(&self, network: u16) -> Option<Instant> {
        let index = self.find(network).ok()?;
        Some(self.entries[index].solicited)
    }

    /// How many networks are being looked for.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The searches remembered, sorted by network.
    fn stored(&self) -> &[Entry] {
        &self.entries[..self.len]
    }

    fn find(&self, network: u16) -> Result<usize, usize> {
        self.stored()
            .binary_search_by_key(&network, |entry| entry.network)
    }
}
