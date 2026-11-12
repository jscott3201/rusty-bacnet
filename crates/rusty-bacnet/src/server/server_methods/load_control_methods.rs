//! The application's routes to a running Load Control's configuration
//! (#2497): Shed_Levels, Full_Duty_Baseline and State_Description. A request
//! under way is put to the `shed` callable again, which runs under the
//! binding re-entry refusal like any other look at it, so a callable that
//! calls one of these is refused rather than left waiting for the database
//! lock the server holds.
use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Replace a Load Control's Shed_Levels and Shed_Level_Descriptions
    /// while the server runs.
    ///
    /// `shed_levels` takes the `add_load_control` shape: `(level,
    /// description)` pairs in order of increasing shed, which may be a
    /// different number from before. A level of 0 raises
    /// BacnetProtocolError with VALUE_OUT_OF_RANGE and changes nothing. An
    /// unknown object raises UNKNOWN_OBJECT and any object other than a Load
    /// Control OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. A request under way is
    /// put to `shed` again and stays in its state unless the answer moves
    /// it. Storage does not keep the levels: give them as `add_load_control`
    /// keywords at every start, so a restored request is put to `shed` with
    /// them. They replace the elements a client wrote, which storage keeps
    /// over the keywords' levels until then, and storage drops those
    /// (#2502); if that save fails, the file holds them until a retry
    /// succeeds, the tries backing off to once every 64 s. Like
    /// `set_reliability_local`, it is not audited.
    #[pyo3(signature = (object_id, shed_levels))]
    fn set_load_control_shed_levels_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        shed_levels: Vec<(u64, String)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_load_control_shed_levels_local(&oid, shed_levels)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Set a Load Control's Full_Duty_Baseline, in kilowatts, while the
    /// server runs, or clear it with `None`.
    ///
    /// A negative or non-finite value raises VALUE_OUT_OF_RANGE (one past
    /// REAL's range rounds to infinity), and the other errors are those of
    /// `set_load_control_shed_levels_local`. A request under way is put to
    /// `shed` again with the new `full_duty_baseline`. Not audited.
    #[pyo3(signature = (object_id, kilowatts))]
    fn set_load_control_full_duty_baseline_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        kilowatts: Option<f64>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_load_control_full_duty_baseline_local(&oid, kilowatts.map(|value| value as f32))
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Set a Load Control's State_Description, a note for operators on its
    /// shed state, while the server runs, or clear it with `None`.
    ///
    /// The errors are those of `set_load_control_shed_levels_local`. A
    /// SubscribeCOVProperty on State_Description hears of the change; the
    /// request is not looked at again. Clearing it sends a subscriber
    /// nothing, and the call is not audited.
    #[pyo3(signature = (object_id, text))]
    fn set_load_control_state_description_local<'py>(
        &self,
        py: Python<'py>,
        object_id: PyObjectIdentifier,
        text: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_load_control_state_description_local(&oid, text)
                .await
                .map_err(to_py_err)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
