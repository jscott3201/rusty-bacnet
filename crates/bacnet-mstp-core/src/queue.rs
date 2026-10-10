//! Bounded, observable MS/TP NPDU queues.
//!
//! The queue implementations deliberately own their payload bytes.  This is
//! important for the core API: a caller may hand an NPDU to the core from a
//! short-lived receive buffer, while the core must retain it until a token is
//! available.  The alloc implementation reserves all of its item storage in
//! its constructor; enqueue and dequeue do not grow a vector.

use core::cmp::Ordering;

use crate::counters::QueueCounters;

/// The largest NPDU accepted by the MS/TP core queue.
pub const MAX_NPDU_LEN: usize = 501;

/// Destination class used by the two independently bounded queues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueKind {
    /// A frame addressed to one MAC address.
    Unicast,
    /// A frame addressed to the MS/TP broadcast MAC (`0xff`).
    Broadcast,
}

impl QueueKind {
    /// The MS/TP broadcast MAC address.
    pub const BROADCAST_DESTINATION: u8 = 0xff;

    /// Classify a destination MAC.
    pub const fn from_destination(destination: u8) -> Self {
        if destination == Self::BROADCAST_DESTINATION {
            Self::Broadcast
        } else {
            Self::Unicast
        }
    }
}

/// BACnet network priorities, in increasing service order.
///
/// Clause 6.2.2 assigns the numeric values shown here.  Consequently
/// [`LifeSafety`](Self::LifeSafety) is selected before
/// [`CriticalEquipment`](Self::CriticalEquipment), then [`Urgent`](Self::Urgent),
/// and finally [`Normal`](Self::Normal).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NetworkPriority {
    /// Normal traffic (`0`).
    Normal = 0,
    /// Urgent traffic (`1`).
    Urgent = 1,
    /// Critical-equipment traffic (`2`).
    CriticalEquipment = 2,
    /// Life-safety traffic (`3`).
    LifeSafety = 3,
}

impl NetworkPriority {
    /// Convert the wire priority value, rejecting reserved values.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Urgent),
            2 => Some(Self::CriticalEquipment),
            3 => Some(Self::LifeSafety),
            _ => None,
        }
    }

    /// Return the Clause 6.2.2 numeric value.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// The reason an NPDU was rejected by a queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueRejectReason {
    /// The destination queue has no free slot.
    Full,
    /// The payload is larger than the queue's owned slot.
    Oversize,
    /// The supplied expiry is at or before the enqueue time, or an item
    /// reached its expiry boundary while waiting.
    Expired,
    /// Reserved for policy layers that reject a request which has exceeded a
    /// starvation budget.  Queue selection itself never drops a starved item.
    Starved,
}

/// Metadata returned after copying a queued NPDU into caller-owned storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DequeuedNpdu {
    /// Destination MAC address.
    pub destination: u8,
    /// Whether this was taken from the unicast or broadcast queue.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Number of bytes copied into the caller's scratch buffer.
    pub len: usize,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
}

/// A non-mutating view of the next item selected by queue policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueItemInfo {
    /// Destination MAC address.
    pub destination: u8,
    /// Queue class.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Payload length.
    pub len: usize,
    /// Enqueue timestamp supplied by the caller.
    pub enqueued_at: u64,
    /// Absolute expiry timestamp, if one was supplied.
    pub expires_at: Option<u64>,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
    /// Number of service opportunities this item has waited.
    pub service_wait: u8,
}

const EMPTY_QUEUE_COUNTERS: QueueCounters = QueueCounters {
    depth: 0,
    high_water: 0,
    enqueued: 0,
    dequeued: 0,
    drops_full: 0,
    drops_oversize: 0,
    drops_expired: 0,
    drops_starved: 0,
};

/// A bounded storage backend for queued NPDUs.
pub trait QueueStorage {
    /// Apply the core's service-opportunity starvation limit.
    fn set_starvation_limit(&mut self, limit: u8);

    /// Add a payload to the destination queue.
    ///
    /// `expires_at` is an absolute timestamp in the same monotonically
    /// increasing unit as `now`.  Expiry is inclusive: an item is expired
    /// when `now >= expires_at`.
    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: u64,
        expires_at: Option<u64>,
    ) -> Result<(), QueueRejectReason>;

    /// Remove expired items at `now`, returning the number removed.
    fn expire(&mut self, now: u64) -> usize;

    /// Inspect the item that policy would service next.
    fn peek(&self, now: u64) -> Option<QueueItemInfo>;

    /// Inspect the next item in one destination class.
    ///
    /// The core uses this form when applying its configured
    /// broadcast/unicast service ratio.
    fn peek_kind(&self, kind: QueueKind, now: u64) -> Option<QueueItemInfo>;

    /// Copy the selected payload into `scratch` and remove it atomically.
    ///
    /// If `scratch` is too small, the item remains queued and
    /// [`QueueRejectReason::Oversize`] is returned.
    fn dequeue_into(
        &mut self,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        let Some(info) = self.peek(now) else {
            return Ok(None);
        };
        self.dequeue_kind_into(info.kind, now, scratch)
    }

    /// Copy and remove the next item from one destination class.
    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason>;

    /// Current depth for one destination class.
    fn depth(&self, kind: QueueKind) -> usize;

    /// Maximum depth and drop counters for one destination class.
    fn stats(&self, kind: QueueKind) -> QueueCounters;

    /// The total number of queued NPDUs.
    fn total_depth(&self) -> usize {
        self.depth(QueueKind::Unicast) + self.depth(QueueKind::Broadcast)
    }
}

#[derive(Clone, Copy, Debug)]
struct Item<const N: usize> {
    destination: u8,
    priority: NetworkPriority,
    enqueued_at: u64,
    expires_at: Option<u64>,
    sequence: u64,
    service_wait: u8,
    len: usize,
    payload: [u8; N],
}

impl<const N: usize> Item<N> {
    fn info(self, kind: QueueKind) -> QueueItemInfo {
        QueueItemInfo {
            destination: self.destination,
            kind,
            priority: self.priority,
            len: self.len,
            enqueued_at: self.enqueued_at,
            expires_at: self.expires_at,
            sequence: self.sequence,
            service_wait: self.service_wait,
        }
    }
}

fn expired(expires_at: Option<u64>, now: u64) -> bool {
    expires_at.is_some_and(|deadline| now >= deadline)
}

fn is_older(left: u64, right: u64) -> bool {
    // Sequence numbers are only compared while both entries are live.  This
    // half-range comparison remains FIFO-correct across a u64 wrap unless a
    // queue retains 2^63 live items, which is impossible for a bounded queue.
    left != right && left.wrapping_sub(right) > (u64::MAX / 2)
}

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

#[cfg(feature = "heapless")]
#[derive(Debug)]
/// Heapless, independently bounded unicast and broadcast queues.
pub struct HeaplessQueues<const U: usize, const B: usize, const N: usize = MAX_NPDU_LEN> {
    unicast: heapless::Vec<Item<N>, U>,
    broadcast: heapless::Vec<Item<N>, B>,
    sequence: u64,
    starvation_limit: u8,
    unicast_counters: QueueCounters,
    broadcast_counters: QueueCounters,
}

#[cfg(feature = "heapless")]
impl<const U: usize, const B: usize, const N: usize> HeaplessQueues<U, B, N> {
    /// Construct queues with the default starvation limit of 16 time units.
    pub const fn new() -> Self {
        Self::with_starvation_limit(16)
    }

    /// Construct queues with an explicit starvation limit.
    pub const fn with_starvation_limit(starvation_limit: u8) -> Self {
        Self {
            unicast: heapless::Vec::new(),
            broadcast: heapless::Vec::new(),
            sequence: 0,
            starvation_limit,
            unicast_counters: EMPTY_QUEUE_COUNTERS,
            broadcast_counters: EMPTY_QUEUE_COUNTERS,
        }
    }

    fn select(&self, now: u64) -> Option<(QueueKind, usize, QueueItemInfo)> {
        let mut selected: Option<(QueueKind, usize, QueueItemInfo)> = None;
        for (index, item) in self.unicast.iter().copied().enumerate() {
            let info = item.info(QueueKind::Unicast);
            if !expired(info.expires_at, now)
                && better(info, selected.map(|v| v.2), self.starvation_limit)
            {
                selected = Some((QueueKind::Unicast, index, info));
            }
        }
        for (index, item) in self.broadcast.iter().copied().enumerate() {
            let info = item.info(QueueKind::Broadcast);
            if !expired(info.expires_at, now)
                && better(info, selected.map(|v| v.2), self.starvation_limit)
            {
                selected = Some((QueueKind::Broadcast, index, info));
            }
        }
        selected
    }

    fn select_kind(&self, kind: QueueKind, now: u64) -> Option<(usize, QueueItemInfo)> {
        let mut selected: Option<(usize, QueueItemInfo)> = None;
        match kind {
            QueueKind::Unicast => {
                for (index, item) in self.unicast.iter().copied().enumerate() {
                    let info = item.info(kind);
                    if !expired(info.expires_at, now)
                        && better(info, selected.map(|v| v.1), self.starvation_limit)
                    {
                        selected = Some((index, info));
                    }
                }
            }
            QueueKind::Broadcast => {
                for (index, item) in self.broadcast.iter().copied().enumerate() {
                    let info = item.info(kind);
                    if !expired(info.expires_at, now)
                        && better(info, selected.map(|v| v.1), self.starvation_limit)
                    {
                        selected = Some((index, info));
                    }
                }
            }
        }
        selected
    }

    fn bump_waits(&mut self) {
        for item in &mut self.unicast {
            item.service_wait = item.service_wait.saturating_add(1);
        }
        for item in &mut self.broadcast {
            item.service_wait = item.service_wait.saturating_add(1);
        }
    }

    fn push_item(&mut self, kind: QueueKind, item: Item<N>) -> Result<(), QueueRejectReason> {
        match kind {
            QueueKind::Unicast => self.unicast.push(item).map_err(|_| QueueRejectReason::Full),
            QueueKind::Broadcast => self
                .broadcast
                .push(item)
                .map_err(|_| QueueRejectReason::Full),
        }
    }

    fn counters_mut(&mut self, kind: QueueKind) -> &mut QueueCounters {
        match kind {
            QueueKind::Unicast => &mut self.unicast_counters,
            QueueKind::Broadcast => &mut self.broadcast_counters,
        }
    }
}

#[cfg(feature = "heapless")]
impl<const U: usize, const B: usize, const N: usize> Default for HeaplessQueues<U, B, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "heapless")]
impl<const U: usize, const B: usize, const N: usize> QueueStorage for HeaplessQueues<U, B, N> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.starvation_limit = limit;
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: u64,
        expires_at: Option<u64>,
    ) -> Result<(), QueueRejectReason> {
        let kind = QueueKind::from_destination(destination);
        if payload.len() > N {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        if expired(expires_at, now) {
            self.counters_mut(kind).record_drop_expired();
            return Err(QueueRejectReason::Expired);
        }
        let mut owned = Item {
            destination,
            priority,
            enqueued_at: now,
            expires_at,
            sequence: self.sequence,
            service_wait: 0,
            len: payload.len(),
            payload: [0; N],
        };
        owned.payload[..payload.len()].copy_from_slice(payload);
        self.sequence = self.sequence.wrapping_add(1);
        self.push_item(kind, owned).map_err(|reason| {
            self.counters_mut(kind).record_drop_full();
            reason
        })?;
        let depth = match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        };
        self.counters_mut(kind).record_enqueue(depth);
        Ok(())
    }

    fn expire(&mut self, now: u64) -> usize {
        let mut removed = 0;
        let mut index = 0;
        while index < self.unicast.len() {
            if expired(self.unicast[index].expires_at, now) {
                self.unicast.remove(index);
                self.unicast_counters.record_drop_expired();
                self.unicast_counters.record_dequeue(self.unicast.len());
                removed += 1;
            } else {
                index += 1;
            }
        }
        let mut index = 0;
        while index < self.broadcast.len() {
            if expired(self.broadcast[index].expires_at, now) {
                self.broadcast.remove(index);
                self.broadcast_counters.record_drop_expired();
                self.broadcast_counters.record_dequeue(self.broadcast.len());
                removed += 1;
            } else {
                index += 1;
            }
        }
        removed
    }

    fn peek(&self, now: u64) -> Option<QueueItemInfo> {
        self.select(now).map(|selected| selected.2)
    }

    fn peek_kind(&self, kind: QueueKind, now: u64) -> Option<QueueItemInfo> {
        self.select_kind(kind, now).map(|selected| selected.1)
    }

    fn dequeue_kind_into(
        &mut self,
        kind: QueueKind,
        now: u64,
        scratch: &mut [u8],
    ) -> Result<Option<DequeuedNpdu>, QueueRejectReason> {
        self.expire(now);
        let Some((index, info)) = self.select_kind(kind, now) else {
            return Ok(None);
        };
        if scratch.len() < info.len {
            return Err(QueueRejectReason::Oversize);
        }
        let item = match kind {
            QueueKind::Unicast => self.unicast.remove(index),
            QueueKind::Broadcast => self.broadcast.remove(index),
        };
        let depth = self.depth(kind);
        self.counters_mut(kind).record_dequeue(depth);
        self.bump_waits();
        scratch[..item.len].copy_from_slice(&item.payload[..item.len]);
        Ok(Some(DequeuedNpdu {
            destination: item.destination,
            kind,
            priority: item.priority,
            len: item.len,
            sequence: item.sequence,
        }))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        }
    }

    fn stats(&self, kind: QueueKind) -> QueueCounters {
        match kind {
            QueueKind::Unicast => self.unicast_counters,
            QueueKind::Broadcast => self.broadcast_counters,
        }
    }
}

#[cfg(feature = "alloc")]
mod alloc;

#[cfg(feature = "alloc")]
pub use alloc::AllocQueues;
#[cfg(test)]
mod tests;
