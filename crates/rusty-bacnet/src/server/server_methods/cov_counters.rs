use super::super::*;

use std::collections::HashMap;

use bacnet_device_core::cov::CovCounters;

/// Every `CovCounters` field under its Rust name: the dict that
/// `BACnetServer.cov_counters()` returns.
///
/// The pattern has no `..`, so a field added to `CovCounters` stops this
/// compiling until it is bound here, and a bound field left out of the map is
/// an unused variable. The dict can't silently miss a counter (#1084).
fn cov_counter_entries(counters: CovCounters) -> HashMap<&'static str, u64> {
    let CovCounters {
        subscriptions_active,
        subscriptions_created,
        subscriptions_rejected_quota,
        subscriptions_rejected_capacity,
        subscriptions_rejected_indefinite,
        subscriptions_cancelled,
        subscriptions_purged,
        notifications_sent,
        notifications_confirmed,
        notifications_unconfirmed,
        notification_bytes_sent,
        notifications_throttled_fanout,
        notifications_throttled_peer,
        timed_changes_dropped,
        untimed_references_oversized,
    } = counters;
    HashMap::from([
        ("subscriptions_active", subscriptions_active),
        ("subscriptions_created", subscriptions_created),
        ("subscriptions_rejected_quota", subscriptions_rejected_quota),
        (
            "subscriptions_rejected_capacity",
            subscriptions_rejected_capacity,
        ),
        (
            "subscriptions_rejected_indefinite",
            subscriptions_rejected_indefinite,
        ),
        ("subscriptions_cancelled", subscriptions_cancelled),
        ("subscriptions_purged", subscriptions_purged),
        ("notifications_sent", notifications_sent),
        ("notifications_confirmed", notifications_confirmed),
        ("notifications_unconfirmed", notifications_unconfirmed),
        ("notification_bytes_sent", notification_bytes_sent),
        (
            "notifications_throttled_fanout",
            notifications_throttled_fanout,
        ),
        ("notifications_throttled_peer", notifications_throttled_peer),
        ("timed_changes_dropped", timed_changes_dropped),
        ("untimed_references_oversized", untimed_references_oversized),
    ])
}

#[pymethods]
impl BACnetServer {
    /// Sample the COV subscription and notification counters: every Rust
    /// `CovCounters` field under its own name, zero at each start. Fields are
    /// sampled independently, not as an atomic aggregate. Raises
    /// RuntimeError("server not started") before start and after stop.
    fn cov_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        crate::py_async::future_into_py(py, async move {
            let counters = {
                let guard = inner.read().await;
                guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?
                    .cov_counters()
            };
            // Owned Rust data only; the bridge builds the dict while attached.
            Ok(cov_counter_entries(counters))
        })
    }
}

#[cfg(test)]
#[path = "cov_counters_tests.rs"]
pub(super) mod tests;
