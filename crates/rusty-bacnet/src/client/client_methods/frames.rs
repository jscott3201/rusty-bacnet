//! Observe the client's own link without taking ownership of its transport.

use super::super::*;
use bacnet_transport::port::TransportPort;

#[pymethods]
impl BACnetClient {
    /// Subscribe to bounded raw frames on this client's existing transport
    /// (B/IP, B/IPv6, SC or MS/TP). A full queue drops new arrivals; the
    /// next frame queued reports the gap in `Frame.missed_before`. Stop ends
    /// existing iterators; a restart needs a new subscription. Router
    /// application clients have no link of their own and are refused.
    #[pyo3(signature = (capacity=1024))]
    fn frames<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
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
            let link = transport
                .data_link()
                .ok_or_else(|| PyValueError::new_err("this transport names no data link"))?;
            let socket = crate::frames::link_socket(transport);
            let address = crate::frames::local_address(link, transport.local_mac(), socket)
                .ok_or_else(|| {
                    PyRuntimeError::new_err("the client's transport reports no local address")
                })?;
            Ok(inner
                .frames
                .subscribe(tap.subscribe_with_capacity(capacity), address, clock))
        })
    }
}
