//! Frozen Python copies of received I-Haves, discovered routers and network
//! reports, and the async iterators over the I-Have and network-report
//! streams (#1850).

use bacnet_client::client::{DiscoveredRouter, ReceivedIHave};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::event_stream::{LossyReceiver, ended};
use crate::types::PyObjectIdentifier;

fn source_network(source: &Option<NpduAddress>) -> Option<u16> {
    source.as_ref().map(|address| address.network)
}

fn source_address<'py>(
    py: Python<'py>,
    source: &Option<NpduAddress>,
) -> Option<Bound<'py, PyBytes>> {
    source
        .as_ref()
        .map(|address| PyBytes::new(py, &address.mac_address))
}

/// An I-Have this client received.
#[pyclass(name = "IHave", module = "rusty_bacnet", frozen)]
pub(crate) struct PyIHave(pub(crate) ReceivedIHave);

#[pymethods]
impl PyIHave {
    /// The device that has the object.
    #[getter]
    fn device_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.0.i_have.device_identifier)
    }

    /// The object it has.
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.0.i_have.object_identifier)
    }

    /// The object's name.
    #[getter]
    fn object_name(&self) -> &str {
        &self.0.i_have.object_name
    }

    /// The link-layer peer it came from: the device, or the router that
    /// passed it on.
    #[getter]
    fn source_mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.source_mac)
    }

    /// SNET, when a router passed it on from another network.
    #[getter]
    fn source_network(&self) -> Option<u16> {
        source_network(&self.0.source_network)
    }

    /// SADR, when a router passed it on from another network.
    #[getter]
    fn source_address<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        source_address(py, &self.0.source_network)
    }

    fn __repr__(&self) -> String {
        format!(
            "IHave(device={}, object={}, name={:?})",
            self.0.i_have.device_identifier.instance_number(),
            self.0.i_have.object_identifier,
            self.0.i_have.object_name
        )
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A router heard from during `discover_routers()`.
#[pyclass(name = "DiscoveredRouter", module = "rusty_bacnet", frozen)]
pub(crate) struct PyDiscoveredRouter(pub(crate) DiscoveredRouter);

#[pymethods]
impl PyDiscoveredRouter {
    /// The router's MAC on the local network: the next hop to `networks`.
    #[getter]
    fn mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.mac)
    }

    /// Every network its I-Am-Router-To-Network messages listed, ascending,
    /// each once.
    #[getter]
    fn networks(&self) -> Vec<u16> {
        self.0.networks.clone()
    }

    fn __repr__(&self) -> String {
        format!("DiscoveredRouter(networks={:?})", self.0.networks)
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A network-layer report this client received.
#[pyclass(name = "NetworkReport", module = "rusty_bacnet", frozen)]
pub(crate) struct PyNetworkReport(ReceivedNetworkReport);

#[pymethods]
impl PyNetworkReport {
    /// `"i_am_router_to_network"`, `"network_number_is"` or
    /// `"reject_message_to_network"`; `"other"` for a report this binding
    /// doesn't name yet.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.0.report {
            NetworkReport::IAmRouterToNetwork(_) => "i_am_router_to_network",
            NetworkReport::NetworkNumberIs { .. } => "network_number_is",
            NetworkReport::RejectMessageToNetwork(_) => "reject_message_to_network",
            _ => "other",
        }
    }

    /// The link-layer peer that sent it: for an I-Am-Router-To-Network,
    /// the router to send through.
    #[getter]
    fn source_mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.source_mac)
    }

    /// SNET, when a router passed it on from another network.
    #[getter]
    fn source_network(&self) -> Option<u16> {
        source_network(&self.0.source_network)
    }

    /// SADR, when a router passed it on from another network.
    #[getter]
    fn source_address<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        source_address(py, &self.0.source_network)
    }

    /// The networks an I-Am-Router-To-Network lists, in its order; `None`
    /// for the other kinds.
    #[getter]
    fn networks(&self) -> Option<Vec<u16>> {
        match &self.0.report {
            NetworkReport::IAmRouterToNetwork(networks) => Some(networks.clone()),
            _ => None,
        }
    }

    /// The local network's number for a Network-Number-Is, or the network
    /// a refused message was for; `None` for an I-Am-Router-To-Network.
    #[getter]
    fn network(&self) -> Option<u16> {
        match &self.0.report {
            NetworkReport::NetworkNumberIs { network, .. } => Some(*network),
            NetworkReport::RejectMessageToNetwork(reject) => Some(reject.network),
            _ => None,
        }
    }

    /// Whether a Network-Number-Is sender had its number configured rather
    /// than learned; `None` for the other kinds.
    #[getter]
    fn configured(&self) -> Option<bool> {
        match &self.0.report {
            NetworkReport::NetworkNumberIs { configured, .. } => Some(*configured),
            _ => None,
        }
    }

    /// A Reject-Message-To-Network's reason octet; `None` for the other
    /// kinds.
    #[getter]
    fn reason(&self) -> Option<u8> {
        match &self.0.report {
            NetworkReport::RejectMessageToNetwork(reject) => Some(reject.reason.to_raw()),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        format!("NetworkReport(kind='{}')", self.kind())
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Async iterator over one `i_have_events()` subscription.
#[pyclass(name = "IHaveIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyIHaveIterator(LossyReceiver<ReceivedIHave>);

impl PyIHaveIterator {
    pub(crate) fn new(rx: tokio::sync::broadcast::Receiver<ReceivedIHave>) -> Self {
        Self(LossyReceiver::new(rx))
    }
}

#[pymethods]
impl PyIHaveIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.0.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(PyIHave))
                    .ok_or_else(|| ended("I-Have events"))
            },
            put_back,
        )
    }

    /// I-Haves this iterator lost: because it fell more than 256 behind, or
    /// dropped on the way to a read, as when its event loop closed first.
    #[getter]
    fn missed(&self) -> u64 {
        self.0.missed()
    }

    /// Refuses `copy` and `pickle`: a live subscription. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Async iterator over one `network_reports()` subscription.
#[pyclass(name = "NetworkReportIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyNetworkReportIterator(LossyReceiver<ReceivedNetworkReport>);

impl PyNetworkReportIterator {
    pub(crate) fn new(rx: tokio::sync::broadcast::Receiver<ReceivedNetworkReport>) -> Self {
        Self(LossyReceiver::new(rx))
    }
}

#[pymethods]
impl PyNetworkReportIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.0.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(PyNetworkReport))
                    .ok_or_else(|| ended("network reports"))
            },
            put_back,
        )
    }

    /// Reports this iterator lost: because it fell more than 64 behind, or
    /// dropped on the way to a read, as when its event loop closed first.
    #[getter]
    fn missed(&self) -> u64 {
        self.0.missed()
    }

    /// Refuses `copy` and `pickle`: a live subscription. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyIHave>()?;
    m.add_class::<PyIHaveIterator>()?;
    m.add_class::<PyDiscoveredRouter>()?;
    m.add_class::<PyNetworkReport>()?;
    m.add_class::<PyNetworkReportIterator>()?;
    Ok(())
}
