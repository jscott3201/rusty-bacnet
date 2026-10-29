//! Allocation-backed queue storage.

use alloc::vec::Vec;
use core::fmt;

use bacnet_time::Instant;

use crate::config::DEFAULT_STARVATION_LIMIT;
use crate::counters::QueueCounters;

use super::slots::{Lane, LiveItems, Selected, Slot, expired};
use super::{
    DequeuedNpdu, MAX_STANDARD_MPDU_DATA, NetworkPriority, QueueItemInfo, QueueKind, QueueRefusal,
    QueueStorage,
};

/// Allocation-backed queues. Capacity is reserved by the constructor, so
/// enqueue and dequeue do not grow a vector after initialization.
///
/// `N` is the longest NPDU a slot holds, as for
/// [`FixedQueues`](crate::FixedQueues): 501 octets by default, and
/// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA) for
/// a core with extended frames on. `N` can be at most 65,535, or the
/// constructors fail to compile:
///
/// ```
/// use bacnet_mstp_core::AllocQueues;
///
/// let longest = AllocQueues::<65_535>::new(1, 0);
/// ```
///
/// ```compile_fail
/// use bacnet_mstp_core::AllocQueues;
///
/// let too_long = AllocQueues::<65_536>::new(1, 0);
/// ```
///
/// Each item is written into a free slot and stays there until it leaves,
/// so no removal moves another item. A slot is first used only when every
/// one before it is in use, so the reserved memory is touched as the queue
/// fills.
///
/// The starvation limit comes from the core's configuration, as for
/// [`FixedQueues`](crate::FixedQueues).
pub struct AllocQueues<const N: usize = MAX_STANDARD_MPDU_DATA> {
    unicast: Lane<Vec<Slot<N>>>,
    broadcast: Lane<Vec<Slot<N>>>,
    unicast_capacity: usize,
    broadcast_capacity: usize,
    sequence: u64,
    starvation_limit: u8,
}

impl<const N: usize> AllocQueues<N> {
    /// Empty queues holding up to `unicast_capacity` and
    /// `broadcast_capacity` items, their room reserved now.
    #[must_use]
    pub fn new(unicast_capacity: usize, broadcast_capacity: usize) -> Self {
        const {
            assert!(
                N <= Slot::<N>::LONGEST,
                "a queue slot holds at most 65,535 octets"
            );
        };
        Self {
            unicast: Lane::new(
                Vec::with_capacity(unicast_capacity),
                QueueCounters::default(),
            ),
            broadcast: Lane::new(
                Vec::with_capacity(broadcast_capacity),
                QueueCounters::default(),
            ),
            unicast_capacity,
            broadcast_capacity,
            sequence: 0,
            starvation_limit: DEFAULT_STARVATION_LIMIT,
        }
    }

    fn select(&self, now: Instant) -> Option<Selected> {
        let limit = self.starvation_limit;
        let unicast = self.unicast.select(QueueKind::Unicast, now, limit, None);
        self.broadcast
            .select(QueueKind::Broadcast, now, limit, unicast)
    }

    fn select_kind(&self, kind: QueueKind, now: Instant) -> Option<Selected> {
        self.lane(kind)
            .select(kind, now, self.starvation_limit, None)
    }

    fn lane(&self, kind: QueueKind) -> &Lane<Vec<Slot<N>>> {
        match kind {
            QueueKind::Unicast => &self.unicast,
            QueueKind::Broadcast => &self.broadcast,
        }
    }

    /// Every item waiting, with its octets, for the tests' model.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Vec<(QueueItemInfo, Vec<u8>)> {
        self.unicast
            .snapshot(QueueKind::Unicast)
            .chain(self.broadcast.snapshot(QueueKind::Broadcast))
            .map(|(info, npdu)| (info, npdu.to_vec()))
            .collect()
    }

    /// Each lane's slots so far, and the room reserved for them.
    #[cfg(test)]
    pub(super) fn slot_room(&self) -> [(usize, usize); 2] {
        [
            (self.unicast.slots.len(), self.unicast.slots.capacity()),
            (self.broadcast.slots.len(), self.broadcast.slots.capacity()),
        ]
    }

    fn lane_mut(&mut self, kind: QueueKind) -> &mut Lane<Vec<Slot<N>>> {
        match kind {
            QueueKind::Unicast => &mut self.unicast,
            QueueKind::Broadcast => &mut self.broadcast,
        }
    }
}

/// The items waiting, oldest slot first, with their sequence numbers, the
/// capacities and the counters; free slots are left out.
impl<const N: usize> fmt::Debug for AllocQueues<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AllocQueues")
            .field("unicast", &LiveItems(&self.unicast.slots))
            .field("broadcast", &LiveItems(&self.broadcast.slots))
            .field("unicast_capacity", &self.unicast_capacity)
            .field("broadcast_capacity", &self.broadcast_capacity)
            .field("sequence", &self.sequence)
            .field("starvation_limit", &self.starvation_limit)
            .field("unicast_counters", &self.unicast.counters)
            .field("broadcast_counters", &self.broadcast.counters)
            .finish()
    }
}

impl<const N: usize> Default for AllocQueues<N> {
    fn default() -> Self {
        Self::new(256, 256)
    }
}

impl<const N: usize> QueueStorage for AllocQueues<N> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.starvation_limit = limit;
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
        let capacity = match kind {
            QueueKind::Unicast => self.unicast_capacity,
            QueueKind::Broadcast => self.broadcast_capacity,
        };
        let sequence = self.sequence;
        let lane = self.lane_mut(kind);
        if payload.len() > N {
            lane.counters.record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        if expired(expires_at, now) {
            lane.counters.record_drop_expired();
            return Err(QueueRefusal::Expired);
        }
        if lane.depth >= capacity {
            lane.counters.record_drop_full();
            return Err(QueueRefusal::Full);
        }
        // Fewer items than the capacity: a slot is free, or every slot so
        // far holds one and a new one fits the reservation.
        let index = match lane.slots.iter().position(Slot::is_free) {
            Some(index) => index,
            None => {
                lane.slots.push(Slot::EMPTY);
                lane.slots.len() - 1
            }
        };
        lane.slots[index].fill(destination, payload, priority, now, expires_at, sequence);
        lane.record_enqueue();
        self.sequence = self.sequence.wrapping_add(1);
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
        self.lane_mut(kind).remove(index);
        if info.len > max_len {
            self.lane_mut(kind).counters.record_drop_oversize();
            return Err(QueueRefusal::Oversize);
        }
        self.unicast.bump_waits();
        self.broadcast.bump_waits();
        Ok(Some(DequeuedNpdu {
            destination: info.destination,
            kind,
            priority: info.priority,
            sequence: info.sequence,
            npdu: self.lane(kind).slots[index].payload(),
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.lane(kind).depth
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.lane(kind).counters
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.lane_mut(kind).counters.take()
    }
}
