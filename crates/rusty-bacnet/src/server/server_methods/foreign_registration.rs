use bacnet_transport::port::TransportPort;

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
                let server = guard
                    .as_ref()
                    .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
                // A B/IPv6 server reports its own shape (#2365).
                if let Some(bip6) = server.transport().and_then(|t| t.bip6_transport()) {
                    let status = bip6.foreign_status().unwrap_or_default();
                    drop(guard);
                    return crate::registration_dicts::bip6_registration_dict(status);
                }
                server.foreign_registration().map_err(to_py_err)?
            };
            crate::registration_dicts::registration_dict(registration)
        })
    }
}
