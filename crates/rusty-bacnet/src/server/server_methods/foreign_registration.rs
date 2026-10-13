use super::super::*;

#[pymethods]
impl BACnetServer {
    /// How the server's foreign-device registration is going (#938), as
    /// `BACnetClient.foreign_registration()` reports it. Sends nothing.
    /// Raises RuntimeError before start and after stop.
    fn foreign_registration<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        crate::py_async::future_into_py(py, async move {
            let registration = {
                let guard = inner.lock().await;
                guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?
                    .foreign_registration()
                    .map_err(to_py_err)?
            };
            crate::bip_options::registration_dict(registration)
        })
    }
}
