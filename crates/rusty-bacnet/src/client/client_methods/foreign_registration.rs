use super::super::*;

#[pymethods]
impl BACnetClient {
    /// How the client's foreign-device registration is going (#938): a
    /// dict with `registered`, `lease_remaining` and `next_attempt_in`
    /// (seconds or None), `last_bbmd`, `last_result` (the BVLC result
    /// code), `last_outcome`, and the `attempts`, `accepted` and `refused`
    /// totals. Sends nothing. Raises RuntimeError before start and after
    /// stop.
    fn foreign_registration<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let registration = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| {
                        PyRuntimeError::new_err("client not started — use 'async with'")
                    })?
                    .foreign_registration()
                    .map_err(to_py_err)?
            };
            crate::bip_options::registration_dict(registration)
        })
    }
}
