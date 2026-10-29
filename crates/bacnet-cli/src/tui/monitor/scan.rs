//! Receive-only baud-scan requests and evidence. Selection is a heuristic.

use bacnet_transport::mstp::monitor::MonitorStats;
use std::cmp::Ordering;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering as AtomicOrdering},
};
use std::time::Duration;

/// An operation identity, nonzero within its session.
pub(crate) type ScanId = u64;

/// Explicit scanner controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanCommand {
    /// Measure every supported rate for this window.
    Start { id: ScanId, window: Duration },
    /// Stop this operation and restore the original rate.
    Cancel { id: ScanId },
}
impl ScanCommand {
    /// The operation this command addresses.
    pub(crate) fn id(self) -> ScanId {
        match self {
            Self::Start { id, .. } | Self::Cancel { id } => id,
        }
    }
}

/// Coordinator reservation covering retirement, candidates and restoration.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScanGate(Arc<AtomicU64>);
impl ScanGate {
    /// Reserve a fresh operation before forwarding it.
    pub(crate) fn reserve(&self, id: ScanId) -> bool {
        id != 0
            && self
                .0
                .compare_exchange(0, id, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
                .is_ok()
    }
    /// Whether this identity already owns the transition.
    pub(crate) fn owns(&self, id: ScanId) -> bool {
        id != 0 && self.0.load(AtomicOrdering::SeqCst) == id
    }
    /// Whether any scan owns this session's serial transitions.
    pub(crate) fn busy(&self) -> bool {
        self.0.load(AtomicOrdering::SeqCst) != 0
    }
    /// Release only the named reservation, after proven retirement.
    pub(crate) fn release(&self, id: ScanId) {
        let _ = self
            .0
            .compare_exchange(id, 0, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst);
    }
}

/// A candidate's measurement status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowStatus {
    /// Not opened yet.
    Pending,
    /// Acquiring a fresh input.
    Opening,
    /// Measuring decoded traffic.
    Measuring,
    /// Waiting for acknowledged release.
    Closing,
    /// The entire requested window was measured and release confirmed.
    Complete,
    /// No full window, with the reason retained.
    Incomplete(String),
    /// Admission or retirement failed.
    Failed(String),
}

/// Counters used by the heuristic, separate from ordinary monitor state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    /// Valid decoded frames, including sparse observations.
    pub(crate) valid: u64,
    /// Header CRC, data CRC, malformed and aborted counts, in that order.
    pub(crate) errors: [u64; 4],
    /// Noise octets, excluded from the ratio.
    pub(crate) noise: u64,
}
impl Evidence {
    /// Snapshot actual decoder counters.
    pub(crate) fn of(stats: &MonitorStats) -> Self {
        let e = stats.errors;
        Self {
            valid: stats.frames.total(),
            errors: [e.header_crc, e.data_crc, e.malformed, e.frame_aborts],
            noise: e.noise_octets,
        }
    }
    /// The sum is wider than every saturated source counter.
    pub(crate) fn errors(self) -> u128 {
        self.errors.into_iter().map(u128::from).sum()
    }
    /// Saturated inputs cannot support an exact ranking.
    pub(crate) fn saturated(self) -> bool {
        self.valid == u64::MAX || self.errors.contains(&u64::MAX)
    }
    /// Display only; winner selection uses exact arithmetic.
    pub(crate) fn percent(self) -> Option<f64> {
        let denominator = u128::from(self.valid) + self.errors();
        (denominator > 0).then(|| 100.0 * self.valid as f64 / denominator as f64)
    }
    fn compare(self, other: Self) -> Ordering {
        ratio_cmp(
            u128::from(self.valid),
            u128::from(self.valid) + self.errors(),
            u128::from(other.valid),
            u128::from(other.valid) + other.errors(),
        )
    }
}

/// Compare nonnegative fractions without overflowing their cross products.
fn ratio_cmp(mut a: u128, mut b: u128, mut c: u128, mut d: u128) -> Ordering {
    let mut reverse = false;
    loop {
        let quotients = (a / b).cmp(&(c / d));
        if quotients != Ordering::Equal {
            return if reverse {
                quotients.reverse()
            } else {
                quotients
            };
        }
        let (r, s) = (a % b, c % d);
        if r == 0 || s == 0 {
            let order = r.cmp(&s);
            return if reverse { order.reverse() } else { order };
        }
        (a, b, c, d) = (b, r, d, s);
        reverse = !reverse;
    }
}

/// One rate's measured evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanRow {
    /// Candidate rate.
    pub(crate) baud: u32,
    /// Time actually measured, excluding close waits.
    pub(crate) elapsed: Duration,
    /// Fresh decoder counters.
    pub(crate) evidence: Evidence,
    /// Measurement and release outcome.
    pub(crate) status: RowStatus,
}

/// Serial ownership phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanPhase {
    /// Closing the ordinary monitor and its capture.
    Retiring,
    /// Opening or measuring a candidate.
    Candidates,
    /// Reopening the original configured rate.
    Restoring,
    /// Original monitor resumed with fresh counters.
    Restored,
    /// Restoration failed or release is unproven.
    Stopped(String),
}

/// A report retained independently of ordinary monitor samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scan {
    /// Matching request identity.
    pub(crate) id: ScanId,
    /// Original rate, the only rate restored automatically.
    pub(crate) original: u32,
    /// Requested per-rate window.
    pub(crate) window: Duration,
    /// Six rates in the supported order.
    pub(crate) rows: Vec<ScanRow>,
    /// Current serial ownership phase.
    pub(crate) phase: ScanPhase,
    /// Manual cancellation makes the result partial.
    pub(crate) cancelled: bool,
}
impl Scan {
    /// Empty evidence for a new explicit scan.
    pub(crate) fn new(id: ScanId, original: u32, window: Duration) -> Self {
        Self {
            id,
            original,
            window,
            rows: crate::mstp::BAUD_RATES
                .iter()
                .map(|&baud| ScanRow {
                    baud,
                    elapsed: Duration::ZERO,
                    evidence: Evidence::default(),
                    status: RowStatus::Pending,
                })
                .collect(),
            phase: ScanPhase::Retiring,
            cancelled: false,
        }
    }
    /// Whether the operation still owns transitions.
    pub(crate) fn active(&self) -> bool {
        !matches!(self.phase, ScanPhase::Restored | ScanPhase::Stopped(_))
    }
    /// Result with no count/order tiebreak or unsupported confidence claim.
    pub(crate) fn suggestion(&self) -> Suggestion {
        if self.cancelled || self.rows.iter().any(|r| r.status != RowStatus::Complete) {
            return Suggestion::Partial;
        }
        if self.rows.iter().any(|r| r.evidence.saturated()) {
            return Suggestion::Saturated;
        }
        let mut leaders: Vec<&ScanRow> = Vec::new();
        for row in self.rows.iter().filter(|r| r.evidence.valid > 0) {
            match leaders
                .first()
                .map(|best| row.evidence.compare(best.evidence))
            {
                None | Some(Ordering::Greater) => {
                    leaders.clear();
                    leaders.push(row);
                }
                Some(Ordering::Equal) => leaders.push(row),
                Some(Ordering::Less) => {}
            }
        }
        match leaders.as_slice() {
            [] => Suggestion::Insufficient,
            [row] => Suggestion::Rate(row.baud),
            rows => Suggestion::Ambiguous(rows.iter().map(|r| r.baud).collect()),
        }
    }
}

/// An evidence-qualified suggestion, never an instruction to change the port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Suggestion {
    /// A unique highest ratio with at least one valid frame.
    Rate(u32),
    /// Equal highest ratios.
    Ambiguous(Vec<u32>),
    /// No rate saw a valid frame.
    Insufficient,
    /// Some candidates were not completely measured.
    Partial,
    /// Source counters hit their saturation limit.
    Saturated,
}

/// Reliable, session-scoped control reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanReport {
    /// The current bounded six-row report.
    Progress(Box<Scan>),
    /// The named command was not admitted.
    Rejected {
        id: ScanId,
        cancel: bool,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    fn complete() -> Scan {
        let mut scan = Scan::new(1, 38400, Duration::from_secs(1));
        for row in &mut scan.rows {
            row.status = RowStatus::Complete;
        }
        scan
    }
    #[test]
    fn scan_score_zero_sparse_tie_noise_and_partial() {
        let mut scan = complete();
        scan.rows[0].evidence.noise = 900;
        assert_eq!(scan.suggestion(), Suggestion::Insufficient);
        scan.rows[2].evidence.valid = 1;
        assert_eq!(scan.suggestion(), Suggestion::Rate(38400));
        scan.rows[4].evidence.valid = 100;
        assert_eq!(scan.suggestion(), Suggestion::Ambiguous(vec![38400, 76800]));
        scan.rows[4].evidence.errors[0] = 1;
        assert_eq!(scan.suggestion(), Suggestion::Rate(38400));
        scan.rows[0].status = RowStatus::Incomplete("EOF".into());
        assert_eq!(scan.suggestion(), Suggestion::Partial);
    }
    #[test]
    fn scan_score_exact_equivalence_extremes_and_saturation() {
        assert_eq!(ratio_cmp(1, 3, 2, 6), Ordering::Equal);
        assert_eq!(ratio_cmp(2, 3, 3, 4), Ordering::Less);
        let max = u128::from(u64::MAX);
        assert_eq!(
            ratio_cmp(max, max * 5, max - 1, (max - 1) * 5),
            Ordering::Equal
        );
        assert_eq!(
            ratio_cmp(max - 1, max * 5, max - 2, max * 5),
            Ordering::Greater
        );
        assert_eq!(ratio_cmp(1, 2, max, max * 5), Ordering::Greater);
        let mut scan = complete();
        scan.rows[2].evidence.valid = u64::MAX;
        assert_eq!(scan.suggestion(), Suggestion::Saturated);
    }
    #[test]
    fn scan_gate_retains_identity_until_matching_release() {
        let gate = ScanGate::default();
        assert!(!gate.reserve(0));
        assert!(gate.reserve(1));
        assert!(!gate.reserve(2));
        gate.release(2);
        assert!(gate.busy());
        gate.release(1);
        assert!(gate.reserve(2));
    }
}
