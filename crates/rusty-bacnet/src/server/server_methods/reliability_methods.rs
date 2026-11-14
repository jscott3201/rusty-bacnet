//! The application route to an object's Reliability (#2469), over the Rust
//! server's `set_reliability_local` (#2446).
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Set the Reliability the application works out for an object, such
    /// as NO_SENSOR when its hardware reports a missing sensor.
    ///
    /// The built-in Inputs, Outputs and Values, the Integer Value, Loop,
    /// Schedule, Life Safety Point and Zone, Access Zone, Credential Data
    /// Input and Accumulator take it. They raise WRITE_ACCESS_DENIED while
    /// Out_Of_Service is set and while Reliability_Evaluation_Inhibit is
    /// TRUE where they have it, and VALUE_OUT_OF_RANGE for a value outside
    /// BACnetReliability (11, 26 to 63, or past 65535). An unknown object
    /// raises UNKNOWN_OBJECT and one that takes no Reliability
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as in `set_present_value_local`.
    /// Status_Flags' FAULT follows the change, and COV subscribers hear of
    /// it.
    #[pyo3(signature = (object_id, reliability))]
    fn set_reliability_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        reliability: crate::types::PyReliability,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let reliability = reliability.to_rust();

        let future = async move {
            let guard = inner.read().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_reliability_local(&oid, reliability)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
