//! Claims on drained timestamped changes and prepared untimestamped values
//! (#986, #1008, #1038, #1090), and how one is settled.
//!
//! A claim is plain data: whoever holds it settles it, with
//! [`TimedHistories::commit`] once its notification is delivered or
//! [`TimedHistories::give_back`] when it is not. A host that hands a claim
//! to something it may lose, a task that can be aborted, lodges it instead
//! ([`TimedHistories::lodge`]) and settles it by its [`ClaimId`]: the
//! histories keep a lodged claim and give it back themselves at its
//! deadline if nobody settled it, as a confirmed report ends itself at its
//! own.

use alloc::vec;
use alloc::vec::Vec;
use bacnet_time::Instant;

use super::{CovSubscriptionKey, DropReason, TimedChange, TimedHistories};

/// Changes drained into one notification, and the untimestamped references
/// whose prepared values it carries. It is settled once: committed when the
/// notification is delivered, which retires them, or given back, which
/// returns the changes to their references. An untimestamped reference given
/// back stays owed if it already was, and becomes owed if the claim is
/// [`TimedClaim::owing`]: its report began going out without it (#1038).
/// Otherwise an undelivered value is handled like any lost notification, by
/// the reference's next fanout.
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
#[must_use = "a claim that is neither committed, given back nor lodged loses its changes"]
pub struct TimedClaim {
    changes: Vec<(CovSubscriptionKey, u64, Vec<TimedChange>)>,
    /// Untimestamped references, each with its generation and, if it was
    /// owed when this report took it, since when.
    untimed: Vec<(CovSubscriptionKey, u64, Option<Instant>)>,
    /// Whether returning these changes applies the context bound.
    evict: bool,
    /// Whether an undelivered untimestamped reference becomes owed.
    owing: bool,
}

impl Default for TimedClaim {
    fn default() -> Self {
        Self::new()
    }
}

impl TimedClaim {
    /// A claim of nothing yet.
    pub fn new() -> Self {
        Self {
            changes: Vec::new(),
            untimed: Vec::new(),
            evict: true,
            owing: false,
        }
    }

    /// Whether the claim holds neither a change nor a reference, so
    /// settling it changes nothing.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.untimed.is_empty()
    }

    /// A claim of nothing with this one's settings.
    fn part(&self) -> Self {
        Self {
            changes: Vec::new(),
            untimed: Vec::new(),
            evict: self.evict,
            owing: self.owing,
        }
    }

    /// Carry the prepared values of an untimestamped reference, owed since
    /// `owed_since` if it was owed.
    pub fn add_untimed(
        &mut self,
        key: CovSubscriptionKey,
        generation: u64,
        owed_since: Option<Instant>,
    ) {
        self.untimed.push((key, generation, owed_since));
    }

    /// Move the untimestamped references `moves` selects into a claim of
    /// their own, for a notification that carries their values.
    pub fn split_untimed(&mut self, moves: impl Fn(&CovSubscriptionKey) -> bool) -> Self {
        let (moved, kept) = core::mem::take(&mut self.untimed)
            .into_iter()
            .partition(|(key, _, _)| moves(key));
        self.untimed = kept;
        Self {
            untimed: moved,
            ..self.part()
        }
    }

    /// Return these changes without applying the context bound: a report
    /// planned to send them and only defers them, so the bound must not drop
    /// what it just drained (#986).
    pub fn without_eviction(mut self) -> Self {
        self.evict = false;
        self
    }

    /// The report this part belongs to began going out without it, so its
    /// untimestamped references are owed until a later report delivers them
    /// or finds them settled (#1038).
    pub fn owing(mut self) -> Self {
        self.owing = true;
        self
    }

    /// Carry `changes` of `key`, drained from the history `incarnation`.
    pub fn add(&mut self, key: CovSubscriptionKey, incarnation: u64, changes: Vec<TimedChange>) {
        if !changes.is_empty() {
            self.changes.push((key, incarnation, changes));
        }
    }

    /// Move the `count` oldest claimed changes, across every reference and
    /// latest changes included (all of them, if fewer), into a claim of their
    /// own, for a notification that goes out before this one.
    pub fn split_oldest(&mut self, count: usize) -> Self {
        let mut part = self.part();
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
    /// it would fail: the second list names the reference of each change
    /// that lost one, once per change, for
    /// [`TimedHistories::lost_values`] to count. A value that would carry
    /// nothing is left out. Of the parts kept, only the one with a change's
    /// last value delivers the change when it is delivered.
    pub fn split_values(
        mut self,
        fit: impl Fn(&CovSubscriptionKey, &TimedChange) -> ValueFit,
    ) -> (Vec<Self>, Vec<CovSubscriptionKey>) {
        let mut all: Vec<_> = core::mem::take(&mut self.changes)
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
                changes: vec![(key.clone(), incarnation, vec![value])],
                ..self.part()
            }));
        }
        (parts, lost)
    }

    /// Every claimed change, in capture order.
    pub fn in_order(&self) -> Vec<(&CovSubscriptionKey, &TimedChange)> {
        let mut all: Vec<_> = self
            .changes
            .iter()
            .flat_map(|(key, _, changes)| changes.iter().map(move |change| (key, change)))
            .collect();
        all.sort_by_key(|(_, change)| change.seq);
        all
    }

    /// Each claimed reference with its last claimed change.
    pub fn last_changes(&self) -> impl Iterator<Item = (&CovSubscriptionKey, &TimedChange)> {
        self.changes
            .iter()
            .filter_map(|(key, _, changes)| Some((key, changes.last()?)))
    }
}

/// A lodged claim: what [`TimedHistories::settle`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimId(u64);

/// How a lodged claim's notification ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Delivered: transmitted when unconfirmed, acknowledged when confirmed.
    /// The claim is committed.
    Delivered,
    /// Not delivered, for whatever reason. The claim is given back.
    Returned,
}

/// A claim the histories keep until it is settled or its deadline passes.
#[derive(Debug)]
pub(super) struct Lodged {
    id: ClaimId,
    deadline: Instant,
    claim: TimedClaim,
}

impl TimedHistories {
    /// The notification carrying these changes and values was delivered:
    /// retire them. A part whose change continues in a later notification
    /// delivers nothing of that change by itself (#1090); the change is in
    /// delivery from then on (#1163).
    pub fn commit(&mut self, claim: TimedClaim) {
        for (key, incarnation, changes) in claim.changes {
            match changes.iter().rev().find(|change| !change.continues) {
                Some(last) => self.commit_through(&key, incarnation, last.seq),
                None => {
                    if let Some(part) = changes.last() {
                        self.deliver_by_value(&key, incarnation, part.seq);
                    }
                }
            }
        }
    }

    /// The notification carrying these changes and values was not delivered:
    /// return the changes to their references, and owe, since `now`, each
    /// untimestamped reference of a claim that is [`TimedClaim::owing`]. One
    /// that was owed already stays owed from when it first was.
    pub fn give_back(&mut self, claim: TimedClaim, now: Instant) {
        for (key, incarnation, changes) in claim.changes {
            self.requeue(&key, incarnation, changes, claim.evict);
        }
        for (key, generation, owed_since) in claim.untimed {
            if let Some(since) = owed_since.or(claim.owing.then_some(now)) {
                self.owe(&key, generation, since);
            }
        }
    }

    /// This part goes out as a confirmed report's first, and the report's
    /// later parts wait in the queue for its Ack. A change of which it
    /// carries only some values is in delivery from now on, so that the bound
    /// keeps the rest of it (#1163).
    pub fn going_out(&mut self, claim: &TimedClaim) {
        for (key, incarnation, changes) in &claim.changes {
            if let Some(part) = changes.last().filter(|part| part.continues) {
                self.deliver_by_value(key, *incarnation, part.seq);
            }
        }
    }

    /// Give up the untimestamped references left in `claim` without owing
    /// them, counting each
    /// ([`TimedCounts::untimed_oversized`](super::TimedCounts::untimed_oversized)):
    /// their values fit no notification, so a later attempt would only fail
    /// again. They are evaluated again at their next fanout (#1066).
    pub fn forgo_untimed(&mut self, claim: &mut TimedClaim) {
        self.counts.untimed_oversized += core::mem::take(&mut claim.untimed).len() as u64;
    }

    /// Count the changes [`TimedClaim::split_values`] found to hold a value
    /// too large for any notification, one per entry of `lost`; each context
    /// reports its first (#1039).
    pub fn lost_values(&mut self, lost: &[CovSubscriptionKey]) {
        for key in lost {
            self.dropped(key, 1, DropReason::TooLarge);
        }
    }

    /// Keep `claim` until [`Self::settle`] names it, or until `deadline`:
    /// from then on [`Self::take_due`] gives it back, as if it had been
    /// returned. For a holder that may be lost before it settles, such as a
    /// task a host can abort. Pick a deadline a live holder always settles
    /// before; one that settles later finds its claim gone.
    pub fn lodge(&mut self, claim: TimedClaim, deadline: Instant) -> ClaimId {
        let id = ClaimId(self.next_claim);
        self.next_claim += 1;
        self.lodged.push(Lodged {
            id,
            deadline,
            claim,
        });
        // The new deadline may be the earliest.
        self.moved = true;
        id
    }

    /// Settle the lodged claim `id` as `outcome` says. `false`, and nothing
    /// changes, when the claim is not lodged any more: it was settled, or
    /// its deadline passed and it was given back.
    pub fn settle(&mut self, id: ClaimId, outcome: Settlement, now: Instant) -> bool {
        let Some(at) = self.lodged.iter().position(|lodged| lodged.id == id) else {
            return false;
        };
        let claim = self.lodged.remove(at).claim;
        match outcome {
            Settlement::Delivered => self.commit(claim),
            Settlement::Returned => self.give_back(claim, now),
        }
        true
    }

    /// Give back every lodged claim whose deadline has passed at `now`,
    /// oldest first.
    pub(super) fn requeue_unsettled(&mut self, now: Instant) {
        let mut at = 0;
        while at < self.lodged.len() {
            if self.lodged[at].deadline <= now {
                let claim = self.lodged.remove(at).claim;
                self.give_back(claim, now);
            } else {
                at += 1;
            }
        }
    }

    /// Earliest deadline of a lodged claim.
    pub(super) fn next_unsettled(&self) -> Option<Instant> {
        self.lodged.iter().map(|lodged| lodged.deadline).min()
    }
}

/// How the notification of one value of an oversized change would go out
/// (#1090).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueFit {
    /// Within the subscriber's limit.
    Fits,
    /// Over the limit even with that value alone.
    TooLarge,
    /// With nothing in it: the context reports that coordinate untimestamped.
    Empty,
}
