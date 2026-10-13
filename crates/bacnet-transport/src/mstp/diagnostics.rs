//! Counts at the host transport boundary plus the master core's own
//! counters; never a wire capture or timing source.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_mstp_core::MstpCounters as CoreCounters;

use super::queues::SharedQueues;
use super::totals::MstpCounters;

/// Owned, redacted MS/TP counts. All fields start at zero, and there is no
/// reset; use before/after snapshots for a run. Every count saturates at
/// `u64::MAX`; a saturated counter cannot provide an exact delta.
///
/// The transport-boundary fields are each loaded atomically with relaxed
/// ordering, and [`Self::mac`] is the core's counters added up as the
/// transport's task last published them: it takes them from the core after
/// each event it handles, and a send takes its queue's. The collection is
/// **not a globally coherent snapshot**, nor does it synchronize
/// application state.
///
/// Transmit counts are of frames the core saw complete on the wire. No
/// timestamps, payloads, identities, paths, or per-peer state are retained.
///
/// The transport no longer decodes frames itself: the core's stream decoder
/// does, so received frames are counted by the core in [`Self::mac`]
/// (`frames_received`, `npdus_delivered`, `parser_errors`, `frame_aborts`)
/// rather than by frame type here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpDiagnosticsSnapshot {
    /// DataExpectingReply frames sent.
    pub der_tx: u64,
    /// Application replies to a received DataExpectingReply, handed back
    /// through the request's one-use reply channel in time and sent as the
    /// answer (DataNotExpectingReply).
    pub dner_tx_direct: u64,
    /// DataNotExpectingReply frames sent from the token queue, including
    /// broadcasts and replies that missed their reply window. These are not
    /// necessarily deferred replies; the transport cannot correlate them.
    pub dner_tx_queued: u64,
    /// ReplyPostponed frames sent.
    pub reply_postponed_tx: u64,
    /// Waits for a reply that ended in T_reply_timeout (Clause 9.5.6.4
    /// ReplyTimeout), rather than with a reply or an invalid frame.
    pub wait_for_reply_timeouts: u64,
    /// Outbound NPDU admissions rejected because the token queue was full.
    pub outbound_queue_full: u64,
    /// Oversize outbound NPDUs or direct application replies rejected at the
    /// transport boundary. Invalid MACs and not-started sends are not included.
    pub outbound_oversize: u64,
    /// Ingress NPDUs dropped because the receive channel was full.
    pub ingress_full: u64,
    /// Ingress NPDUs dropped because the receive channel was closed.
    pub ingress_closed: u64,
    /// Serial reads that failed: a read that completed with an error, or
    /// the port's end of file, three reads in a row that returned no octets
    /// (see [`SerialPort::read`](super::SerialPort::read)). The transport's
    /// task ends at the first one.
    pub serial_read_errors: u64,
    /// Serial write or drain calls that completed with an error. The frame
    /// is reported to the core as aborted.
    pub serial_write_errors: u64,
    /// Frames sent on a port with [`Echo::Full`](super::Echo::Full) during
    /// whose echo hunt octets other than the echo were read and passed on to
    /// the core, outside the probe after a grace with no echo: a stray or
    /// pad octet, line noise, a collision, an echo missing its first octets,
    /// or a peer that answered before the echo was seen. Counted once per
    /// frame.
    pub echo_mismatches: u64,
    /// Frames sent on a port with [`Echo::Full`](super::Echo::Full) after
    /// which nothing at all was read within the echo grace (a grace in which
    /// other octets passed on counts in `echo_mismatches` instead). A port
    /// declared with an echo that does not echo shows here.
    pub echo_missing: u64,
    /// Fragments of an echo dropped without the whole frame having
    /// matched: they had waited longer than T_frame_abort for the octet that
    /// would release them (a receiver would have abandoned them anyway), or
    /// this station confirmed its next start while they were held, as the
    /// previous echo's continuation.
    pub echo_fragments_dropped: u64,
    /// Application answers that came after the core had finished their
    /// request (it had sent Reply Postponed, or given up), queued as
    /// ordinary NPDUs to the requester, to go out with the token.
    pub late_answers_queued: u64,
    /// Reply channels of finished requests given up because more than the
    /// transport keeps (eight) were waiting: the oldest goes.
    pub late_answers_dropped: u64,
    /// Reply channels of finished requests given up at their request's
    /// arrival plus the queue's maximum age, answered after it or not at
    /// all.
    pub late_answers_expired: u64,
    /// The master core's counters, with its queues', added up since the
    /// transport started ([`MstpCounters`]).
    pub mac: MstpCounters,
}

/// Cloneable counts-only handle obtained from [`super::MstpTransport::diagnostics`]
/// before moving the transport into a router or endpoint.
///
/// Clones share the same bounded counters in both execution modes. They keep only
/// the counters alive, not the transport, serial resource, core, or runtime. Counts
/// remain readable after stop/drop. Drop/abort request cancellation, not awaited
/// shutdown, so an in-flight task may still update counts until it exits. There is
/// no reset or persistence across newly constructed transports.
#[derive(Debug, Clone)]
pub struct MstpDiagnostics {
    pub(super) counters: Arc<Counters>,
}

impl MstpDiagnostics {
    pub(super) fn new() -> Self {
        Self {
            counters: Arc::new(Counters::default()),
        }
    }

    /// Load an owned snapshot without allocating.
    /// See [`MstpDiagnosticsSnapshot`] for coherence and host-vs-wire limitations.
    pub fn snapshot(&self) -> MstpDiagnosticsSnapshot {
        self.counters.snapshot()
    }
}

// Keep the storage and loads together; no event log or per-frame allocation.
macro_rules! counters {
    ($($field:ident),+ $(,)?) => {
        #[derive(Debug, Default)]
        pub(super) struct Counters {
            $(pub(super) $field: AtomicU64,)+
            /// The core's counters as last published by the task.
            mac: Mutex<MstpCounters>,
        }

        impl Counters {
            fn snapshot(&self) -> MstpDiagnosticsSnapshot {
                MstpDiagnosticsSnapshot {
                    $($field: self.$field.load(Ordering::Relaxed),)+
                    mac: *self.mac.lock().unwrap_or_else(PoisonError::into_inner),
                }
            }
        }

        #[cfg(test)]
        #[test]
        fn every_counter_starts_at_zero_and_saturates() {
            let handle = MstpDiagnostics::new();
            assert_eq!(handle.snapshot(), MstpDiagnosticsSnapshot::default());
            $(handle.counters.$field.store(u64::MAX - 1, Ordering::Relaxed);
              increment(&handle.counters.$field);
              increment(&handle.counters.$field);
              assert_eq!(handle.snapshot().$field, u64::MAX);)+
        }
    };
}

counters!(
    der_tx,
    dner_tx_direct,
    dner_tx_queued,
    reply_postponed_tx,
    wait_for_reply_timeouts,
    outbound_queue_full,
    outbound_oversize,
    ingress_full,
    ingress_closed,
    serial_read_errors,
    serial_write_errors,
    echo_mismatches,
    echo_missing,
    echo_fragments_dropped,
    late_answers_queued,
    late_answers_dropped,
    late_answers_expired,
);

pub(super) fn increment(counter: &AtomicU64) {
    // Statistics only: never used for decisions or state publication.
    #[allow(deprecated, reason = "try_update needs Rust 1.95; the MSRV is 1.93")]
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

impl Counters {
    /// Add the core's counters to the totals: `take` takes them, its
    /// queues' with them ([`MasterCore::take_counters`]).
    ///
    /// `take` runs while the totals are locked, as [`Self::publish_queues`]
    /// takes the queues' counters, so takes reach the totals in the order
    /// they were made and the queue depth is always the newest. The lock
    /// order is always the totals, then the queues, and neither is held
    /// across an await.
    ///
    /// [`MasterCore::take_counters`]: bacnet_mstp_core::MasterCore::take_counters
    pub(super) fn publish(&self, take: impl FnOnce() -> CoreCounters) {
        let mut totals = self.mac.lock().unwrap_or_else(PoisonError::into_inner);
        totals.add(take());
    }

    /// Add the queues' counters to the totals after a send changed them.
    pub(super) fn publish_queues(&self, queues: &SharedQueues) {
        let mut totals = self.mac.lock().unwrap_or_else(PoisonError::into_inner);
        let (unicast, broadcast) = queues.take_counters();
        totals.unicast.add(unicast);
        totals.broadcast.add(broadcast);
    }

    pub(super) fn delivery<T>(
        &self,
        result: &Result<(), tokio::sync::mpsc::error::TrySendError<T>>,
    ) {
        use tokio::sync::mpsc::error::TrySendError;
        match result {
            Err(TrySendError::Full(_)) => increment(&self.ingress_full),
            Err(TrySendError::Closed(_)) => increment(&self.ingress_closed),
            Ok(()) => {}
        }
    }
}

#[cfg(test)]
#[test]
fn cloned_handles_share_concurrent_saturating_updates() {
    let handle = MstpDiagnostics::new();
    handle
        .counters
        .serial_read_errors
        .store(u64::MAX - 2, Ordering::Relaxed);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let clone = handle.clone();
            scope.spawn(move || {
                for _ in 0..100 {
                    increment(&clone.counters.der_tx);
                    increment(&clone.counters.serial_read_errors);
                }
            });
        }
    });
    assert_eq!(handle.snapshot().der_tx, 400);
    assert_eq!(handle.snapshot().serial_read_errors, u64::MAX);
}

/// Each publish adds what was counted since the one before: a send's
/// enqueue is taken once, by whichever publish comes first, and the core's
/// counts add up.
#[cfg(test)]
#[test]
fn published_core_counters_add_up_in_snapshots() {
    use bacnet_mstp_core::{Duration, Instant, QueueKind, QueueStorage};

    let handle = MstpDiagnostics::new();
    let queues = SharedQueues::new(false);
    let mut core_queues = queues.for_core();
    let mut mac = CoreCounters::default();
    mac.frames_received = 3;
    handle.counters.publish(|| mac);
    let age = Duration::from_secs(1);
    queues.enqueue(5, &[1, 0], Instant::ZERO, age).unwrap();
    handle.counters.publish_queues(&queues);
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.mac.frames_received, 3);
    assert_eq!(
        (snapshot.mac.unicast.enqueued, snapshot.mac.unicast.depth),
        (1, 1)
    );

    // The core's next take, its queues' with it, adds to the totals.
    queues.enqueue(5, &[1, 0], Instant::ZERO, age).unwrap();
    handle.counters.publish(|| {
        let mut taken = mac;
        taken.unicast = core_queues.take_counters(QueueKind::Unicast);
        taken
    });
    let totals = handle.snapshot().mac;
    assert_eq!(totals.frames_received, 6);
    assert_eq!(
        (
            totals.unicast.enqueued,
            totals.unicast.depth,
            totals.unicast.high_water
        ),
        (2, 2, 2)
    );
}
