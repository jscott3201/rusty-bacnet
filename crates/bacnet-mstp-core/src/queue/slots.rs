//! What the two queue backends share: the slots items wait in, a lane of
//! them per destination class, and the policy that picks the next item.
//!
//! An item is written straight into a free slot and stays there until it
//! is removed, and removing it only marks the slot free. So no item is ever
//! moved, whichever one leaves: no removal shifts the slots behind it, and
//! no item is copied out by value. The order items leave in comes from
//! their priority, wait and sequence number alone (see [`better`]), never
//! from the slot they happen to be in.

use core::cmp::Ordering;
use core::fmt;

use bacnet_time::Instant;

use crate::counters::QueueCounters;

use super::{NetworkPriority, QueueItemInfo, QueueKind};

/// One queue slot: an item, or room for one. A freed slot keeps the octets
/// of the item that left it, so [`LiveItems`] is how a queue shows its
/// slots.
pub(super) struct Slot<const N: usize> {
    /// Whether the slot holds an item. The other fields mean nothing
    /// without one.
    occupied: bool,
    destination: u8,
    priority: NetworkPriority,
    service_wait: u8,
    /// At most `N`, which [`Slot::LONGEST`] bounds, so it fits: a `u16`
    /// keeps an extended slot of 1,497 octets from growing by an alignment
    /// step for the occupancy flag.
    len: u16,
    enqueued_at: Instant,
    expires_at: Option<Instant>,
    sequence: u64,
    payload: [u8; N],
}

impl<const N: usize> Slot<N> {
    /// The longest slot a queue backend accepts, checked when it is built.
    pub(super) const LONGEST: usize = u16::MAX as usize;

    /// A free slot. It is all zero bits, so zeroed queues can live in
    /// `.bss`.
    pub(super) const EMPTY: Self = Self {
        occupied: false,
        destination: 0,
        priority: NetworkPriority::Normal,
        service_wait: 0,
        len: 0,
        enqueued_at: Instant::ZERO,
        expires_at: None,
        sequence: 0,
        payload: [0; N],
    };

    /// Whether the slot is free.
    pub(super) fn is_free(&self) -> bool {
        !self.occupied
    }

    /// Write an item into this free slot, in place. `payload` holds at
    /// most `N` octets, and `N` at most [`Self::LONGEST`].
    pub(super) fn fill(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
        sequence: u64,
    ) {
        self.occupied = true;
        self.destination = destination;
        self.priority = priority;
        self.service_wait = 0;
        // At most N, which is at most u16::MAX: nothing is cut off.
        self.len = payload.len() as u16;
        self.enqueued_at = now;
        self.expires_at = expires_at;
        self.sequence = sequence;
        self.payload[..payload.len()].copy_from_slice(payload);
    }

    /// The item's metadata.
    pub(super) fn info(&self, kind: QueueKind) -> QueueItemInfo {
        QueueItemInfo {
            destination: self.destination,
            kind,
            priority: self.priority,
            len: usize::from(self.len),
            enqueued_at: self.enqueued_at,
            expires_at: self.expires_at,
            sequence: self.sequence,
            service_wait: self.service_wait,
        }
    }

    /// The item's payload. The slot keeps it after the item is removed,
    /// until another item is written there.
    pub(super) fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.len)]
    }
}

/// The item a selection picked: its class, its slot and its metadata.
pub(super) type Selected = (QueueKind, usize, QueueItemInfo);

/// One destination class's queue: its slots, how many hold an item, and its
/// counters. `S` keeps the slots: an array in `FixedQueues`, a vector
/// that grows to its capacity in `AllocQueues`.
pub(super) struct Lane<S> {
    pub(super) slots: S,
    /// Slots that hold an item.
    pub(super) depth: usize,
    pub(super) counters: QueueCounters,
}

impl<S> Lane<S> {
    /// An empty lane over `slots`.
    pub(super) const fn new(slots: S, counters: QueueCounters) -> Self {
        Self {
            slots,
            depth: 0,
            counters,
        }
    }

    /// The live item in this lane that policy prefers to `best`, if one is,
    /// or else `best`.
    pub(super) fn select<const N: usize>(
        &self,
        kind: QueueKind,
        now: Instant,
        starvation_limit: u8,
        mut best: Option<Selected>,
    ) -> Option<Selected>
    where
        S: AsRef<[Slot<N>]>,
    {
        for (index, slot) in self.slots.as_ref().iter().enumerate() {
            if slot.is_free() || expired(slot.expires_at, now) {
                continue;
            }
            let info = slot.info(kind);
            if better(info, best.map(|selected| selected.2), starvation_limit) {
                best = Some((kind, index, info));
            }
        }
        best
    }

    /// Remove the items expired at `now`, counting each, and return how
    /// many there were.
    pub(super) fn expire<const N: usize>(&mut self, now: Instant) -> usize
    where
        S: AsMut<[Slot<N>]>,
    {
        let mut removed = 0;
        for index in 0..self.slots.as_mut().len() {
            let slot = &self.slots.as_mut()[index];
            if !slot.is_free() && expired(slot.expires_at, now) {
                self.remove(index);
                self.counters.record_drop_expired();
                removed += 1;
            }
        }
        removed
    }

    /// Remove the item in slot `index`, counting it as dequeued. The slot
    /// keeps its octets until another item is written there.
    pub(super) fn remove<const N: usize>(&mut self, index: usize)
    where
        S: AsMut<[Slot<N>]>,
    {
        self.slots.as_mut()[index].occupied = false;
        self.depth -= 1;
        self.counters.record_dequeue(self.depth);
    }

    /// One more service opportunity for every item waiting here.
    pub(super) fn bump_waits<const N: usize>(&mut self)
    where
        S: AsMut<[Slot<N>]>,
    {
        for slot in self.slots.as_mut() {
            if !slot.is_free() {
                slot.service_wait = slot.service_wait.saturating_add(1);
            }
        }
    }

    /// The first free slot, if there is one. (`AllocQueues` adds a slot
    /// instead when none is free.)
    pub(super) fn free_slot<const N: usize>(&mut self) -> Option<&mut Slot<N>>
    where
        S: AsMut<[Slot<N>]>,
    {
        self.slots.as_mut().iter_mut().find(|slot| slot.is_free())
    }

    /// Count an item just written into a free slot.
    pub(super) fn record_enqueue(&mut self) {
        self.depth += 1;
        self.counters.record_enqueue(self.depth);
    }

    /// Every item waiting here, with its octets, for the tests' model.
    #[cfg(test)]
    pub(super) fn snapshot<const N: usize>(
        &self,
        kind: QueueKind,
    ) -> impl Iterator<Item = (QueueItemInfo, &[u8])>
    where
        S: AsRef<[Slot<N>]>,
    {
        self.slots
            .as_ref()
            .iter()
            .filter(|slot| !slot.is_free())
            .map(move |slot| (slot.info(kind), slot.payload()))
    }
}

/// A lane's items, for a queue's `Debug`: free slots, and the octets they
/// still hold, are left out.
pub(super) struct LiveItems<'a, const N: usize>(pub(super) &'a [Slot<N>]);

impl<const N: usize> fmt::Debug for LiveItems<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().filter(|slot| !slot.is_free()).map(LiveItem))
            .finish()
    }
}

/// One item, as [`LiveItems`] shows it: its NPDU's octets, not the slot's.
struct LiveItem<'a, const N: usize>(&'a Slot<N>);

impl<const N: usize> fmt::Debug for LiveItem<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slot = self.0;
        f.debug_struct("Item")
            .field("destination", &slot.destination)
            .field("priority", &slot.priority)
            .field("enqueued_at", &slot.enqueued_at)
            .field("expires_at", &slot.expires_at)
            .field("sequence", &slot.sequence)
            .field("service_wait", &slot.service_wait)
            .field("npdu", &slot.payload())
            .finish()
    }
}

/// Whether an item with this expiry has expired at `now`. Expiry is
/// inclusive.
pub(super) fn expired(expires_at: Option<Instant>, now: Instant) -> bool {
    expires_at.is_some_and(|deadline| now >= deadline)
}

fn is_older(left: u64, right: u64) -> bool {
    // Sequence numbers are only compared while both entries are live.  This
    // half-range comparison remains FIFO-correct across a u64 wrap unless a
    // queue retains 2^63 live items, which is impossible for a bounded queue.
    left != right && left.wrapping_sub(right) > (u64::MAX / 2)
}

/// Whether policy serves `candidate` before `current`: a starved item
/// first, the oldest of them; otherwise the highest priority, the oldest
/// within it. Sequence numbers are unique among live items, so this orders
/// them totally, and a scan finds the same item in any slot order.
fn better(candidate: QueueItemInfo, current: Option<QueueItemInfo>, starvation_limit: u8) -> bool {
    let Some(current) = current else { return true };
    let candidate_starved = candidate.service_wait >= starvation_limit;
    let current_starved = current.service_wait >= starvation_limit;
    match (candidate_starved, current_starved) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => is_older(candidate.sequence, current.sequence),
        (false, false) => match candidate.priority.cmp(&current.priority) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => is_older(candidate.sequence, current.sequence),
        },
    }
}
