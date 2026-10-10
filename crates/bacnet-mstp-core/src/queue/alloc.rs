//! Allocation-backed queue storage.

use alloc::vec::Vec;

use crate::counters::QueueCounters;

use super::{
    better, expired, DequeuedNpdu, Item, NetworkPriority, QueueItemInfo, QueueKind,
    QueueRejectReason, QueueStorage, MAX_NPDU_LEN,
};

/// Allocation-backed queues. Capacity is reserved by the constructor, so
/// enqueue and dequeue do not grow a vector after initialization.
#[derive(Debug)]
pub struct AllocQueues {
    unicast: Vec<Item<MAX_NPDU_LEN>>,
    broadcast: Vec<Item<MAX_NPDU_LEN>>,
    unicast_capacity: usize,
    broadcast_capacity: usize,
    sequence: u64,
    starvation_limit: u8,
    unicast_counters: QueueCounters,
    broadcast_counters: QueueCounters,
}

impl AllocQueues {
    /// Construct independently bounded queues.
    pub fn new(unicast_capacity: usize, broadcast_capacity: usize) -> Self {
        Self::with_starvation_limit(unicast_capacity, broadcast_capacity, 16)
    }

    /// Construct queues with an explicit service-opportunity starvation limit.
    pub fn with_starvation_limit(
        unicast_capacity: usize,
        broadcast_capacity: usize,
        starvation_limit: u8,
    ) -> Self {
        Self {
            unicast: Vec::with_capacity(unicast_capacity),
            broadcast: Vec::with_capacity(broadcast_capacity),
            unicast_capacity,
            broadcast_capacity,
            sequence: 0,
            starvation_limit,
            unicast_counters: QueueCounters::default(),
            broadcast_counters: QueueCounters::default(),
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

    fn counters_mut(&mut self, kind: QueueKind) -> &mut QueueCounters {
        match kind {
            QueueKind::Unicast => &mut self.unicast_counters,
            QueueKind::Broadcast => &mut self.broadcast_counters,
        }
    }
}

impl Default for AllocQueues {
    fn default() -> Self {
        Self::new(256, 256)
    }
}

impl QueueStorage for AllocQueues {
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
        if payload.len() > MAX_NPDU_LEN {
            self.counters_mut(kind).record_drop_oversize();
            return Err(QueueRejectReason::Oversize);
        }
        if expired(expires_at, now) {
            self.counters_mut(kind).record_drop_expired();
            return Err(QueueRejectReason::Expired);
        }
        let capacity = match kind {
            QueueKind::Unicast => self.unicast_capacity,
            QueueKind::Broadcast => self.broadcast_capacity,
        };
        let length = match kind {
            QueueKind::Unicast => self.unicast.len(),
            QueueKind::Broadcast => self.broadcast.len(),
        };
        if length >= capacity {
            self.counters_mut(kind).record_drop_full();
            return Err(QueueRejectReason::Full);
        }
        let mut owned = Item {
            destination,
            priority,
            enqueued_at: now,
            expires_at,
            sequence: self.sequence,
            service_wait: 0,
            len: payload.len(),
            payload: [0; MAX_NPDU_LEN],
        };
        owned.payload[..payload.len()].copy_from_slice(payload);
        self.sequence = self.sequence.wrapping_add(1);
        match kind {
            QueueKind::Unicast => self.unicast.push(owned),
            QueueKind::Broadcast => self.broadcast.push(owned),
        }
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
