use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Copy this started SC transport's lifetime NPDU drop counters.
    /// Requires a started SC server, even when its Hub is disconnected.
    fn sc_npdu_drop_counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let counts = {
                let guard = inner.read().await;
                let server = guard.server()?;
                match server.transport().and_then(AppTransport::link) {
                    Some(AnyTransport::Sc(transport)) => transport.npdu_drop_counts(),
                    _ => {
                        return Err(PyRuntimeError::new_err(
                            "sc_npdu_drop_counts requires an SC server",
                        ));
                    }
                }
            };
            Ok(crate::sc_npdu::snapshot(counts))
        })
    }
}
