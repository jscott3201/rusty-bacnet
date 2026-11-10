//! How the timestamped COV-multiple histories follow the subscription
//! table.
//!
//! The histories hold one entry for each COV-multiple reference the table
//! holds, and each context's delay and sizing terms. The table says what
//! its mutations changed ([`Fenced`]); [`follow_table`] applies that to the
//! histories. A host calls it after each table mutation, with what the
//! mutation published, before anything else reads the histories.
use bacnet_time::Instant;

use super::TimedHistories;
use crate::cov::{CovSubscriptionSnapshot, CovTable, CovTimeRemaining, Fenced};

/// Apply to `histories` what `table`'s mutations report in `fenced`, after
/// they `published` those snapshots, in the order the table made the
/// changes:
///
/// - a removed reference loses its history, or its owed mark;
/// - a published COV-multiple reference is bound to its new generation,
///   with its context's Max_Notification_Delay: a timestamped one keeps a
///   history, an untimestamped one only what it may come to be owed. Any
///   other published reference has neither;
/// - a refreshed context takes its admitted delay, its subscriber's maximum
///   APDU and the lifetime it has left at `now`.
pub fn follow_table(
    histories: &mut TimedHistories,
    table: &CovTable,
    fenced: &Fenced,
    published: &[CovSubscriptionSnapshot],
    now: Instant,
) {
    for key in &fenced.removed {
        histories.remove(key);
    }
    for snapshot in published {
        let delay = snapshot.max_notification_delay().unwrap_or_default();
        let (key, generation) = (snapshot.key(), snapshot.generation());
        match (key.multiple_context(), snapshot.timestamped) {
            (Some(_), true) => histories.reset(key, generation, delay),
            (Some(_), false) => histories.reset_untimed(key, generation, delay),
            (None, _) => histories.remove(key),
        }
    }
    for context in &fenced.contexts {
        let Some(entry) = table.multiple_context_references(context).next() else {
            continue;
        };
        histories.set_delay(context, entry.max_notification_delay().unwrap_or_default());
        // Lifetime left now, rounded up as notifications report it; an
        // expired one, which no notification reports, sizes as the longest.
        let time_remaining = CovTimeRemaining::at(entry.expires_at, now)
            .wire_seconds()
            .unwrap_or(u32::MAX);
        histories.set_sizing(context, entry.subscriber_max_apdu(), time_remaining);
    }
}
