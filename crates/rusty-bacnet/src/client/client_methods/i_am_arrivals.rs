//! One record per I-Am the client's link hands it (#2612), beside `frames()`.

use super::super::*;

#[pymethods]
impl BACnetClient {
    /// Subscribe to a record of every I-Am this client's existing transport
    /// receives (B/IP, B/IPv6, SC or MS/TP), repeats and duplicate
    /// instances kept, read from a bounded frame-tap queue of `capacity`
    /// frames of every kind. A full queue drops new frames; the next record
    /// reports the gap in `IAmArrival.missed_before`. A client sends no
    /// I-Am, so nothing from its own address is left out. Stop ends
    /// existing iterators; a restart needs a new subscription. Router
    /// application clients have no link of their own and are refused.
    #[pyo3(signature = (capacity=1024))]
    fn i_am_arrivals<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let clock = crate::frames::Clock::get(py)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let transport = client.transport().link().ok_or_else(|| {
                PyValueError::new_err("a router application client has no frame tap of its own")
            })?;
            let tap = transport.frame_tap().ok_or_else(|| {
                PyValueError::new_err("this transport does not support frame observation")
            })?;
            Ok(crate::i_am_arrivals::subscribe(
                &inner.frames,
                tap.subscribe_with_capacity(capacity),
                None,
                clock,
            ))
        })
    }
}
