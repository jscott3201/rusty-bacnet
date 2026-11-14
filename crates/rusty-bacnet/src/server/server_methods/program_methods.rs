//! The application route to a stored Program's Program_State (#2258), which
//! is read-only on the network and to `write_property_local`.
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Report a Program's Program_State, the state of the process the
    /// application runs.
    ///
    /// The value is an Enumerated BACnetProgramState from 0 (IDLE) to 5
    /// (UNLOADING): a larger number raises VALUE_OUT_OF_RANGE and another
    /// datatype INVALID_DATA_TYPE. Any object other than a Program raises
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. `write_property_local` refuses
    /// Program_State with WRITE_ACCESS_DENIED, as the network does.
    #[pyo3(signature = (object_id, value))]
    fn set_program_state_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        value: PyPropertyValue,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let prop_value = value.inner;

        let future = async move {
            let guard = inner.read().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_program_state_local(&oid, prop_value)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
