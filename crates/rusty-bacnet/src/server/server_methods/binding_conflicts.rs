use super::super::*;

#[pymethods]
impl BACnetServer {
    /// The server's Who-Is for a device that two or more stations answered,
    /// so none was bound (#2825): a first contact's, a lapsed binding's, or
    /// a binding check's second. Raises RuntimeError("server not started")
    /// before start and after stop.
    fn device_binding_conflicts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let guard = inner.read().await;
            Ok(guard.server()?.device_binding_conflicts().await)
        })
    }
}
