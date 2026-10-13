//! The transmit queues the core sends from, shared with the transport's
//! send calls, and the clock that maps Tokio time onto the core's.
//!
//! The core lives in the transport's task, but `send_unicast` and
//! `send_broadcast` run on the caller's task and report a full queue or an
//! oversized NPDU at once, as they always have. So the queues sit behind a
//! [`std::sync::Mutex`], which the core locks through [`QueueStorage`] for
//! each queue operation and a send locks for one enqueue. Neither side holds
//! it across an await.
//!
//! The core's handle, [`CoreQueues`], has one buffer of its own: a dequeue
//! lends the NPDU, which can't stay borrowed from inside the lock, so it is
//! copied there under the lock and lent from there.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_mstp_codec::{MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA};

use bacnet_mstp_core::{
    AllocQueues, DequeuedNpdu, Duration as CoreDuration, Instant as CoreInstant, NetworkPriority,
    QueueCounters, QueueItemInfo, QueueKind, QueueRefusal, QueueStorage,
};

/// Room in each queue, unicast and broadcast: the host transport's limit
/// before the core, which had one queue of this depth for both.
pub(super) const QUEUE_CAPACITY: usize = 256;

/// The core's queues, and whether the task that sends from them still runs.
#[derive(Clone)]
pub(super) struct SharedQueues(Arc<Mutex<Shared>>);

struct Shared {
    /// Slots of a standard frame's 501 octets, or of an extended frame's
    /// 1497 with extended frames on: the slot size is what refuses an NPDU
    /// too long to send, at the send call.
    queues: Box<dyn QueueStorage + Send>,
    /// Cleared when the transport's task ends, so a send after that fails
    /// instead of queueing a frame nothing will send.
    open: bool,
}

/// Why a send could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnqueueError {
    /// The transport's task has ended.
    Closed,
    /// The queue refused it.
    Rejected(QueueRefusal),
}

impl SharedQueues {
    /// Queues whose slots hold the longest NPDU the port sends: 1497 octets
    /// with `extended_frames`, 501 without.
    pub(super) fn new(extended_frames: bool) -> Self {
        let queues: Box<dyn QueueStorage + Send> = if extended_frames {
            Box::new(AllocQueues::<MAX_EXTENDED_MPDU_DATA>::new(
                QUEUE_CAPACITY,
                QUEUE_CAPACITY,
            ))
        } else {
            Box::new(AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(
                QUEUE_CAPACITY,
                QUEUE_CAPACITY,
            ))
        };
        Self(Arc::new(Mutex::new(Shared { queues, open: true })))
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Refuse every later send.
    pub(super) fn close(&self) {
        self.lock().open = false;
    }

    /// Queue an NPDU from a send call, at `now` on the core's clock and
    /// expiring `max_age` later, as the core's own enqueue does.
    pub(super) fn enqueue(
        &self,
        destination: u8,
        npdu: &[u8],
        now: CoreInstant,
        max_age: CoreDuration,
    ) -> Result<(), EnqueueError> {
        self.enqueue_until(destination, npdu, now, now.saturating_add(max_age))
    }

    /// Queue an NPDU at `now`, expiring at `expires`.
    pub(super) fn enqueue_until(
        &self,
        destination: u8,
        npdu: &[u8],
        now: CoreInstant,
        expires: CoreInstant,
    ) -> Result<(), EnqueueError> {
        let mut shared = self.lock();
        if !shared.open {
            return Err(EnqueueError::Closed);
        }
        shared
            .queues
            .enqueue_npdu(destination, npdu, npdu_priority(npdu), now, Some(expires))
            .map_err(EnqueueError::Rejected)
    }

    /// The core's handle on these queues.
    pub(super) fn for_core(&self) -> CoreQueues {
        let longest = self.lock().queues.max_npdu();
        CoreQueues {
            shared: self.clone(),
            lent: vec![0; longest].into_boxed_slice(),
        }
    }

    /// Take the unicast and broadcast queue counters, which then start
    /// again (see [`QueueStorage::take_counters`]).
    pub(super) fn take_counters(&self) -> (QueueCounters, QueueCounters) {
        let mut shared = self.lock();
        (
            shared.queues.take_counters(QueueKind::Unicast),
            shared.queues.take_counters(QueueKind::Broadcast),
        )
    }
}

/// The network priority in an NPDU's control octet (Clause 6.2.2), so the
/// core's queues serve life-safety traffic first; normal for an NPDU too
/// short to carry one.
pub(super) fn npdu_priority(npdu: &[u8]) -> NetworkPriority {
    npdu.get(1)
        .and_then(|control| NetworkPriority::try_from(control & 0x03).ok())
        .unwrap_or(NetworkPriority::Normal)
}

/// The core's handle on the [`SharedQueues`], from
/// [`SharedQueues::for_core`]: the queues, and a buffer as long as their
/// slots, allocated once, that a dequeued NPDU is lent from.
pub(super) struct CoreQueues {
    shared: SharedQueues,
    lent: Box<[u8]>,
}

impl CoreQueues {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock()
    }
}

impl QueueStorage for CoreQueues {
    fn set_starvation_limit(&mut self, limit: u8) {
        self.lock().queues.set_starvation_limit(limit);
    }

    fn max_npdu(&self) -> usize {
        self.lock().queues.max_npdu()
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: CoreInstant,
        expires_at: Option<CoreInstant>,
    ) -> Result<(), QueueRefusal> {
        self.lock()
            .queues
            .enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: CoreInstant) -> usize {
        self.lock().queues.expire(now)
    }

    fn peek(&self, now: CoreInstant) -> Option<QueueItemInfo> {
        self.lock().queues.peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: CoreInstant) -> Option<QueueItemInfo> {
        self.lock().queues.peek_kind(kind, now)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: CoreInstant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        let mut shared = self.shared.lock();
        let Some(item) = shared.queues.dequeue_kind(kind, now, max_len)? else {
            return Ok(None);
        };
        let len = item.npdu.len();
        // The buffer is as long as the queues' slots and a queue lends at
        // most a slot, so the else branch never runs. If it did, the item,
        // already off its queue, would be lost without a count: a debug
        // build stops first.
        debug_assert!(
            len <= self.lent.len(),
            "a queue lent {len} octets, more than its {}-octet slots",
            self.lent.len()
        );
        let Some(lent) = self.lent.get_mut(..len) else {
            return Err(QueueRefusal::Oversize);
        };
        lent.copy_from_slice(item.npdu);
        let (destination, kind, priority, sequence) =
            (item.destination, item.kind, item.priority, item.sequence);
        drop(shared);
        Ok(Some(DequeuedNpdu::new(
            destination,
            kind,
            priority,
            sequence,
            &self.lent[..len],
        )))
    }

    fn depth(&self, kind: QueueKind) -> usize {
        self.lock().queues.depth(kind)
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        self.lock().queues.counters(kind)
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        self.lock().queues.take_counters(kind)
    }
}

/// Tokio time mapped onto the core's nanosecond timeline from a fixed
/// origin, the moment the transport started. Both the task and the send
/// calls use it, so queue times and core events share one clock.
#[derive(Debug, Clone, Copy)]
pub(super) struct CoreClock {
    origin: tokio::time::Instant,
}

impl CoreClock {
    pub(super) fn new() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }

    /// The core's time now.
    pub(super) fn now(&self) -> CoreInstant {
        self.at(tokio::time::Instant::now())
    }

    /// The core's time at `instant`, exactly (saturating some 584 years
    /// after the origin).
    pub(super) fn at(&self, instant: tokio::time::Instant) -> CoreInstant {
        let elapsed = instant.saturating_duration_since(self.origin);
        CoreInstant::ZERO + CoreDuration::saturating_from_core(elapsed)
    }

    /// The Tokio instant of a core time. Both ways are exact, so a timer set
    /// here never wakes before the core's deadline is due.
    pub(super) fn instant(&self, at: CoreInstant) -> tokio::time::Instant {
        let offset = std::time::Duration::from_nanos(at.as_nanos());
        self.origin
            .checked_add(offset)
            .unwrap_or_else(|| self.origin + std::time::Duration::from_secs(86_400 * 365))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_comes_from_the_npdu_control_octet() {
        assert_eq!(npdu_priority(&[1, 0x03]), NetworkPriority::LifeSafety);
        assert_eq!(
            npdu_priority(&[1, 0x06]),
            NetworkPriority::CriticalEquipment
        );
        assert_eq!(npdu_priority(&[1, 0x01]), NetworkPriority::Urgent);
        assert_eq!(npdu_priority(&[1, 0x04]), NetworkPriority::Normal);
        assert_eq!(npdu_priority(&[1]), NetworkPriority::Normal);
    }

    #[test]
    fn a_closed_queue_refuses_sends() {
        let queues = SharedQueues::new(false);
        let now = CoreInstant::ZERO;
        let age = CoreDuration::from_secs(1);
        assert_eq!(queues.enqueue(5, &[1, 0], now, age), Ok(()));
        queues.close();
        assert_eq!(
            queues.enqueue(5, &[1, 0], now, age),
            Err(EnqueueError::Closed)
        );
        assert_eq!(queues.take_counters().0.enqueued, 1);
    }

    #[test]
    fn the_core_handle_lends_what_the_sends_queued() {
        let queues = SharedQueues::new(true);
        let mut core = queues.for_core();
        let now = CoreInstant::ZERO;
        let age = CoreDuration::from_secs(1);
        // Normal priority (its control octet is 0), then an urgent one.
        let mut long: Vec<u8> = (0..MAX_EXTENDED_MPDU_DATA).map(|i| i as u8).collect();
        long[1] = 0;
        let urgent = [1, 0x01, 9];
        queues.enqueue(5, &long, now, age).unwrap();
        queues.enqueue(5, &urgent, now, age).unwrap();
        queues.enqueue(6, &[1, 0], now, age).unwrap();
        let first = core.dequeue(now, MAX_EXTENDED_MPDU_DATA).unwrap().unwrap();
        assert_eq!(
            (first.destination, first.priority, first.npdu),
            (5, NetworkPriority::Urgent, &urgent[..])
        );
        let second = core.dequeue(now, MAX_EXTENDED_MPDU_DATA).unwrap().unwrap();
        assert_eq!((second.destination, second.npdu), (5, &long[..]));
        // Longer than the core takes: dropped and counted.
        assert_eq!(core.dequeue(now, 1), Err(QueueRefusal::Oversize));
        assert_eq!(core.dequeue(now, MAX_EXTENDED_MPDU_DATA), Ok(None));
        let (unicast, _) = queues.take_counters();
        assert_eq!(
            (unicast.depth, unicast.dequeued, unicast.drops_oversize),
            (0, 3, 1)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_clock_round_trips_without_waking_early() {
        let clock = CoreClock::new();
        tokio::time::advance(std::time::Duration::from_nanos(2_500)).await;
        let now = clock.now();
        assert_eq!(now, CoreInstant::from_nanos(2_500));
        assert_eq!(clock.instant(now), tokio::time::Instant::now());
        let deadline = CoreInstant::from_nanos(10_001);
        assert_eq!(clock.at(clock.instant(deadline)), deadline);
    }
}
