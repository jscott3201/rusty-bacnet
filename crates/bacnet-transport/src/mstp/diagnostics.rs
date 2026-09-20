//! Counts at the host transport boundary, never a wire capture or timing source.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::mstp_frame::FrameType;

/// Owned, redacted MS/TP host counts. All fields start at zero and saturate at
/// `u64::MAX`. There is no reset; use before/after snapshots for a run. A saturated
/// counter cannot provide an exact delta.
///
/// Each field is loaded atomically with relaxed ordering. The collection is **not
/// a globally coherent snapshot**, nor does it synchronize application state.
/// Receive counts include all valid frames decoded by this host, even if addressed
/// elsewhere or ignored by the MAC. They do not imply NPDU/application delivery.
/// Transmit counts require successful encoding and `SerialPort::write` completion;
/// driver acceptance does not prove transmission, drain, peer receipt or timing.
/// Cancelled writes have no completed outcome to count, and failed writes may have
/// emitted partial bytes. No timestamps (including host-read times), payloads,
/// identities, paths, or per-peer state are retained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpDiagnosticsSnapshot {
    /// Valid DataExpectingReply frames successfully written by the host.
    pub der_tx: u64,
    /// Valid DataExpectingReply frames decoded by the host.
    pub der_rx: u64,
    /// DNER application replies written via the transport's one-use reply channel.
    pub dner_tx_direct: u64,
    /// DNER frames written from the token queue, including broadcasts. These are
    /// not necessarily deferred replies; the transport cannot correlate them.
    pub dner_tx_queued: u64,
    /// Valid DataNotExpectingReply frames decoded by the host.
    pub dner_rx: u64,
    /// ReplyPostponed frames successfully written by the host.
    pub reply_postponed_tx: u64,
    /// Valid ReplyPostponed frames decoded, not necessarily for our transaction.
    pub reply_postponed_rx: u64,
    /// Timer expirations processed while the MAC was in WaitForReply.
    pub wait_for_reply_timeouts: u64,
    /// Host assembly discard operations: invalid stream decodes, noise/preamble
    /// resynchronization, or full-incomplete-buffer fallback. Not a count of bad
    /// wire frames or bytes; chunk boundaries can affect the number of events.
    pub invalid_frame_discards: u64,
    /// Partial host assemblies cleared after the existing stale-host gap limit.
    /// This is not wire Tframe_abort or a measurement of inter-octet silence.
    pub stale_partial_resets: u64,
    /// Outbound NPDU admissions rejected because the token queue was full.
    pub outbound_queue_full: u64,
    /// Oversize outbound NPDUs or direct application replies rejected at the
    /// transport boundary. Invalid MACs and not-started sends are not included.
    pub outbound_oversize: u64,
    /// Ingress NPDUs dropped because the receive channel was full.
    pub ingress_full: u64,
    /// Ingress NPDUs dropped because the receive channel was closed.
    pub ingress_closed: u64,
    /// Serial read calls that completed with an error.
    pub serial_read_errors: u64,
    /// Serial write calls that completed with an error, including backend
    /// direction/drain errors surfaced by that call. No separate drain is added.
    pub serial_write_errors: u64,
}

/// Cloneable counts-only handle obtained from [`super::MstpTransport::diagnostics`]
/// before moving the transport into a router or endpoint.
///
/// Clones share the same bounded counters in both execution modes. They keep only
/// the counters alive, not the transport, serial resource, node, or runtime. Counts
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

    /// Load an owned snapshot without locking or allocating.
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
        }

        impl Counters {
            fn snapshot(&self) -> MstpDiagnosticsSnapshot {
                MstpDiagnosticsSnapshot {
                    $($field: self.$field.load(Ordering::Relaxed),)+
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
    der_rx,
    dner_tx_direct,
    dner_tx_queued,
    dner_rx,
    reply_postponed_tx,
    reply_postponed_rx,
    wait_for_reply_timeouts,
    invalid_frame_discards,
    stale_partial_resets,
    outbound_queue_full,
    outbound_oversize,
    ingress_full,
    ingress_closed,
    serial_read_errors,
    serial_write_errors,
);

pub(super) fn increment(counter: &AtomicU64) {
    // Statistics only: never used for decisions or state publication.
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

impl Counters {
    pub(super) fn received(&self, kind: FrameType) {
        match kind {
            FrameType::BACnetDataExpectingReply => increment(&self.der_rx),
            FrameType::BACnetDataNotExpectingReply => increment(&self.dner_rx),
            FrameType::ReplyPostponed => increment(&self.reply_postponed_rx),
            _ => {}
        }
    }

    // The caller supplies exactly one successfully encoded/written standard
    // frame. Inspect only its type octet; no second decode, copy, or capture.
    pub(super) fn transmitted(&self, encoded: &[u8], direct_reply: bool) {
        match FrameType::from_raw(encoded[2]) {
            FrameType::BACnetDataExpectingReply => increment(&self.der_tx),
            FrameType::BACnetDataNotExpectingReply => {
                increment(if direct_reply {
                    &self.dner_tx_direct
                } else {
                    &self.dner_tx_queued
                });
            }
            FrameType::ReplyPostponed => increment(&self.reply_postponed_tx),
            _ => {}
        }
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
                    increment(&clone.counters.der_rx);
                    increment(&clone.counters.serial_read_errors);
                }
            });
        }
    });
    assert_eq!(handle.snapshot().der_rx, 400);
    assert_eq!(handle.snapshot().serial_read_errors, u64::MAX);
}
