//! Observe the client's own link without taking ownership of its transport.

use super::super::*;
use bacnet_transport::port::TransportPort;
use bacnet_types::data_link::DataLink;
use std::net::SocketAddr;

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
            let mac = transport.local_mac();
            let socket = match link {
                DataLink::Bip => bacnet_transport::bvll::decode_bip_mac(mac)
                    .ok()
                    .map(|(ip, port)| SocketAddr::from((ip, port))),
                DataLink::Bip6 => transport
                    .bip6_transport()
                    .and_then(|bip6| bip6.local_address())
                    .map(SocketAddr::V6),
                _ => None,
            };
            let address = crate::frames::local_address(link, mac, socket).ok_or_else(|| {
                PyRuntimeError::new_err("the client's transport reports no local address")
            })?;
            Ok(inner
                .frames
                .subscribe(tap.subscribe_with_capacity(capacity), address, clock))
        })
    }
}
