//! The [`Reporter`]'s invoke IDs (#2512): reports owed for want of one are
//! kept apart from reports owed for room in the outbox, and looked at again
//! only as IDs come back, so [`next_deadline`](Reporter::next_deadline)
//! never asks a host to look while every ID it allows is out. The host
//! reports the IDs that come back from outside the reporter; the reporter
//! counts its own.

use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;

use super::Reporter;
use crate::cov::{CovSubscriptionKey, Token};
use crate::report::InvokeIds;

impl Reporter {
    /// Invoke IDs came back to the host's pool from outside this reporter,
    /// such as the device's own client requests ending: the reports owed
    /// for want of one are looked at again at the next
    /// [`on_deadline`](Self::on_deadline), as many as came back. The
    /// reporter counts the IDs it gives back itself; a host need report
    /// only the others, and only while a report waits for one (a
    /// [`pop`](Self::pop) found [`InvokeIds::take`] refusing). A host that
    /// refuses IDs past a bound of its own needn't report anything: only
    /// this reporter's own notifications ending free a place under it.
    pub fn invoke_ids_released(&mut self, count: usize) {
        self.ids_freed = self.ids_freed.saturating_add(count);
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
        self.ids_freed = self.ids_freed.saturating_add(1);
    }

    /// Owe `key` a look once an invoke ID comes back; counted as
    /// [`owe`](Self::owe) counts.
    pub(super) fn owe_id(&mut self, key: CovSubscriptionKey) {
        if !self.owed_ids.contains(&key) && !self.owed.contains(&key) {
            self.counters.owed += 1;
            self.owed_ids.push(key);
        }
    }

    /// Owe nothing more to the references `gone` names, for room or an ID.
    pub(super) fn forgive(&mut self, gone: impl Fn(&CovSubscriptionKey) -> bool) {
        self.owed.retain(|key| !gone(key));
        self.owed_ids.retain(|key| !gone(key));
    }
}
