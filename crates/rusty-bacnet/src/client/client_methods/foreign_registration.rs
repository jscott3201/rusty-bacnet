use bacnet_transport::port::TransportPort;

use super::super::*;

#[pymethods]
impl BACnetClient {
    /// How the client's foreign-device registration is going (#938): a
    /// dict with `registered`, `lease_remaining` and `next_attempt_in`
    /// (seconds or None), `last_bbmd`, `last_result` (the BVLC result
    /// code), `last_outcome`, and the `attempts`, `accepted` and `refused`
    /// totals. A B/IPv6 client returns its own dict (#2365): the BBMD in
    /// use, `waiting_for_bbmd`, the lookups and the releases of BBMDs left.
    /// Sends nothing. Raises RuntimeError before start and after stop.
    fn foreign_registration<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            // A B/IPv6 client reports its own shape (#2365).
            if let Some(bip6) = client.transport().bip6_transport() {
                let status = bip6.foreign_status().unwrap_or_default();
                return crate::registration_dicts::bip6_registration_dict(status);
            }
            let registration = client.foreign_registration().map_err(to_py_err)?;
            crate::registration_dicts::registration_dict(registration)
        })
    }
}
