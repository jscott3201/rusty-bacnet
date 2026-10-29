//! Owned immutable B/IP construction inputs; parsing performs no I/O.
use crate::bip_options::{PyBbmd, PyForeignDevice, bip_mode, parse_ip};
use bacnet_transport::bip::{BipMode, BipTransport};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::net::Ipv4Addr;

#[derive(Clone)]
pub(super) struct PortConfig {
    pub network_number: u16,
    pub interface: Ipv4Addr,
    pub port: u16,
    pub broadcast: Ipv4Addr,
    pub mode: BipMode,
}

impl PortConfig {
    pub fn transport(&self) -> BipTransport {
        let mut transport = BipTransport::new(self.interface, self.port, self.broadcast);
        transport.set_mode(self.mode.clone());
        transport
    }
    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            BipMode::Normal => "normal",
            BipMode::Bbmd(_) => "bbmd",
            BipMode::Foreign(_) => "foreign_device",
        }
    }
}

/// Immutable configuration of one B/IP router port; construction performs no I/O.
#[derive(Clone)]
#[pyclass(
    name = "BipRouterPort",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyBipRouterPort(pub(super) PortConfig);

#[pymethods]
impl PyBipRouterPort {
    /// Configure one attached network. The default is normal B/IP; BBMD foreign
    /// device acceptance is enabled only by the supplied explicit Bbmd options.
    #[new]
    #[pyo3(signature = (network_number, *, interface="0.0.0.0", port=47808, broadcast_address="255.255.255.255", bbmd=None, foreign_device=None))]
    fn new(
        network_number: u16,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        bbmd: Option<PyRef<'_, PyBbmd>>,
        foreign_device: Option<PyRef<'_, PyForeignDevice>>,
    ) -> PyResult<Self> {
        if matches!(network_number, 0 | 65535) {
            return Err(PyValueError::new_err(
                "network_number must be between 1 and 65534",
            ));
        }
        Ok(Self(PortConfig {
            network_number,
            interface: parse_ip(interface, "interface")?,
            port,
            broadcast: parse_ip(broadcast_address, "broadcast_address")?,
            mode: bip_mode("bip", bbmd, foreign_device)?,
        }))
    }
    /// The directly attached network number.
    #[getter]
    fn network_number(&self) -> u16 {
        self.0.network_number
    }
    /// The configured IPv4 interface.
    #[getter]
    fn interface(&self) -> String {
        self.0.interface.to_string()
    }
    /// Configured UDP port; zero requests an actual ephemeral port at startup.
    #[getter]
    fn port(&self) -> u16 {
        self.0.port
    }
    /// The configured IPv4 broadcast address.
    #[getter]
    fn broadcast_address(&self) -> String {
        self.0.broadcast.to_string()
    }
    /// Normal B/IP, BBMD, or foreign-device mode.
    #[getter]
    fn mode(&self) -> &'static str {
        self.0.mode_name()
    }
    fn __repr__(&self) -> String {
        format!(
            "BipRouterPort(network_number={}, interface={:?}, port={}, mode={:?})",
            self.0.network_number,
            self.interface(),
            self.0.port,
            self.mode()
        )
    }
}
