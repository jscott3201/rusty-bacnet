//! The application route that feeds a stored Accumulator its input pulses
//! (#2259).
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Feed an Accumulator the input pulses its meter produced since the
    /// last call, counted by the application.
    ///
    /// In service the pulses step Present_Value, through Prescale's ratio
    /// when the object has one, rolling over modulo Max_Pres_Value + 1, and
    /// they count toward Pulse_Rate: with Limit_Monitoring_Interval set, the
    /// server closes a window at the end of each interval and Pulse_Rate
    /// becomes its count. While Out_Of_Service is TRUE Present_Value holds
    /// and Pulse_Rate keeps a client's value: the pulses still fill the rate
    /// window, but Present_Value drops them. `pulses` is an unsigned 64-bit count. An unknown object raises
    /// UNKNOWN_OBJECT and any object other than an Accumulator
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as in `set_present_value_local`.
    #[pyo3(signature = (object_id, pulses))]
    fn add_pulses_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        pulses: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        let future = async move {
            let guard = inner.read().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.add_pulses_local(&oid, pulses).await.map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
