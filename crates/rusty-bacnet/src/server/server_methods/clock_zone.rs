//! `BACnetServer.set_clock_zone`: the Device clock's zone, changed while
//! the server runs (#2395).

use pyo3::exceptions::PyValueError;

use super::super::*;

#[pymethods]
impl BACnetServer {
    /// Change the Device clock's zone while the server runs
    /// ([`set_clock_zone`](server::BACnetServer::set_clock_zone)): the
    /// Device's UTC_Offset, `utc_offset_minutes` west of UTC, and its
    /// Daylight_Savings_Status.
    ///
    /// The offset is checked when the method is called: one that
    /// `ClockConfig::new` refuses raises ValueError and nothing changes. A
    /// zone the running clock refuses raises ValueError from the awaitable.
    fn set_clock_zone<'py>(
        &self,
        py: Python<'py>,
        utc_offset_minutes: i16,
        daylight_savings: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let invalid = |error: bacnet_types::error::Error| PyValueError::new_err(error.to_string());
        let zone =
            server::ClockConfig::new(utc_offset_minutes, daylight_savings).map_err(invalid)?;
        let inner = Arc::clone(&self.inner);
        let future = async move {
            let guard = inner.lock().await;
            let srv = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("server not started"))?;
            srv.set_clock_zone(zone).await.map_err(invalid)
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }
}
