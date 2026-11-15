//! Observe the client's own link without taking ownership of its transport.

use super::super::*;
use bacnet_transport::port::TransportPort;

#[pymethods]
impl BACnetClient {
    /// Subscribe to bounded raw frames on this client's existing transport.
    /// Full queues drop new arrivals and count them in `FrameIterator.missed`.
    /// Stop ends existing iterators; a restart needs a new subscription.
    /// Router application clients have no link of their own and are refused.
    #[pyo3(signature = (capacity=1024))]
    fn frames<'py>(&self, py: Python<'py>, capacity: usize) -> PyResult<Bound<'py, PyAny>> {
        let capacity = crate::frames::capacity(capacity)?;
        let inner = self.inner.clone();
        crate::py_async::future_into_py(py, async move {
            let client = started(&inner).await?;
            let transport = client.transport().link().ok_or_else(|| {
                PyValueError::new_err("a router application client has no frame tap of its own")
            })?;
            let tap = transport.frame_tap().ok_or_else(|| {
                PyValueError::new_err("this transport does not support frame observation")
            })?;
            let address = if let Some(bip) = transport.bip_transport() {
                bacnet_transport::bvll::decode_bip_mac(bip.local_mac())
                    .map(|(ip, port)| std::net::SocketAddrV4::new(ip.into(), port).to_string())
                    .unwrap_or_default()
            } else {
                crate::frames::hex_address(transport.local_mac())
            };
            Ok(inner
                .frames
                .subscribe(tap.subscribe_with_capacity(capacity), address))
        })
    }
}
