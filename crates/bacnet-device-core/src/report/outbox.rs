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
//! grows.
//!
//! An event notification that doesn't fit evicts the newest queued
//! COVNotification, confirmed or not, then the next newest, until it fits;
//! the reporter owes each evicted subscription a later look. One that fits
//! nowhere even then is dropped. A COV-multiple notification isn't evicted:
//! it holds its context's turn, and neither is a held event notification.
//! A confirmed event notification that finds no invoke ID goes back to its
//! place, held, and waits there for one: it can't be worked out again
//! later, as a COV report can. A later one to the same recipient is held
//! behind it, so each recipient hears its events in order.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[cfg(feature = "event-reporting")]
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_time::Instant;
#[cfg(feature = "event-reporting")]
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};

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

/// Whom a queued notification is for.
#[derive(Debug, Clone)]
pub(super) enum Route {
    /// A COV subscription, and the route it arrived on.
    Cov {
        key: CovSubscriptionKey,
        to: SubscriberEndpoint,
    },
    /// A Notification Class recipient, resolved at `pop`, at the network
    /// priority its event priority gives.
    #[cfg(feature = "event-reporting")]
    Event {
        to: BACnetRecipient,
        priority: NetworkPriority,
    },
}

/// One queued notification.
#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub(super) token: Token,
    pub(super) route: Route,
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
    /// A confirmed event notification waiting in its place for an invoke
    /// ID, or behind one to its recipient that is: `pop` passes it by.
    held: bool,
    /// It has waited for an ID before, and was counted owed then.
    #[cfg(feature = "event-reporting")]
    pub(super) counted: bool,
}

impl Entry {
    /// The subscription a COV notification reports to.
    pub(super) fn key(&self) -> Option<&CovSubscriptionKey> {
        match &self.route {
            Route::Cov { key, .. } => Some(key),
            #[cfg(feature = "event-reporting")]
            Route::Event { .. } => None,
        }
    }

    /// Whether a new unconfirmed COVNotification for `key` takes this
    /// entry's place: it is the one queued for that subscription.
    fn replaces(&self, confirmed: bool, service: NotifyService, key: &CovSubscriptionKey) -> bool {
        let single = service == NotifyService::Cov && self.service == NotifyService::Cov;
        single && !confirmed && self.live && !self.confirmed && self.key() == Some(key)
    }

    /// Whether an event notification may evict it: a live COVNotification.
    #[cfg(feature = "event-reporting")]
    fn evictable(&self) -> bool {
        self.live && self.service == NotifyService::Cov
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
            route: Route::Cov { key, to },
            confirmed,
            service,
            begun,
            start,
            len: params.len(),
            live: true,
            held: false,
            #[cfg(feature = "event-reporting")]
            counted: false,
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

    /// Queue an event notification for `to`, with service parameters
    /// `params`, evicting the newest COVNotifications queued while it
    /// doesn't fit. What it evicted comes back, newest first, whether or
    /// not it was then queued: it is queued unless no COVNotification was
    /// left to evict, or `params` would not fit an empty outbox, which
    /// evicts nothing.
    #[cfg(feature = "event-reporting")]
    pub(super) fn push_event(
        &mut self,
        token: Token,
        (to, priority): (BACnetRecipient, NetworkPriority),
        confirmed: bool,
        begun: Instant,
        params: &[u8],
    ) -> EventPushed {
        let mut pushed = EventPushed::default();
        if self.limits.entries == 0 || params.len() > self.limits.octets {
            return pushed;
        }
        let fits = |outbox: &Self| {
            outbox.live < outbox.limits.entries
                && outbox.live_octets + params.len() <= outbox.limits.octets
        };
        while !fits(self) {
            let Some(newest) = self
                .entries
                .iter_mut()
                .rev()
                .find(|entry| entry.evictable())
            else {
                return pushed;
            };
            newest.live = false;
            self.live -= 1;
            self.live_octets -= newest.len;
            pushed.evicted.push(newest.clone());
        }
        let entry = Entry {
            token,
            route: Route::Event { to, priority },
            confirmed,
            service: NotifyService::Event,
            begun,
            start: 0,
            len: 0,
            live: true,
            held: false,
            #[cfg(feature = "event-reporting")]
            counted: false,
        };
        self.insert(None, entry, params);
        pushed.queued = true;
        pushed
    }

    #[cfg(feature = "event-reporting")]
    /// Put `entry`, with `params`, at `at` in the queue (at its end for
    /// `None`), live; the caller has checked it fits.
    fn insert(&mut self, at: Option<usize>, mut entry: Entry, params: &[u8]) {
        if self.entries.len() == self.limits.entries {
            // A held entry goes back where `pop` just took it from, so only
            // a new one finds every slot used.
            debug_assert!(at.is_none(), "a slot is free where it left");
            // Dead entries give back their slots: no allocation.
            self.entries.retain(|entry| entry.live);
        }
        if self.buf.len() + params.len() > self.limits.octets {
            self.compact();
        }
        entry.start = self.buf.len();
        entry.len = params.len();
        entry.live = true;
        self.buf.extend_from_slice(params);
        match at {
            Some(at) => self.entries.insert(at.min(self.entries.len()), entry),
            None => self.entries.push_back(entry),
        }
        self.live += 1;
        self.live_octets += params.len();
    }

    #[cfg(feature = "event-reporting")]
    /// Put back an event notification [`pop`](Self::pop) took from `at`,
    /// held in that place: it found no invoke ID (`refused`, counted owed
    /// once), or an older one to its recipient is held. `pop` passes held
    /// entries by; [`held_entries`](Self::held_entries) and
    /// [`take_at`](Self::take_at) serve them. It fits, having just left.
    pub(super) fn hold(&mut self, at: usize, mut entry: Entry, params: &[u8], refused: bool) {
        entry.held = true;
        entry.counted |= refused;
        self.insert(Some(at), entry, params);
    }

    /// Confirmed event notifications held, each waiting for an invoke ID or
    /// behind one that is.
    pub(super) fn held(&self) -> usize {
        #[cfg(feature = "event-reporting")]
        return self
            .held_entries()
            .filter(|(_, entry)| entry.confirmed)
            .count();
        #[cfg(not(feature = "event-reporting"))]
        0
    }

    #[cfg(feature = "event-reporting")]
    /// The held entries, oldest first, with their places.
    pub(super) fn held_entries(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.live && entry.held)
    }

    #[cfg(feature = "event-reporting")]
    /// Whether an event notification to `peer` is held.
    pub(super) fn holds_for(&self, peer: &BACnetAddress) -> bool {
        self.held_entries().any(|(_, entry)| match &entry.route {
            Route::Event {
                to: BACnetRecipient::Address(address),
                ..
            } => address == peer,
            _ => false,
        })
    }

    #[cfg(feature = "event-reporting")]
    /// Take the live entry at `at` out, with its parameters.
    pub(super) fn take_at(&mut self, at: usize) -> (Entry, Vec<u8>) {
        let entry = self.entries.remove(at).expect("a held entry's place");
        debug_assert!(entry.live);
        let params = self.buf[entry.start..entry.start + entry.len].to_vec();
        self.live -= 1;
        self.live_octets -= entry.len;
        if self.live == 0 {
            self.entries.clear();
            self.buf.clear();
        }
        (entry, params)
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

    /// Take the oldest live entry not held, with its parameters and the
    /// place it had; dead entries ahead of every live one go too.
    pub(super) fn pop(&mut self) -> Option<(usize, Entry, Vec<u8>)> {
        while self.entries.front().is_some_and(|entry| !entry.live) {
            self.entries.pop_front();
        }
        let at = self
            .entries
            .iter()
            .position(|entry| entry.live && !entry.held)?;
        let entry = self.entries.remove(at).expect("found above");
        let params = self.buf[entry.start..entry.start + entry.len].to_vec();
        self.live -= 1;
        self.live_octets -= entry.len;
        if self.live == 0 {
            self.entries.clear();
            self.buf.clear();
        }
        Some((at, entry, params))
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
            .filter(|entry| entry.live && entry.key() == Some(key))
            .count()
    }
}

/// What [`Outbox::push_event`] did.
#[cfg(feature = "event-reporting")]
#[derive(Debug, Default)]
pub(super) struct EventPushed {
    /// The notification is queued.
    pub(super) queued: bool,
    /// The COV notifications it evicted, newest first: each ends unsent.
    pub(super) evicted: Vec<Entry>,
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
