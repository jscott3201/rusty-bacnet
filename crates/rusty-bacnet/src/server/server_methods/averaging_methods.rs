//! Averaging registration and the application route that feeds a stored
//! Averaging object (#1083, #1092).
use super::super::*;
use bacnet_types::error::Error;

#[pymethods]
impl BACnetServer {
    /// Add an Averaging object to the server (before starting).
    ///
    /// The keyword arguments set Window_Interval in seconds (900 when
    /// omitted) and Window_Samples (15 when omitted); peers can write both,
    /// and each write discards the samples. An interval of 0, or a sample
    /// count of 0 or above 1440, raises VALUE_OUT_OF_RANGE.
    #[pyo3(signature = (instance, name, *, window_interval=None, window_samples=None))]
    fn add_averaging(
        &self,
        instance: u32,
        name: &str,
        window_interval: Option<u32>,
        window_samples: Option<u32>,
    ) -> PyResult<()> {
        let obj =
            averaging_object(instance, name, window_interval, window_samples).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Record one sample for an Averaging object, taken by the application.
    ///
    /// The server samples an object holding an Object_Property_Reference
    /// itself, every Window_Interval / Window_Samples seconds; this route
    /// feeds an object without one, and on one the server samples it adds
    /// one more attempt without moving the server's schedule.
    ///
    /// The value is a BOOLEAN (0 or 1), Signed, Unsigned, Enumerated or finite
    /// REAL, or `None` for an attempt that produced no value, which counts
    /// toward Attempted_Samples but not Valid_Samples. Another datatype raises
    /// INVALID_DATA_TYPE and NaN or an infinity VALUE_OUT_OF_RANGE. Any object
    /// other than an Averaging object raises
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as in `set_present_value_local`.
    /// Each call fills the next slot of the Window_Samples window and the
    /// statistics change together and go through the server's COV path: a
    /// SubscribeCOVProperty on one of them is notified, while SubscribeCOV on
    /// an Averaging object is refused.
    #[pyo3(signature = (object_id, value))]
    fn add_averaging_sample_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        value: Option<PyPropertyValue>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let sample = value.map(|value| value.inner);

        let future = async move {
            let guard = inner.read().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.add_averaging_sample_local(&oid, sample)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}

/// An Averaging object with the window the keyword arguments ask for.
fn averaging_object(
    instance: u32,
    name: &str,
    window_interval: Option<u32>,
    window_samples: Option<u32>,
) -> Result<AveragingObject, Error> {
    let mut obj = AveragingObject::new(instance, name)?;
    if let Some(seconds) = window_interval {
        obj.set_window_interval(seconds)?;
    }
    if let Some(samples) = window_samples {
        obj.set_window_samples(samples)?;
    }
    Ok(obj)
}
