use super::super::*;

use std::collections::HashMap;

use bacnet_server::server::EventNotificationCounters;

/// Every `EventNotificationCounters` field under its Rust name: the dict that
/// `BACnetServer.event_notification_counters()` returns.
///
/// The pattern has no `..`, so a field added in Rust stops this compiling
/// until it is bound here, and a bound field left out of the map is an unused
/// variable.
fn event_notification_counter_entries(
    counters: EventNotificationCounters,
) -> HashMap<&'static str, u64> {
    let EventNotificationCounters {
        notification_class_missing,
        recipient_list_unavailable,
        recipient_list_invalid,
        recipient_list_too_long,
        device_recipient_unbound,
        recipient_unroutable,
        confirmed_broadcast_recipient,
        confirmed_waited_for_invoke_id,
        confirmed_rejected,
        confirmed_unanswered,
        unconfirmed_send_failed,
        apdu_too_large,
        received_not_forwarded,
        forwarding_cap_dropped,
        received_not_logged,
        lane_full,
        lanes_entries_full,
        lanes_full,
        cov_wait_expired,
    } = counters;
    HashMap::from([
        ("notification_class_missing", notification_class_missing),
        ("recipient_list_unavailable", recipient_list_unavailable),
        ("recipient_list_invalid", recipient_list_invalid),
        ("recipient_list_too_long", recipient_list_too_long),
        ("device_recipient_unbound", device_recipient_unbound),
        ("recipient_unroutable", recipient_unroutable),
        (
            "confirmed_broadcast_recipient",
            confirmed_broadcast_recipient,
        ),
        (
            "confirmed_waited_for_invoke_id",
            confirmed_waited_for_invoke_id,
        ),
        ("confirmed_rejected", confirmed_rejected),
        ("confirmed_unanswered", confirmed_unanswered),
        ("unconfirmed_send_failed", unconfirmed_send_failed),
        ("apdu_too_large", apdu_too_large),
        ("received_not_forwarded", received_not_forwarded),
        ("forwarding_cap_dropped", forwarding_cap_dropped),
        ("received_not_logged", received_not_logged),
        ("lane_full", lane_full),
        ("lanes_entries_full", lanes_entries_full),
        ("lanes_full", lanes_full),
        ("cov_wait_expired", cov_wait_expired),
    ])
}

#[pymethods]
impl BACnetServer {
    /// Sample the totals of event notifications the server did not deliver:
    /// every Rust `EventNotificationCounters` field under its own name, zero
    /// at each start and saturating at 2**64-1. Fields are sampled
    /// independently, not as an atomic aggregate. Raises
    /// RuntimeError("server not started") before start and after stop.
    fn event_notification_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        crate::py_async::future_into_py(py, async move {
            let counters = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?
                    .event_notification_counters()
            };
            // Owned Rust data only; the bridge builds the dict while attached.
            Ok(event_notification_counter_entries(counters))
        })
    }
}

#[cfg(test)]
#[path = "event_notification_counters_tests.rs"]
mod tests;
