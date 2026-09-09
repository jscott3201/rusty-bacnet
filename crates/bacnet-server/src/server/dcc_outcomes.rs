use super::*;
use std::fmt;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
#[path = "dcc_trace_tests.rs"]
pub(crate) mod trace_tests;

/// Lifetime totals for completed, admitted DCC handlers, saturating at `u64::MAX`.
/// New servers start at zero. Fields are independently sampled, not an atomic
/// aggregate. These are local operational telemetry, not a durable audit log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DccOutcomeCounters {
    /// State/timer replacement committed; does not imply response delivery.
    pub accepted_total: u64,
    /// A valid mode was refused by local policy.
    pub policy_denied_total: u64,
    /// The configured password check failed.
    pub password_failure_total: u64,
    /// Deprecated DISABLE was refused after the password check.
    pub deprecated_denied_total: u64,
    /// Decode failed or the decoded mode was unknown after the password check.
    pub malformed_total: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum DccOutcome {
    Accepted,
    PolicyDenied,
    PasswordFailure,
    DeprecatedDenied,
    Malformed,
}

impl DccOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::PolicyDenied => "policy_denied",
            Self::PasswordFailure => "password_failure",
            Self::DeprecatedDenied => "deprecated_denied",
            Self::Malformed => "malformed",
        }
    }
}

/// Only decoded, non-secret metadata can cross the validation boundary.
#[derive(Clone, Copy, Default)]
pub(crate) struct DccMetadata {
    pub mode: Option<u32>,
    pub duration: Option<u16>,
}

#[derive(Default)]
pub(super) struct DccOutcomes([AtomicU64; 5]);

impl DccOutcomes {
    pub(super) fn snapshot(&self) -> DccOutcomeCounters {
        let [accepted_total, policy_denied_total, password_failure_total, deprecated_denied_total, malformed_total] =
            self.0.each_ref().map(|v| v.load(Ordering::Relaxed));
        DccOutcomeCounters {
            accepted_total,
            policy_denied_total,
            password_failure_total,
            deprecated_denied_total,
            malformed_total,
        }
    }

    pub(super) fn record(
        &self,
        outcome: DccOutcome,
        metadata: DccMetadata,
        invoke_id: u8,
        source_mac: &[u8],
        source: Option<&NpduAddress>,
    ) {
        // Counters publish no other data. Saturation and relaxed independent
        // samples intentionally provide no cross-handler ordering guarantee.
        let index = match outcome {
            DccOutcome::Accepted => 0,
            DccOutcome::PolicyDenied => 1,
            DccOutcome::PasswordFailure => 2,
            DccOutcome::DeprecatedDenied => 3,
            DccOutcome::Malformed => 4,
        };
        let _ = self.0[index].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            Some(n.saturating_add(1))
        });
        // Lazy bounded Display performs no address formatting/allocation when
        // the event's own callsite is disabled; no second filter callsite needed.
        let routed = source.map(|s| s.mac_address.as_slice());
        tracing::debug!(target: "bacnet_server::dcc_outcome",
            outcome = outcome.label(), invoke_id, service = 17u8,
            decoded_mode = metadata.mode, duration_minutes = metadata.duration,
            source_kind = if source.is_some() { "claimed_routed" } else { "claimed_direct" },
            claimed_source_mac = %BoundedHex(source_mac),
            source_mac_truncated = source_mac.len() > 32,
            claimed_snet = source.map(|s| s.network),
            claimed_sadr = %BoundedHex(routed.unwrap_or_default()),
            sadr_truncated = routed.is_some_and(|s| s.len() > 32),
        );
    }
}

struct BoundedHex<'a>(&'a [u8]);
#[test]
fn dcc_outcomes_saturation_without_subscriber() {
    let counters = DccOutcomes::default();
    for counter in &counters.0 {
        counter.store(u64::MAX - 1, Ordering::Relaxed);
    }
    tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
        for _ in 0..3 {
            for outcome in [
                DccOutcome::Accepted,
                DccOutcome::PolicyDenied,
                DccOutcome::PasswordFailure,
                DccOutcome::DeprecatedDenied,
                DccOutcome::Malformed,
            ] {
                counters.record(outcome, DccMetadata::default(), 0, &[], None);
            }
        }
    });
    assert!(counters
        .0
        .iter()
        .all(|counter| counter.load(Ordering::Relaxed) == u64::MAX));
}
impl fmt::Display for BoundedHex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter().take(32) {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Sample completed admitted DCC outcomes, including after `stop()`.
    /// Excludes pre-handler rejections, duplicates, incomplete cancelled handlers,
    /// timer expiry and response delivery outcomes. No history is retained.
    pub fn dcc_outcome_counters(&self) -> DccOutcomeCounters {
        self.dcc_outcomes.snapshot()
    }
}
