//! Dropped timestamped COV-multiple changes: every one is counted, and each
//! drop is reported with whether it is its context's first for that cause
//! (#1039).
//!
//! A subscriber whose maximum APDU cannot hold one timestamped value of its
//! references loses every such value, and a context blocked for long keeps
//! evicting from its history bound. A warning per drop would flood a log in
//! both cases, so a report says whether it is the context's first for its
//! cause: a host warns for those and only counts the rest. A context reports
//! a first again once it is admitted afresh: a timestamped reference of it
//! is (re)published, or an admission changes the size of its notifications.

use super::{CovSubscriptionKey, TimedHistories};

/// Reports of later drops that wait for the host at most; see
/// [`TimedHistories::take_drops`].
const REPEATS_KEPT: usize = 64;

/// Why pending timestamped changes were dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropReason {
    /// Evicted on overflow of the context's history bound.
    HistoryFull,
    /// A value of the change too large for any notification to the
    /// subscriber, even on its own; the rest of the change still goes out
    /// (#1090).
    TooLarge,
    /// Superseded by a newer change of the reference that was delivered.
    Superseded,
}

impl DropReason {
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Pending timestamped changes of one reference dropped for one cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dropped {
    /// The reference that lost them.
    pub(crate) key: CovSubscriptionKey,
    /// How many changes.
    pub(crate) count: usize,
    /// Why.
    pub(crate) reason: DropReason,
    /// Octets one notification to the reference's context may take, while
    /// the context still has a timestamped reference.
    pub(crate) apdu: Option<usize>,
    /// Whether this is the context's first drop for `reason` since it was
    /// admitted afresh, the one worth a warning. A reference whose context
    /// lost its last timestamped reference meanwhile always reports a first.
    pub(crate) first: bool,
}

/// Causes a context has warned about since it was last admitted afresh.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DropWarnings(u8);

impl DropWarnings {
    /// Note a drop for `reason`; `true` if it is the first since the reset.
    fn first(&mut self, reason: DropReason) -> bool {
        let first = self.0 & reason.bit() == 0;
        self.0 |= reason.bit();
        first
    }
}

impl TimedHistories {
    /// Count `count` dropped changes of `key`, and report them: as a first
    /// unless its context already reported one for `reason`. A reference
    /// whose context has no terms left (it lost its last timestamped
    /// reference meanwhile) always reports a first.
    pub(super) fn dropped(&mut self, key: &CovSubscriptionKey, count: usize, reason: DropReason) {
        self.counts.changes_dropped += count as u64;
        let terms = key
            .multiple_context()
            .and_then(|context| self.terms.get_mut(context));
        let apdu = terms.as_ref().map(|terms| terms.apdu);
        let first = terms.is_none_or(|terms| terms.warned.first(reason));
        // Every drop is counted above. Of the reports, every first waits
        // for the host; later ones only while few wait, so a host that never
        // takes them holds a bounded list.
        if first || self.drops.len() < REPEATS_KEPT {
            self.drops.push(Dropped {
                key: key.clone(),
                count,
                reason,
                apdu,
                first,
            });
        }
    }

    /// The drops reported since the last call, oldest first. Take them after
    /// each call that can drop a change: the counts
    /// ([`Self::take_counts`]) always hold every drop, but once
    /// [`REPEATS_KEPT`] reports wait, only a context's first for a cause
    /// still joins them.
    pub(crate) fn take_drops(&mut self) -> Vec<Dropped> {
        core::mem::take(&mut self.drops)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{dropped, histories, key};
    use super::*;

    #[test]
    fn each_cause_warns_once_per_context_until_it_is_admitted_afresh() {
        let mut h = histories(8, 4);
        // The firsts reported so far: the drops a host warns about.
        let mut firsts = 0;
        let mut warnings = |h: &mut TimedHistories| {
            firsts += h.take_drops().iter().filter(|drop| drop.first).count();
            firsts
        };
        let (a, b, other) = (key(1, 1), key(1, 2), key(2, 1));
        for k in [&a, &b, &other] {
            h.reset(k, 1, 10);
        }
        let context = a.multiple_context().unwrap().clone();
        h.set_sizing(&context, Some(50), 60);
        // Several drops of one cause, across the context's references, warn
        // once; every change is counted.
        h.dropped(&a, 1, DropReason::TooLarge);
        h.dropped(&b, 2, DropReason::TooLarge);
        h.dropped(&a, 1, DropReason::TooLarge);
        assert_eq!((warnings(&mut h), dropped(&h)), (1, 4));
        // Another cause, or another context, warns for itself.
        h.dropped(&a, 1, DropReason::HistoryFull);
        h.dropped(&other, 1, DropReason::TooLarge);
        assert_eq!((warnings(&mut h), dropped(&h)), (3, 6));
        // A renewal advertising the same maximum APDU changes nothing...
        h.set_sizing(&context, Some(50), 60);
        h.dropped(&a, 1, DropReason::TooLarge);
        assert_eq!(warnings(&mut h), 3);
        // ...while one advertising another size warns afresh,
        h.set_sizing(&context, Some(128), 60);
        h.dropped(&a, 1, DropReason::TooLarge);
        assert_eq!(warnings(&mut h), 4);
        // and so does a re-admitted reference of the context.
        h.dropped(&b, 1, DropReason::TooLarge);
        assert_eq!(warnings(&mut h), 4);
        h.reset(&b, 2, 10);
        h.dropped(&a, 1, DropReason::TooLarge);
        assert_eq!((warnings(&mut h), dropped(&h)), (5, 10));
        // A reference whose context has no terms left always warns.
        h.remove(&other);
        h.dropped(&other, 1, DropReason::Superseded);
        h.dropped(&other, 1, DropReason::Superseded);
        assert_eq!((warnings(&mut h), dropped(&h)), (7, 12));
    }

    #[test]
    fn a_report_names_the_reference_the_count_the_cause_and_the_size() {
        let mut h = histories(8, 4);
        let k = key(1, 1);
        h.reset(&k, 1, 10);
        h.set_sizing(k.multiple_context().unwrap(), Some(50), 60);
        h.dropped(&k, 2, DropReason::HistoryFull);
        h.dropped(&k, 1, DropReason::HistoryFull);
        let report = |count, first| Dropped {
            key: k.clone(),
            count,
            reason: DropReason::HistoryFull,
            apdu: Some(50),
            first,
        };
        assert_eq!(h.take_drops(), [report(2, true), report(1, false)]);
        assert!(h.take_drops().is_empty(), "taken once");
        assert_eq!(h.take_counts().changes_dropped, 3);
        assert_eq!(h.take_counts().changes_dropped, 0, "taken once");
    }

    #[test]
    fn a_host_that_never_takes_the_reports_holds_a_bounded_list() {
        let mut h = histories(8, 4);
        let k = key(1, 1);
        h.reset(&k, 1, 10);
        for _ in 0..3 * REPEATS_KEPT {
            h.dropped(&k, 1, DropReason::HistoryFull);
        }
        // Another cause's first still joins the full list.
        h.dropped(&k, 1, DropReason::TooLarge);
        let drops = h.take_drops();
        assert_eq!(drops.len(), REPEATS_KEPT + 1);
        assert_eq!(
            drops.iter().filter(|drop| drop.first).count(),
            2,
            "both causes' firsts"
        );
        assert_eq!(drops.last().unwrap().reason, DropReason::TooLarge);
        assert_eq!(dropped(&h), 3 * REPEATS_KEPT as u64 + 1, "every drop");
    }
}
