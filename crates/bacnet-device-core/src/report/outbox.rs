//! The outbox: notifications decided but not yet handed out, as service
//! parameters in one buffer allocated when the [`Reporter`](super::Reporter)
//! is built, bounded by entries and by octets.
//!
//! Entries leave in the order they came. A subscription has at most one
//! unconfirmed notification queued: a newer one replaces it in its place,
//! so a subscription that changes faster than the link sends holds one
//! entry, with its latest values, and keeps its turn rather than losing it
//! to each change (a confirmed subscription has one report at a time
//! anyway). An entry whose report a table change fenced, or whose
//! subscription left, is marked dead and skipped. When the free space at
//! the buffer's end is too short, the live parameters move to its start, and
//! dead entries leave the queue when it needs their slots, so neither
//! grows. Event notifications (#2303's third part) will evict the newest
//! COV entry when the outbox is full.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use bacnet_time::Instant;

use super::NotifyService;
use crate::cov::{CovSubscriptionKey, SubscriberEndpoint, Token};

/// How much the outbox holds: entries, and octets of their service
/// parameters. Both are allocated once. `octets` must hold the largest
/// notification the device sends (its Max_APDU_Length_Accepted at most),
/// or that notification is never queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxLimits {
    /// Notifications queued at once.
    pub entries: usize,
    /// Octets of their service parameters, together.
    pub octets: usize,
}

impl Default for OutboxLimits {
    /// 16 entries in 2,048 octets.
    fn default() -> Self {
        Self {
            entries: 16,
            octets: 2048,
        }
    }
}

/// One queued notification.
#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub(super) token: Token,
    pub(super) key: CovSubscriptionKey,
    pub(super) to: SubscriberEndpoint,
    pub(super) confirmed: bool,
    /// Which service it is. A report to a COV-multiple context may queue
    /// several notifications under one reference, so only a
    /// [`NotifyService::Cov`] one replaces, or is replaced.
    pub(super) service: NotifyService,
    /// When the table began the report: a confirmed one's answers are cut
    /// off from it, as the server's worker's are.
    pub(super) begun: Instant,
    start: usize,
    len: usize,
    live: bool,
}

impl Entry {
    /// Whether a new unconfirmed COVNotification for `key` takes this
    /// entry's place: it is the one queued for that subscription.
    fn replaces(&self, confirmed: bool, service: NotifyService, key: &CovSubscriptionKey) -> bool {
        let single = service == NotifyService::Cov && self.service == NotifyService::Cov;
        single && !confirmed && self.live && !self.confirmed && self.key == *key
    }
}

/// Room for more entries, as an evaluation reserves it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Room {
    entries: usize,
    octets: usize,
}

impl Room {
    /// Entries still free.
    pub(super) fn entries_free(&self) -> usize {
        self.entries
    }

    /// Octets still free.
    pub(super) fn octets_free(&self) -> usize {
        self.octets
    }

    /// Reserve one entry of `octets`, if both fit; or, when it replaces a
    /// queued entry of `replaced` octets, only the octets it adds.
    pub(super) fn take(&mut self, octets: usize, replaced: Option<usize>) -> bool {
        match replaced {
            Some(old) => {
                let more = octets.saturating_sub(old);
                if more > self.octets {
                    return false;
                }
                self.octets = self.octets - more + old.saturating_sub(octets);
                true
            }
            None => {
                if self.entries == 0 || octets > self.octets {
                    return false;
                }
                self.entries -= 1;
                self.octets -= octets;
                true
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Outbox {
    limits: OutboxLimits,
    buf: Vec<u8>,
    entries: VecDeque<Entry>,
    /// Entries still to send, and the octets of their parameters.
    live: usize,
    live_octets: usize,
}

impl Outbox {
    pub(super) fn new(limits: OutboxLimits) -> Self {
        Self {
            limits,
            buf: Vec::with_capacity(limits.octets),
            entries: VecDeque::with_capacity(limits.entries),
            live: 0,
            live_octets: 0,
        }
    }

    pub(super) fn limits(&self) -> OutboxLimits {
        self.limits
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// What may still be queued.
    pub(super) fn room(&self) -> Room {
        Room {
            entries: self.limits.entries - self.live,
            octets: self.limits.octets - self.live_octets,
        }
    }

    /// The octets of the unconfirmed notification queued for `key`, which a
    /// newer one would replace.
    pub(super) fn queued(&self, key: &CovSubscriptionKey) -> Option<usize> {
        self.entries
            .iter()
            .find(|entry| entry.replaces(false, NotifyService::Cov, key))
            .map(|entry| entry.len)
    }

    /// Queue a notification of `service`, confirmed or not, with service
    /// parameters `params`, if it fits.
    /// An unconfirmed one replaces, in its place, the one queued for its
    /// subscription, whose token it returns; a replaced one that no longer
    /// fits leaves too, and the new one isn't queued.
    pub(super) fn push(
        &mut self,
        token: Token,
        key: CovSubscriptionKey,
        to: SubscriberEndpoint,
        (confirmed, service): (bool, NotifyService),
        begun: Instant,
        params: &[u8],
    ) -> Pushed {
        let at = self
            .entries
            .iter()
            .position(|entry| entry.replaces(confirmed, service, &key));
        let replaced = at.map(|at| {
            let old = &mut self.entries[at];
            old.live = false;
            self.live -= 1;
            self.live_octets -= old.len;
            old.token
        });
        let fits = self.live < self.limits.entries
            && self.live_octets + params.len() <= self.limits.octets;
        if !fits {
            return Pushed {
                queued: false,
                replaced,
            };
        }
        if at.is_none() && self.entries.len() == self.limits.entries {
            // Dead entries give back their slots: no allocation.
            self.entries.retain(|entry| entry.live);
        }
        if self.buf.len() + params.len() > self.limits.octets {
            self.compact();
        }
        let start = self.buf.len();
        self.buf.extend_from_slice(params);
        let entry = Entry {
            token,
            key,
            to,
            confirmed,
            service,
            begun,
            start,
            len: params.len(),
            live: true,
        };
        match at {
            // The replaced entry's turn, now with the latest values.
            Some(at) => self.entries[at] = entry,
            None => self.entries.push_back(entry),
        }
        self.live += 1;
        self.live_octets += params.len();
        Pushed {
            queued: true,
            replaced,
        }
    }

    /// Move the live entries' parameters to the start of the buffer. Each
    /// moves down, and they go in the order their parameters sit, so none
    /// overwrites one not yet moved. Dead entries' parameters are dropped.
    fn compact(&mut self) {
        for entry in self.entries.iter_mut().filter(|entry| entry.len == 0) {
            entry.start = 0;
        }
        let mut next = 0;
        loop {
            let lowest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.live && entry.len > 0 && entry.start >= next)
                .min_by_key(|(_, entry)| entry.start)
                .map(|(at, _)| at);
            let Some(at) = lowest else {
                break;
            };
            let entry = &mut self.entries[at];
            self.buf
                .copy_within(entry.start..entry.start + entry.len, next);
            entry.start = next;
            next += entry.len;
        }
        self.buf.truncate(next);
    }

    /// Hand the oldest live entry and its parameters to `take`, and drop
    /// it; dead entries before it go too.
    pub(super) fn pop<R>(&mut self, take: impl FnOnce(&Entry, &[u8]) -> R) -> Option<R> {
        let taken = loop {
            let entry = self.entries.pop_front()?;
            if entry.live {
                self.live -= 1;
                self.live_octets -= entry.len;
                break take(&entry, &self.buf[entry.start..entry.start + entry.len]);
            }
        };
        if self.live == 0 {
            self.entries.clear();
            self.buf.clear();
        }
        Some(taken)
    }

    /// Mark dead every queued entry `dead` names.
    pub(super) fn kill(&mut self, dead: impl Fn(&Entry) -> bool) {
        for entry in &mut self.entries {
            if entry.live && dead(entry) {
                entry.live = false;
                self.live -= 1;
                self.live_octets -= entry.len;
            }
        }
    }

    /// Live entries queued for `key`.
    #[cfg(test)]
    pub(super) fn live_for(&self, key: &CovSubscriptionKey) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.live && entry.key == *key)
            .count()
    }
}

/// What [`Outbox::push`] did.
#[derive(Debug)]
pub(super) struct Pushed {
    /// The notification is queued.
    pub(super) queued: bool,
    /// The token of the queued notification it replaced, which ends unsent.
    pub(super) replaced: Option<Token>,
}

#[cfg(test)]
mod tests;
