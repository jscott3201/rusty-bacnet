//! Fixed-capacity queue storage, with no allocator.

use core::fmt;

use bacnet_time::Instant;

use crate::config::DEFAULT_STARVATION_LIMIT;
use crate::counters::QueueCounters;

use super::slots::{expired, Lane, LiveItems, Selected, Slot};
use super::{
    DequeuedNpdu, NetworkPriority, QueueItemInfo, QueueKind, QueueRefusal, QueueStorage,
    MAX_STANDARD_MPDU_DATA,
};

const EMPTY_QUEUE_COUNTERS: QueueCounters = QueueCounters {
    depth: 0,
    high_water: 0,
    enqueued: 0,
    dequeued: 0,
    drops_full: 0,
    drops_oversize: 0,
    drops_expired: 0,
};

/// Fixed-capacity, independently bounded unicast and broadcast queues.
///
/// `U` and `B` are the unicast and broadcast capacities, and `N` the longest
/// NPDU a slot holds: a standard frame's 501 octets by default, and
/// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA) for
/// a core with extended frames on. `N` can be at most 65,535, or the
/// constructor fails to compile:
///
/// ```
/// use bacnet_mstp_core::FixedQueues;
///
/// static LONGEST: FixedQueues<1, 0, 65_535> = FixedQueues::new();
/// ```
///
/// ```compile_fail
/// use bacnet_mstp_core::FixedQueues;
///
/// static TOO_LONG: FixedQueues<1, 0, 65_536> = FixedQueues::new();
/// ```
///
/// Each item is written into a free slot and stays there until it leaves,
/// so no removal moves another item.
///
/// [`Self::new`] is a `const fn` whose result is all zero bits, so the
/// queues can live in a zero-initialized static (`.bss`) and be lent to the
/// core as `&mut` (see the crate docs).
///
/// The starvation limit comes from the core's configuration
/// ([`MstpCoreConfig::starvation_limit`](crate::MstpCoreConfig::starvation_limit)),
/// which [`MasterCore::new`](crate::MasterCore::new) hands over; until
/// then the queues use that field's default.
pub struct FixedQueues<const U: usize, const B: usize, const N: usize = MAX_STANDARD_MPDU_DATA> {
    unicast: Lane<[Slot<N>; U]>,
    broadcast: Lane<[Slot<N>; B]>,
    sequence: u64,
    /// `None` for [`DEFAULT_STARVATION_LIMIT`], so that `new` is all zeros.
    starvation_limit: Option<u8>,
}

impl<const U: usize, const B: usize, const N: usize> FixedQueues<U, B, N> {
    /// Empty queues.
    #[must_use]
    pub const fn new() -> Self {
        const {
            assert!(
                N <= Slot::<N>::LONGEST,
                "a queue slot holds at most 65,535 octets"
            );
        };
        Self {
            unicast: Lane::new([Slot::EMPTY; U], EMPTY_QUEUE_COUNTERS),
            broadcast: Lane::new([Slot::EMPTY; B], EMPTY_QUEUE_COUNTERS),
            sequence: 0,
            starvation_limit: None,
        }
    }

    fn starvation_limit(&self) -> u8 {
        self.starvation_limit.unwrap_or(DEFAULT_STARVATION_LIMIT)
    }

    fn select(&self, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit();
        let unicast = self.unicast.select(QueueKind::Unicast, now, limit, None);
        self.broadcast
            .select(QueueKind::Broadcast, now, limit, unicast)
    }

    fn select_kind(&self, kind: QueueKind, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit();
        match kind {
            QueueKind::Unicast => self.unicast.select(kind, now, limit, None),
            QueueKind::Broadcast => self.broadcast.select(kind, now, limit, None),
        }
    }

    fn slot(&self, kind: QueueKind, index: usize) -> &Slot<N> {
        match kind {
            QueueKind::Unicast => &self.unicast.slots[index],
            QueueKind::Broadcast => &self.broadcast.slots[index],
        }
    }

    /// Every item waiting, with its octets, for the tests' model.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> std::vec::Vec<(QueueItemInfo, std::vec::Vec<u8>)> {
        self.unicast
            .snapshot(QueueKind::Unicast)
            .chain(self.broadcast.snapshot(QueueKind::Broadcast))
            .map(|(info, npdu)| (info, npdu.to_vec()))
            .collect()
    }

    fn counters_mut(&mut self, kind: QueueKind) -> &mut QueueCounters {
        match kind {
            QueueKind::Unicast => &mut self.unicast.counters,
            QueueKind::Broadcast => &mut self.broadcast.counters,
        }
    }
}

/// The items waiting, oldest slot first, with their sequence numbers, and
/// the counters; free slots are left out.
impl<const U: usize, const B: usize, const N: usize> fmt::Debug for FixedQueues<U, B, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedQueues")
            .field("unicast", &LiveItems(&self.unicast.slots))
            .field("broadcast", &LiveItems(&self.broadcast.slots))
            .field("sequence", &self.sequence)
            .field("starvation_limit", &self.starvation_limit())
            .field("unicast_counters", &self.unicast.counters)
            .field("broadcast_counters", &self.broadcast.counters)
            .finish()
    }
}

impl<const U: usize, const B: usize, const N: usize> Default for FixedQueues<U, B, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const U: usize, const B: usize, const N: usize> QueueStorage for FixedQueues<U, B, N> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.starvation_limit = Some(limit);
    }

    fn max_npdu(&self) -> usize {
        N
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        let kind = QueueKind::from_destination(destination);
        if payload.len() > N {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        if expired(expires_at, now) {
            self.counters_mut(kind).record_drop_expired();
            return Err(QueueRefusal::Expired);
        }
        // The number is taken before the room is checked, so a full queue
        // uses one up too, as it always has here.
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let free = match kind {
            QueueKind::Unicast => self.unicast.free_slot(),
            QueueKind::Broadcast => self.broadcast.free_slot(),
        };
        let Some(slot) = free else {
            self.counters_mut(kind).record_drop_full();
            return Err(QueueRefusal::Full);
        };
        slot.fill(destination, payload, priority, now, expires_at, sequence);
        match kind {
            QueueKind::Unicast => self.unicast.record_enqueue(),
            QueueKind::Broadcast => self.broadcast.record_enqueue(),
        }
        Ok(())
    }

    fn expire(&mut self, now: Instant) -> usize {
        self.unicast.expire(now) + self.broadcast.expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.select(now).map(|selected| selected.2)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.select_kind(kind, now).map(|selected| selected.2)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        self.expire(now);
        let Some((_, index, info)) = self.select_kind(kind, now) else {
            return Ok(None);
        };
        match kind {
            QueueKind::Unicast => self.unicast.remove(index),
            QueueKind::Broadcast => self.broadcast.remove(index),
        }
        if info.len > max_len {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        self.unicast.bump_waits();
        self.broadcast.bump_waits();
        Ok(Some(DequeuedNpdu {
            destination: info.destination,
            kind,
            priority: info.priority,
            sequence: info.sequence,
            npdu: self.slot(kind, index).payload(),
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        match kind {
            QueueKind::Unicast => self.unicast.depth,
            QueueKind::Broadcast => self.broadcast.depth,
        }
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        match kind {
            QueueKind::Unicast => self.unicast.counters,
            QueueKind::Broadcast => self.broadcast.counters,
        }
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.counters_mut(kind).take()
    }
}
