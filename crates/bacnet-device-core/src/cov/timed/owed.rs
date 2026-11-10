//! Untimestamped COV-multiple references whose values a report began going
//! out without (#1038).
//!
//! Untimestamped values have no queue: each report reads them afresh, so the
//! newest value always wins. What a reference needs once a report that split
//! its values leaves it undelivered is a later report, and the mark kept here
//! gets it one: the deadline backstop hands an owed reference out like a
//! pending timestamped change of its context, measured from when it was first
//! owed, and a hold-off or re-enabled communication moves that attempt the
//! same way. The report that next evaluates the reference takes the mark,
//! whether or not the reference still has anything to report, and owes it
//! again only if that report does not deliver it either.

use bacnet_time::{Duration, Instant};

use super::{CovSubscriptionKey, MultipleContextKey, TimedHistories, UntimedReference};

impl TimedHistories {
    /// Bind a (re)published untimestamped reference generation with its
    /// context's admitted Max_Notification_Delay in seconds. Whatever an
    /// earlier generation owed goes with it: the admission's initial report
    /// conveys the reference afresh.
    pub fn reset_untimed(&mut self, key: &CovSubscriptionKey, generation: u64, delay: u32) {
        self.remove(key);
        self.untimed.insert(
            key.clone(),
            UntimedReference {
                generation,
                delay: Duration::from_secs(u64::from(delay)),
                owed_since: None,
            },
        );
    }

    /// Take the mark of a live untimestamped reference that a report is
    /// evaluating now: since when it is owed, if it is.
    pub fn take_owed(&mut self, key: &CovSubscriptionKey, generation: u64) -> Option<Instant> {
        self.untimed
            .get_mut(key)
            .filter(|reference| reference.generation == generation)?
            .owed_since
            .take()
    }

    /// A report did not deliver the values it prepared of a live
    /// untimestamped reference, owed since `since`. A cancelled or renewed
    /// reference owes nothing.
    pub(super) fn owe(&mut self, key: &CovSubscriptionKey, generation: u64, since: Instant) {
        let Some(reference) = self
            .untimed
            .get_mut(key)
            .filter(|reference| reference.generation == generation)
        else {
            return;
        };
        if reference.owed_since.is_none_or(|owed| since < owed) {
            reference.owed_since = Some(since);
            // The context's deadline may have moved earlier.
            self.moved = true;
        }
    }

    /// Whether some untimestamped reference of `context` is owed.
    pub(super) fn owes(&self, context: &MultipleContextKey) -> bool {
        self.untimed.iter().any(|(key, reference)| {
            reference.owed_since.is_some() && key.multiple_context() == Some(context)
        })
    }
}
