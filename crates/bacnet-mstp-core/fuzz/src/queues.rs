//! The core's transmit queues, wrapped so the target sees every call the
//! core makes on them.
//!
//! The wrapper checks that neither queue ever holds more than the capacity
//! the target gave it, that an accepted NPDU adds exactly one item, and that
//! nothing longer than the station sends gets in or comes out. It also notes
//! the NPDU the queue handed out last, which the checker compares with the
//! data frame the core then sends.
//!
//! A dequeue lends the NPDU from the queue, which stays borrowed while it is
//! lent, so the wrapper copies it into a buffer of its own and lends it from
//! there: that frees the queue for the depth check that follows.

use std::cell::RefCell;
use std::rc::Rc;

use bacnet_mstp_codec::BROADCAST_MAC;
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    DequeuedNpdu, Instant, QueueCounters, QueueItemInfo, QueueKind, QueueRefusal, QueueStorage,
};

/// What the wrapper saw, shared with the checker.
#[derive(Debug, Default)]
pub struct QueueLog {
    /// The destination and NPDU of the item the queue handed out last.
    pub last_dequeued: Option<(u8, Vec<u8>)>,
    /// A broken rule, if any.
    pub broken: Option<String>,
}

/// Queues `Q`, with the capacities the target gave them.
pub struct Logged<Q> {
    inner: Q,
    /// Unicast, then broadcast.
    capacity: [usize; 2],
    max_npdu: usize,
    log: Rc<RefCell<QueueLog>>,
    /// The NPDU the last dequeue lent, copied out of the queue.
    lent: Vec<u8>,
}

impl<Q: QueueStorage> Logged<Q> {
    /// Wrap `inner`, whose queues hold `unicast` and `broadcast` items, for a
    /// station that sends NPDUs of up to `max_npdu` octets.
    pub fn new(
        inner: Q,
        unicast: usize,
        broadcast: usize,
        max_npdu: usize,
        log: Rc<RefCell<QueueLog>>,
    ) -> Self {
        Self {
            inner,
            capacity: [unicast, broadcast],
            max_npdu,
            log,
            lent: Vec::new(),
        }
    }

    fn broken(&self, what: String) {
        self.log.borrow_mut().broken.get_or_insert(what);
    }

    /// Check both depths against their capacities.
    fn check_depths(&self, call: &str) {
        for (kind, capacity) in [QueueKind::Unicast, QueueKind::Broadcast]
            .into_iter()
            .zip(self.capacity)
        {
            let depth = self.inner.depth(kind);
            if depth > capacity {
                self.broken(format!(
                    "the {kind:?} queue holds {depth} items after {call}, more than its {capacity} slots"
                ));
            }
        }
    }
}

fn index(kind: QueueKind) -> usize {
    match kind {
        QueueKind::Unicast => 0,
        QueueKind::Broadcast => 1,
    }
}

impl<Q: QueueStorage> QueueStorage for Logged<Q> {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.inner.set_starvation_limit(limit);
    }

    fn max_npdu(&self) -> usize {
        self.inner.max_npdu()
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        if payload.len() > self.max_npdu {
            self.broken(format!(
                "the core queued an NPDU of {} octets, longer than the {} the station sends",
                payload.len(),
                self.max_npdu
            ));
        }
        let kind = QueueKind::from_destination(destination);
        let before = self.inner.depth(kind);
        let result = self
            .inner
            .enqueue_npdu(destination, payload, priority, now, expires_at);
        let after = self.inner.depth(kind);
        if result.is_ok() && after != before + 1 {
            self.broken(format!(
                "an accepted NPDU took the {kind:?} queue from {before} items to {after}"
            ));
        }
        if result == Err(QueueRefusal::Full) && before < self.capacity[index(kind)] {
            self.broken(format!(
                "the {kind:?} queue refused an NPDU as full with {before} of {} slots used",
                self.capacity[index(kind)]
            ));
        }
        self.check_depths("an enqueue");
        result
    }

    fn expire(&mut self, now: Instant) -> usize {
        let removed = self.inner.expire(now);
        self.check_depths("an expiry");
        removed
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        self.inner.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        self.inner.peek_kind(kind, now)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        // The item, its NPDU copied into `lent` so the queue is free again.
        let result = self.inner.dequeue_kind(kind, now, max_len).map(|item| {
            item.map(|item| {
                self.lent.clear();
                self.lent.extend_from_slice(item.npdu);
                DequeuedNpdu::new(
                    item.destination,
                    item.kind,
                    item.priority,
                    item.sequence,
                    &[],
                )
            })
        });
        if let Ok(Some(item)) = result {
            let len = self.lent.len();
            if len > max_len || len > self.max_npdu {
                self.broken(format!(
                    "the queue handed out {len} octets, for a caller that takes {max_len} and a station that sends {}",
                    self.max_npdu
                ));
            } else if item.kind != kind
                || (item.destination == BROADCAST_MAC) != (kind == QueueKind::Broadcast)
            {
                self.broken(format!(
                    "asked for a {kind:?} item, the queue handed out a {:?} one to {}",
                    item.kind, item.destination
                ));
            } else {
                self.log.borrow_mut().last_dequeued = Some((item.destination, self.lent.clone()));
            }
        }
        self.check_depths("a dequeue");
        result.map(|item| {
            item.map(|item| {
                DequeuedNpdu::new(
                    item.destination,
                    item.kind,
                    item.priority,
                    item.sequence,
                    &self.lent,
                )
            })
        })
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.inner.depth(kind)
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.inner.counters(kind)
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.inner.take_counters(kind)
    }
}
