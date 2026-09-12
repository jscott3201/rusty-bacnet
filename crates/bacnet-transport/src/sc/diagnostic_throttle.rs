//! Owner-local throttle for malformed-frame diagnostics only.
//!
//! Bounds `warn!`/`debug!` volume on hostile-peer paths without changing any
//! accept/reject/NAK/silence decision bit-for-bit. NAK sends stay under the
//! existing [`super::rejection::RejectionBudget`]; this throttle only gates
//! log emission.
//!
//! Exact bound: at most [`MAX_DIAGNOSTICS_PER_WINDOW`] diagnostic event(s) per
//! [`DIAGNOSTIC_WINDOW`] per throttle instance. A burst of N malformed frames
//! within one window emits exactly 1 diagnostic (the first) and counts N-1 as
//! suppressed; the suppressed count is available for a periodic summary on the
//! next emission. A lone error always emits (no silent-everything regression).
//! Over W windows the total is O(W), never O(N).
//!
//! Window choice (1s) reuses the existing local-window concepts
//! (`MANAGEMENT_RATE_WINDOW` = 1s for B/IP management quota,
//! `SOLICITED_ADVERTISEMENT_MIN_INTERVAL` = 1s for solicited replies) rather
//! than inventing a new rate policy. No Annex AB clause mandates log rates:
//! AB specifies BVLC wire behavior, error codes, and connection procedures
//! (Annex AB, local PDF pp. 1377-1410 per STANDARD_NAVIGATION.md); diagnostic
//! volume is implementation-local, consistent with the existing "local
//! anti-storm policy (not a wire deadline)" and "owner-local budget" comments.
//! Each loop/connection holds its own instance so one flooding peer cannot
//! suppress another instance's first-occurrence diagnostic.

use std::time::{Duration, Instant};

/// Throttle window for malformed-frame diagnostics.
///
/// Reuses the 1s local-window precedent from B/IP management quota and the
/// solicited-Advertisement anti-storm interval.
pub(crate) const DIAGNOSTIC_WINDOW: Duration = Duration::from_secs(1);

/// Maximum diagnostic events emitted per window per throttle instance.
///
/// Bound: burst of N frames in one window emits exactly 1 diagnostic.
/// Single isolated errors always emit.
pub(crate) const MAX_DIAGNOSTICS_PER_WINDOW: u32 = 1;

/// Fixed-window, strictly bounded diagnostic throttle.
///
/// Time is supplied by the caller (`should_emit`) so accounting stays
/// deterministic in tests without sleeps; production uses
/// [`DiagnosticThrottle::should_emit_now`].
#[derive(Clone, Debug)]
pub(crate) struct DiagnosticThrottle {
    window_start: Option<Instant>,
    emitted_in_window: u32,
    suppressed: u64,
}

impl DiagnosticThrottle {
    pub(crate) fn new() -> Self {
        Self {
            window_start: None,
            emitted_in_window: 0,
            suppressed: 0,
        }
    }

    fn roll_window_if_expired(&mut self, now: Instant) {
        match self.window_start {
            None => {
                self.window_start = Some(now);
            }
            Some(start) => {
                if now.saturating_duration_since(start) >= DIAGNOSTIC_WINDOW {
                    self.window_start = Some(now);
                    self.emitted_in_window = 0;
                }
            }
        }
    }

    /// Returns `true` if the caller should emit a diagnostic now.
    ///
    /// On `false` the event is counted as suppressed (see [`Self::suppressed`]
    /// / [`Self::take_suppressed`]) and the caller must not log. On `true`
    /// the caller should log once and may include [`Self::take_suppressed`]
    /// as a summary suffix. Never affects wire decisions.
    pub(crate) fn should_emit(&mut self, now: Instant) -> bool {
        self.roll_window_if_expired(now);
        if self.emitted_in_window < MAX_DIAGNOSTICS_PER_WINDOW {
            self.emitted_in_window += 1;
            true
        } else {
            self.suppressed += 1;
            false
        }
    }

    /// Production entry point using monotonic time.
    pub(crate) fn should_emit_now(&mut self) -> bool {
        self.should_emit(Instant::now())
    }

    /// Total suppressed events since the last [`Self::take_suppressed`].
    pub(crate) fn suppressed(&self) -> u64 {
        self.suppressed
    }

    /// Takes the suppressed count for a periodic summary, resetting to zero.
    pub(crate) fn take_suppressed(&mut self) -> u64 {
        std::mem::take(&mut self.suppressed)
    }
}

impl Default for DiagnosticThrottle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_error_emits_without_suppression() {
        let mut throttle = DiagnosticThrottle::new();
        let now = Instant::now();
        assert!(throttle.should_emit(now));
        assert_eq!(throttle.suppressed(), 0);
        assert_eq!(throttle.take_suppressed(), 0);
    }

    #[test]
    fn burst_of_malformed_frames_produces_single_diagnostic() {
        let mut throttle = DiagnosticThrottle::new();
        let now = Instant::now();
        let burst = 1_000;
        let mut emitted = 0;
        for _ in 0..burst {
            if throttle.should_emit(now) {
                emitted += 1;
            }
        }
        assert_eq!(
            emitted, MAX_DIAGNOSTICS_PER_WINDOW as usize,
            "burst of {burst} within one window must emit exactly \
             {MAX_DIAGNOSTICS_PER_WINDOW}"
        );
        assert_eq!(throttle.suppressed(), (burst - 1) as u64);
        assert_eq!(throttle.take_suppressed(), (burst - 1) as u64);
        assert_eq!(throttle.suppressed(), 0);
    }

    #[test]
    fn window_roll_allows_next_diagnostic_with_summary() {
        let mut throttle = DiagnosticThrottle::new();
        let start = Instant::now();
        assert!(throttle.should_emit(start));
        assert!(!throttle.should_emit(start));
        assert!(!throttle.should_emit(start));
        assert_eq!(throttle.suppressed(), 2);
        let next_window = start + DIAGNOSTIC_WINDOW + Duration::from_nanos(1);
        assert!(
            throttle.should_emit(next_window),
            "next window must allow one more diagnostic"
        );
        assert_eq!(
            throttle.take_suppressed(),
            2,
            "suppressed count survives the window roll for the summary"
        );
    }

    /// Burst through the real connection gate: every oversized NPDU keeps its
    /// existing drop-without-state-change treatment bit-for-bit, while the
    /// connection throttle bounds diagnostics to one per window.
    #[test]
    fn burst_preserves_wire_decisions_while_bounding_diagnostics() {
        use super::super::{ScConnection, ScConnectionState};
        use crate::sc_frame::{ScFunction, ScMessage};
        use bytes::Bytes;

        let mut conn = ScConnection::new([0x01; 6], [1; 16]);
        conn.state = ScConnectionState::Connected;
        let oversized = vec![0x01; 2_000];
        for _ in 0..100 {
            let msg = ScMessage {
                function: ScFunction::EncapsulatedNpdu,
                message_id: 0x2233,
                originating_vmac: Some([0x22; 6]),
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::from(oversized.clone()),
            };
            assert!(
                conn.handle_received(&msg).is_none(),
                "oversized NPDU must stay dropped"
            );
            assert_eq!(
                conn.state,
                ScConnectionState::Connected,
                "oversized NPDU must not change state"
            );
        }
        assert_eq!(
            conn.malformed_diag_suppressed(),
            99,
            "burst of 100 within one window must emit 1 diagnostic and suppress 99"
        );
        // A valid frame after the burst still delivers (no state corruption).
        let valid = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x2234,
            originating_vmac: Some([0x22; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
        };
        assert!(
            conn.handle_received(&valid).is_some(),
            "valid NPDU must still deliver after a malformed burst"
        );
    }

    /// Single legitimate error still emits (no silent-everything regression)
    /// at the connection gate.
    #[test]
    fn single_connection_error_still_emits() {
        use super::super::{ScConnection, ScConnectionState};
        use crate::sc_frame::{ScFunction, ScMessage};
        use bytes::Bytes;

        let mut conn = ScConnection::new([0x01; 6], [1; 16]);
        conn.state = ScConnectionState::Connected;
        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x2233,
            originating_vmac: Some([0x22; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(vec![0x01; 2_000]),
        };
        assert!(conn.handle_received(&msg).is_none());
        assert_eq!(conn.malformed_diag_suppressed(), 0);
    }
}
