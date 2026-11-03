//! The outbox: notifications decided but not yet handed out, as service
//! parameters in one buffer allocated when the [`Reporter`](super::Reporter)
//! is built, bounded by entries and by octets.
//!
//! Entries leave in the order they came. Their parameters sit back to back
//! in the buffer; when the free space at its end is too short, the queued
//! parameters move to its start, so the buffer never grows. An entry whose
//! report a table change fenced, or whose subscription left, is marked dead
//! and skipped. Event notifications (#2303's third part) will evict the
//! newest COV entry when the outbox is full, from the back.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

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
    start: usize,
    len: usize,
    live: bool,
}

/// Room for more entries, as an evaluation reserves it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Room {
    entries: usize,
    octets: usize,
}

impl Room {
    /// Reserve one entry of `octets`, if both fit.
    pub(super) fn take(&mut self, octets: usize) -> bool {
        if self.entries == 0 || octets > self.octets {
            return false;
        }
        self.entries -= 1;
        self.octets -= octets;
        true
    }
}

#[derive(Debug)]
pub(super) struct Outbox {
    limits: OutboxLimits,
    buf: Vec<u8>,
    /// Octets at the front of `buf` whose entries have left.
    head: usize,
    entries: VecDeque<Entry>,
}

impl Outbox {
    pub(super) fn new(limits: OutboxLimits) -> Self {
        Self {
            limits,
            buf: Vec::with_capacity(limits.octets),
            head: 0,
            entries: VecDeque::with_capacity(limits.entries),
        }
    }

    pub(super) fn limits(&self) -> OutboxLimits {
        self.limits
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// What may still be queued.
    pub(super) fn room(&self) -> Room {
        Room {
            entries: self.limits.entries - self.entries.len(),
            octets: self.limits.octets - (self.buf.len() - self.head),
        }
    }

    /// Queue a notification with service parameters `params`, if it fits.
    pub(super) fn push(
        &mut self,
        token: Token,
        key: CovSubscriptionKey,
        to: SubscriberEndpoint,
        confirmed: bool,
        params: &[u8],
    ) -> bool {
        if !self.room().take(params.len()) {
            return false;
        }
        if self.buf.len() + params.len() > self.limits.octets {
            // Move what is queued to the start: no allocation.
            self.buf.drain(..self.head);
            for entry in &mut self.entries {
                entry.start -= self.head;
            }
            self.head = 0;
        }
        let start = self.buf.len();
        self.buf.extend_from_slice(params);
        self.entries.push_back(Entry {
            token,
            key,
            to,
            confirmed,
            start,
            len: params.len(),
            live: true,
        });
        true
    }

    /// Hand the oldest live entry and its parameters to `take`, and drop
    /// it; dead entries before it go too.
    pub(super) fn pop<R>(&mut self, take: impl FnOnce(&Entry, &[u8]) -> R) -> Option<R> {
        let taken = loop {
            let entry = self.entries.pop_front()?;
            self.head = entry.start + entry.len;
            if entry.live {
                break take(&entry, &self.buf[entry.start..self.head]);
            }
        };
        if self.entries.is_empty() {
            self.buf.clear();
            self.head = 0;
        }
        Some(taken)
    }

    /// Mark dead every queued entry `dead` names.
    pub(super) fn kill(&mut self, dead: impl Fn(&Entry) -> bool) {
        for entry in &mut self.entries {
            if dead(entry) {
                entry.live = false;
            }
        }
    }
}

#[cfg(test)]
mod tests;
