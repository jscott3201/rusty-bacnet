//! Claims on drained timestamped changes and prepared untimestamped values,
//! and the one report an unconfirmed context sends at a time (#986, #1008,
//! #1038, #1090).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::time::Instant;

use super::{CovSubscriptionKey, DropReason, MultipleContextKey, TimedChange, TimedStore};
use crate::cov::CovRevisits;

/// Changes drained into one notification, and the untimestamped references
/// whose prepared values it carries. Unless [`TimedClaim::commit`] is called
/// once the notification is delivered, dropping the claim returns the changes
/// to their references. An untimestamped reference stays owed if it already
/// was, and becomes owed if the claim is [`TimedClaim::owing`]: its report
/// began going out without it (#1038). Otherwise an undelivered value is
/// handled like any lost notification, by the reference's next fanout.
///
/// A claim is only a set of changes. How a notification conveys them, as
/// history or as a reference's current state, is decided by the report that
/// builds it; [`TimedClaim::split_oldest`] moves the oldest of them into a
/// claim of their own, for a notification that goes out first (#986, #1008),
/// [`TimedClaim::split_values`] parts a change too large for any
/// notification into one claim per value (#1090), and
/// [`TimedClaim::split_untimed`] moves untimestamped references into a claim
/// of their own.
#[derive(Debug)]
pub(crate) struct TimedClaim {
    store: TimedStore,
    changes: Vec<(CovSubscriptionKey, u64, Vec<TimedChange>)>,
    /// Untimestamped references, each with its generation and, if it was
    /// owed when this report took it, since when.
    untimed: Vec<(CovSubscriptionKey, u64, Option<Instant>)>,
    /// Whether returning these changes applies the context bound.
    evict: bool,
    /// Whether an undelivered untimestamped reference becomes owed.
    owing: bool,
}

impl TimedClaim {
    pub(crate) fn new(store: TimedStore) -> Self {
        Self {
            store,
            changes: Vec::new(),
            untimed: Vec::new(),
            evict: true,
            owing: false,
        }
    }

    /// Carry the prepared values of an untimestamped reference, owed since
    /// `owed_since` if it was owed.
    pub(crate) fn add_untimed(
        &mut self,
        key: CovSubscriptionKey,
        generation: u64,
        owed_since: Option<Instant>,
    ) {
        self.untimed.push((key, generation, owed_since));
    }

    /// Move the untimestamped references among `keys` into a claim of their
    /// own, for a notification that carries their values.
    pub(crate) fn split_untimed(&mut self, keys: &HashSet<CovSubscriptionKey>) -> Self {
        let (moved, kept) = std::mem::take(&mut self.untimed)
            .into_iter()
            .partition(|(key, _, _)| keys.contains(key));
        self.untimed = kept;
        Self {
            store: self.store.clone(),
            changes: Vec::new(),
            untimed: moved,
            evict: self.evict,
            owing: self.owing,
        }
    }

    /// Give up the untimestamped references left in this claim without owing
    /// them, counting each in
    /// [`untimed_references_oversized`](crate::cov::AtomicCovCounters::untimed_references_oversized):
    /// their values fit no notification, so a later attempt would only fail
    /// again. They are evaluated again at their next fanout (#1066).
    pub(crate) fn forgo_untimed(&mut self) {
        let left_out = std::mem::take(&mut self.untimed).len();
        if left_out > 0 {
            self.store
                .lock()
                .counters
                .untimed_references_oversized
                .fetch_add(left_out as u64, Ordering::Relaxed);
        }
    }

    /// Return these changes without applying the context bound: a report
    /// planned to send them and only defers them, so the bound must not drop
    /// what it just drained (#986).
    pub(crate) fn without_eviction(mut self) -> Self {
        self.evict = false;
        self
    }

    /// The report this part belongs to began going out without it, so its
    /// untimestamped references are owed until a later report delivers them
    /// or finds them settled (#1038).
    pub(crate) fn owing(mut self) -> Self {
        self.owing = true;
        self
    }

    pub(crate) fn add(
        &mut self,
        key: CovSubscriptionKey,
        incarnation: u64,
        changes: Vec<TimedChange>,
    ) {
        if !changes.is_empty() {
            self.changes.push((key, incarnation, changes));
        }
    }

    /// Move the `count` oldest claimed changes, across every reference and
    /// latest changes included (all of them, if fewer), into a claim of their
    /// own, for a notification that goes out before this one.
    pub(crate) fn split_oldest(&mut self, count: usize) -> Self {
        let mut part = Self {
            store: self.store.clone(),
            changes: Vec::new(),
            untimed: Vec::new(),
            evict: self.evict,
            owing: self.owing,
        };
        let cut = {
            let all = self.in_order();
            let moved = count.min(all.len());
            moved.checked_sub(1).map(|last| all[last].1.seq)
        };
        let Some(cut) = cut else {
            return part;
        };
        for (key, incarnation, changes) in &mut self.changes {
            let moved = changes.partition_point(|change| change.seq <= cut);
            if moved > 0 {
                part.changes
                    .push((key.clone(), *incarnation, changes.drain(..moved).collect()));
            }
        }
        self.changes.retain(|(_, _, changes)| !changes.is_empty());
        part
    }

    /// Part this claim's changes, each too large for one notification even
    /// alone, into claims of one value each, for notifications that go out in
    /// the order returned: capture order, and each change's values in the
    /// order they were captured (§13.1 lets several notifications convey the
    /// changes; #1090). `fit` judges the notification of one part alone.
    ///
    /// A value too large even then is given up, since every attempt to send
    /// it would fail; each change that loses one counts once as dropped, and
    /// the context warns once until it is admitted afresh (#1039). A value
    /// that would carry nothing is left out. Of the parts kept, only the one
    /// with a change's last value delivers the change when it is delivered.
    pub(crate) fn split_values(
        mut self,
        fit: impl Fn(&CovSubscriptionKey, &TimedChange) -> ValueFit,
    ) -> Vec<Self> {
        let mut all: Vec<_> = std::mem::take(&mut self.changes)
            .into_iter()
            .flat_map(|(key, incarnation, changes)| {
                changes
                    .into_iter()
                    .map(move |change| (key.clone(), incarnation, change))
            })
            .collect();
        all.sort_by_key(|(_, _, change)| change.seq);
        let mut parts = Vec::new();
        let mut lost = Vec::new();
        for (key, incarnation, change) in all {
            let mut kept = Vec::new();
            let mut too_large = false;
            for value in change.into_values() {
                match fit(&key, &value) {
                    ValueFit::Fits => kept.push(value),
                    ValueFit::TooLarge => too_large = true,
                    ValueFit::Empty => {}
                }
            }
            if too_large {
                lost.push(key.clone());
            }
            if let Some(last) = kept.last_mut() {
                last.continues = false;
            }
            parts.extend(kept.into_iter().map(|value| Self {
                store: self.store.clone(),
                changes: vec![(key.clone(), incarnation, vec![value])],
                untimed: Vec::new(),
                evict: self.evict,
                owing: self.owing,
            }));
        }
        if !lost.is_empty() {
            let mut store = self.store.lock();
            for key in lost {
                store.dropped(&key, 1, DropReason::TooLarge);
            }
        }
        parts
    }

    /// Every claimed change, in capture order.
    pub(crate) fn in_order(&self) -> Vec<(&CovSubscriptionKey, &TimedChange)> {
        let mut all: Vec<_> = self
            .changes
            .iter()
            .flat_map(|(key, _, changes)| changes.iter().map(move |change| (key, change)))
            .collect();
        all.sort_by_key(|(_, change)| change.seq);
        all
    }

    /// Each claimed reference with its last claimed change.
    pub(crate) fn last_changes(&self) -> impl Iterator<Item = (&CovSubscriptionKey, &TimedChange)> {
        self.changes
            .iter()
            .filter_map(|(key, _, changes)| Some((key, changes.last()?)))
    }

    /// The notification carrying these changes and values was delivered:
    /// retire them. A part whose change continues in a later notification
    /// delivers nothing of that change by itself (#1090); the change is in
    /// delivery from then on (#1163).
    pub(crate) fn commit(mut self) {
        self.untimed.clear();
        let mut store = self.store.lock();
        for (key, incarnation, changes) in self.changes.drain(..) {
            match changes.iter().rev().find(|change| !change.continues) {
                Some(last) => store.commit(&key, incarnation, last.seq),
                None => {
                    if let Some(part) = changes.last() {
                        store.deliver_by_value(&key, incarnation, part.seq);
                    }
                }
            }
        }
    }

    /// This part goes out as a confirmed report's first, and the report's
    /// later parts wait in the queue for its Ack. A change of which it
    /// carries only some values is in delivery from now on, so that the bound
    /// keeps the rest of it (#1163).
    pub(crate) fn going_out(&self) {
        let mut ahead = self
            .changes
            .iter()
            .filter_map(|(key, incarnation, changes)| {
                let part = changes.last().filter(|part| part.continues)?;
                Some((key, *incarnation, part.seq))
            })
            .peekable();
        if ahead.peek().is_some() {
            let mut store = self.store.lock();
            for (key, incarnation, seq) in ahead {
                store.deliver_by_value(key, incarnation, seq);
            }
        }
    }
}

impl Drop for TimedClaim {
    fn drop(&mut self) {
        if self.changes.is_empty() && self.untimed.is_empty() {
            return;
        }
        let mut store = self.store.lock();
        for (key, incarnation, changes) in self.changes.drain(..) {
            store.requeue(&key, incarnation, changes, self.evict);
        }
        for (key, generation, owed_since) in self.untimed.drain(..) {
            if let Some(since) = owed_since.or_else(|| self.owing.then(Instant::now)) {
                store.owe(&key, generation, since);
            }
        }
    }
}

/// How the notification of one value of an oversized change would go out
/// (#1090).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueFit {
    /// Within the subscriber's limit.
    Fits,
    /// Over the limit even with that value alone.
    TooLarge,
    /// With nothing in it: the context reports that coordinate untimestamped.
    Empty,
}

/// The one report an unconfirmed context is sending (#986, #1038).
///
/// Its parts go out one await apart, and an unconfirmed context has no
/// outstanding-report mark, so another fanout could send a newer change or
/// value between them: the subscriber would see a reference move back to
/// older history, or to the older value a later part carries. While a turn is
/// held, other fanouts of the context stand back and leave their changes
/// queued. Dropping the turn, after the last part or on any early return,
/// hands the context to one follow-up if a fanout stood back; the follow-up
/// reads the untimestamped values afresh.
#[must_use = "dropping the turn ends the report"]
pub(crate) struct SendTurn {
    store: TimedStore,
    context: MultipleContextKey,
    revisits: Arc<CovRevisits>,
    keys: Vec<CovSubscriptionKey>,
}

impl SendTurn {
    /// Take the turn of `context`, unless a report of it is still going out.
    /// `keys` are the context's references, for the follow-up.
    pub(crate) fn begin(
        store: &TimedStore,
        context: &MultipleContextKey,
        revisits: &Arc<CovRevisits>,
        keys: Vec<CovSubscriptionKey>,
    ) -> Option<Self> {
        store.lock().begin_send(context).then(|| Self {
            store: store.clone(),
            context: context.clone(),
            revisits: Arc::clone(revisits),
            keys,
        })
    }
}

impl Drop for SendTurn {
    fn drop(&mut self) {
        let owed = self.store.lock().end_send(&self.context);
        if owed {
            self.revisits.request(std::mem::take(&mut self.keys));
        }
    }
}
