use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Sample independent request-admission counters; not an atomic aggregate.
    /// Like comm_state(), raises RuntimeError before start and after stop.
    fn request_admission_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let counters = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?
                    .request_admission_counters()
            };
            // Owned Rust data only; no Python borrow or server guard escapes.
            Ok(std::collections::HashMap::from([
                ("confirmed_active", counters.confirmed_active as u64),
                (
                    "confirmed_admitted_total",
                    counters.confirmed_admitted_total,
                ),
                (
                    "confirmed_overloaded_total",
                    counters.confirmed_overloaded_total,
                ),
                (
                    "confirmed_shutdown_rejected_total",
                    counters.confirmed_shutdown_rejected_total,
                ),
                ("unconfirmed_active", counters.unconfirmed_active as u64),
                (
                    "unconfirmed_admitted_total",
                    counters.unconfirmed_admitted_total,
                ),
                (
                    "unconfirmed_overloaded_total",
                    counters.unconfirmed_overloaded_total,
                ),
                (
                    "unconfirmed_shutdown_rejected_total",
                    counters.unconfirmed_shutdown_rejected_total,
                ),
                ("abort_active", counters.abort_active as u64),
                ("abort_admitted_total", counters.abort_admitted_total),
                (
                    "confirmed_fallback_dropped_total",
                    counters.confirmed_fallback_dropped_total,
                ),
                (
                    "abort_shutdown_rejected_total",
                    counters.abort_shutdown_rejected_total,
                ),
            ]))
        })
    }
}
