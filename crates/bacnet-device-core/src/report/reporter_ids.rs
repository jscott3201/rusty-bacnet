//! The [`Reporter`]'s invoke IDs (#2512): reports owed for want of one are
//! kept apart from reports owed for room in the outbox, and looked at again
//! only as IDs come back, so [`next_deadline`](Reporter::next_deadline)
//! never asks a host to look while every ID it allows is out. The host
//! reports the IDs that come back from outside the reporter; the reporter
//! counts its own.

use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;

#[cfg(feature = "event-reporting")]
use super::super::outbox::Entry;
use super::Reporter;
use crate::cov::{CovSubscriptionKey, Token};
use crate::report::InvokeIds;

impl Reporter {
    /// Whether a report waits for an invoke ID: a [`pop`](Self::pop) found
    /// [`InvokeIds::take`] refusing, and no look has served it since. While
    /// this holds, a host whose IDs other work shares watches for them
    /// coming back and reports them with
    /// [`invoke_ids_released`](Self::invoke_ids_released); one report
    /// getting an ID doesn't end the wait while others remain.
    pub fn owes_invoke_id(&self) -> bool {
        !self.owed_ids.is_empty() || self.outbox.held() > 0
    }

    /// Invoke IDs came back to the host's pool from outside this reporter,
    /// such as the device's own client requests ending, and `free` are free
    /// now: up to that many of the reports owed one are looked at again at
    /// the next [`on_deadline`](Self::on_deadline), in one pass, oldest
    /// first. `free` is how many the host has, not how many came back, so
    /// one this reporter gave back itself, which it counts, isn't counted
    /// twice. A host need report only while
    /// [`owes_invoke_id`](Self::owes_invoke_id) holds, and a host that
    /// refuses IDs past a bound of its own needn't report anything: only
    /// this reporter's own notifications ending free a place under it.
    pub fn invoke_ids_released(&mut self, free: usize) {
        self.ids_freed = self.ids_freed.max(free);
    }

    /// When the confirmed notification `token` names next times out, if it
    /// is out: the current attempt's answer deadline, or its cutoff when
    /// that comes first or a resend waits for the next [`pop`](Self::pop).
    /// [`next_deadline`](Self::next_deadline) is the earliest of these and
    /// the reporter's other deadlines; this is for a host that keeps
    /// something per notification.
    pub fn notification_deadline(&self, token: Token) -> Option<Instant> {
        self.confirmed.deadline_of(token)
    }

    /// Give `id`, taken for `peer`, back to `ids`, and count it for the
    /// reports owed one.
    pub(super) fn give_back(&mut self, ids: &mut dyn InvokeIds, peer: &BACnetAddress, id: u8) {
        ids.release(peer, id);
        self.count_freed(1);
    }

    /// Count `count` IDs given back. Only those counted while a report
    /// waits for one matter: the first report owed one drops the rest
    /// ([`owe_id`](Self::owe_id)).
    pub(super) fn count_freed(&mut self, count: usize) {
        self.ids_freed = self.ids_freed.saturating_add(count);
    }

    /// Owe `key` a look once an invoke ID comes back; counted as
    /// [`owe`](Self::owe) counts.
    pub(super) fn owe_id(&mut self, key: CovSubscriptionKey) {
        // IDs counted while nothing was owed one, or for reports owed
        // before and since served or gone, say nothing of this one.
        if !self.owes_invoke_id() {
            self.ids_freed = 0;
        }
        if !self.owed_ids.contains(&key) && !self.owed.contains(&key) {
            self.counters.owed += 1;
            self.owed_ids.push(key);
        }
    }

    #[cfg(feature = "event-reporting")]
    /// Hold the confirmed event notification `entry`, which `pop` took from
    /// `at` and found no invoke ID for, in that place until one comes back
    /// (#2303): each `pop` tries the held notifications first, oldest
    /// first, then anything newer. It is counted owed once, however often
    /// it is refused.
    pub(super) fn hold_event(&mut self, at: usize, entry: Entry, params: &[u8]) {
        if !self.owes_invoke_id() {
            self.ids_freed = 0;
        }
        if !entry.counted {
            self.counters.owed += 1;
        }
        self.outbox.hold(at, entry, params, true);
    }

    /// Owe nothing more to the references `gone` names, for room or an ID.
    pub(super) fn forgive(&mut self, gone: impl Fn(&CovSubscriptionKey) -> bool) {
        self.owed.retain(|key| !gone(key));
        self.owed_ids.retain(|key| !gone(key));
    }
}
