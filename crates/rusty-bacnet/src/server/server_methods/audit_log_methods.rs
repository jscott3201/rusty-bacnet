//! The application's purge of an Audit Log on a running server (#1238).
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Purge an Audit Log: clear its records and append a BUFFER_PURGED
    /// status record.
    ///
    /// No peer can purge an Audit Log, since its Record_Count is read-only,
    /// so this is the application's route. The record is appended whether
    /// or not logging is enabled, and also carries LOG_DISABLED while
    /// logging is off; Total_Record_Count keeps counting.
    /// The log commits the purge to its storage before serving it, with the
    /// server's object database free meanwhile. Raises BacnetProtocolError
    /// with OBJECT / UNKNOWN_OBJECT when nothing has that identifier, OBJECT
    /// / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED when it is not an Audit Log,
    /// and DEVICE / OPERATIONAL_PROBLEM without a valid clock or when the
    /// commit fails; the log is then left as it was. RuntimeError when the
    /// server is not running.
    #[pyo3(signature = (object_id))]
    fn purge_audit_log<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.purge_audit_log(&oid).await.map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Record a reset the application makes for its own reasons: a
    /// DEVICE_RESET Audit record from this device, with no invoke ID
    /// (#1564). A peer's accepted restart is recorded already. RuntimeError
    /// when the server is not running.
    fn report_device_reset_local<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.report_device_reset_local().await.map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
