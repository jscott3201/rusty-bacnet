//! The application route to a stored Life Safety object's Tracking_Value
//! (#1123); Present_Value goes through `set_present_value_local`.
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Set a Life Safety Point's or Zone's Tracking_Value, the live state the
    /// application derived.
    ///
    /// The value is an Enumerated BACnetLifeSafetyState, standard or from
    /// 256..=65535. Another number raises VALUE_OUT_OF_RANGE and another
    /// datatype INVALID_DATA_TYPE. Any object other than a Life Safety Point
    /// or Zone raises OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as in
    /// `set_present_value_local`. Present_Value, Silenced and
    /// Operation_Expected stay as they are. While Out_Of_Service is set the
    /// value waits for the return to service behind a client's simulated one.
    /// In service a SubscribeCOVProperty on Tracking_Value is notified.
    #[pyo3(signature = (object_id, value))]
    fn set_tracking_value_local<'py>(
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
            srv.set_tracking_value_local(&oid, prop_value)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
