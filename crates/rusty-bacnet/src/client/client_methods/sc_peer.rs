use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Copy this started SC transport's lifetime NPDU drop counters.
    /// Available while reconnecting; requires a started SC client.
    fn sc_npdu_drop_counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let counts = {
                let client = started(&inner).await?;
                match client.transport() {
                    AnyTransport::Sc(transport) => transport.npdu_drop_counts(),
                    _ => {
                        return Err(PyRuntimeError::new_err(
                            "sc_npdu_drop_counts requires an SC client",
                        ))
                    }
                }
            };
            Ok(crate::sc_npdu::snapshot(counts))
        })
    }

    /// Observe the accepted active SC Hub without network I/O. Returns None
    /// while disconnected; raises RuntimeError for a stopped or non-SC client.
    /// The frozen result is historical and does not keep the client alive.
    fn sc_peer_info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let info = {
                let client = started(&inner).await?;
                match client.transport() {
                    AnyTransport::Sc(transport) => transport.hub_peer_info().await,
                    _ => {
                        return Err(PyRuntimeError::new_err(
                            "sc_peer_info requires an SC client",
                        ))
                    }
                }
            };
            // No lifecycle lease or runtime object crosses the Python return.
            Ok(info.map(crate::sc_peer::PyScPeerInfo))
        })
    }

    /// Subscribe to this SC client's hub-connector events (#1829): an
    /// async iterator of `ScEvent`. The first call after a start gets the
    /// events since the client was built, beginning with the first
    /// `connected` (#1908); each later call gets the events from that call
    /// on. The connector never waits for the iterator; one more than 64
    /// events behind loses the oldest and counts them in its `missed`.
    /// Iteration ends when the client stops. Raises RuntimeError for a
    /// stopped or non-SC client.
    fn connection_events<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            match client.transport() {
                AnyTransport::Sc(transport) => Ok(crate::sc_events::PyScEventIterator::new(
                    inner
                        .take_first_sc_events()
                        .unwrap_or_else(|| transport.connection_events()),
                )),
                _ => Err(PyRuntimeError::new_err(
                    "connection_events requires an SC client",
                )),
            }
        })
    }

    /// A snapshot of this SC client's hub link (#1829): state, active hub,
    /// hub VMAC and heartbeat timing, without network I/O. Raises
    /// RuntimeError for a stopped or non-SC client.
    fn connection_status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            match client.transport() {
                AnyTransport::Sc(transport) => Ok(crate::sc_events::PyScLinkStatus(
                    transport.connection_status(),
                )),
                _ => Err(PyRuntimeError::new_err(
                    "connection_status requires an SC client",
                )),
            }
        })
    }
}
