//! One row for a write and the event-state transition it causes (#2535).
//!
//! A write's capture queues the reference's row as the write commits. When
//! the evaluation that follows commits a transition, the capture taken after
//! it stands in for that row instead of queueing a second, so a subscriber
//! never gets a row showing the object between the two.

use super::{Held, TimedChange, TimedHistories};
use crate::cov::CovSubscriptionKey;

impl TimedHistories {
    /// Queue `change` in place of the reference's newest pending change, the
    /// row the same write's capture queued.
    ///
    /// The row is replaced only while it is still the reference's newest
    /// captured change, still pending (no report has drained it) and not
    /// partly sent. Otherwise `change` is queued after it, as
    /// [`push`](Self::push) queues a change. A report drains pending changes
    /// only under the guard it reads the objects under, so a host that runs
    /// the write's capture and this one under one database guard always
    /// replaces, whatever the context's Max_Notification_Delay. The
    /// replacement keeps the replaced row's capture instant, so its
    /// Max_Notification_Delay still runs from the write. Returns `false`
    /// when the generation is no longer live and nothing was queued.
    pub fn replace_latest(
        &mut self,
        key: &CovSubscriptionKey,
        generation: u64,
        mut change: TimedChange,
    ) -> bool {
        let Some(history) = self.history_mut(key, generation) else {
            return false;
        };
        let replaceable = history.entries.back().is_some_and(|newest| {
            newest.seq == history.latest && newest.seq != history.in_delivery
        });
        if replaceable && let Some(replaced) = history.entries.pop_back() {
            if replaced.captured_at < change.captured_at {
                change.captured_at = replaced.captured_at;
            }
            self.release(key, Held::of([&replaced]));
        }
        self.push(key, generation, change)
    }
}
