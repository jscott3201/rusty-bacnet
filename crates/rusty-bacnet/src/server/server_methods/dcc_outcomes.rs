use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Independently sample five saturating lifetime DCC completion totals.
    /// Local telemetry, not a trace bridge or durable audit log. Raises
    /// RuntimeError("server not started") before start and after stop.
    fn dcc_outcome_counters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let counters = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?
                    .dcc_outcome_counters()
            };
            Ok(std::collections::HashMap::from([
                ("accepted_total", counters.accepted_total),
                ("policy_denied_total", counters.policy_denied_total),
                ("password_failure_total", counters.password_failure_total),
                ("deprecated_denied_total", counters.deprecated_denied_total),
                ("malformed_total", counters.malformed_total),
            ]))
        })
    }
}
