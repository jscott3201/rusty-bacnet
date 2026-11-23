//! Owned immutable B/IP construction inputs; parsing performs no I/O.
use crate::bip_options::{PyBbmd, PyForeignDevice, bip_mode, parse_ip};
use bacnet_network::router::{ControlPolicy, RouterOptions};
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
    /// Kept apart from the router's other isolated ports (#2716).
    pub isolated: bool,
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
    ///
    /// `isolated=True` keeps the port apart from the router's other isolated
    /// ports (#2716): `RouterPort::isolated` in Rust. It limits transit
    /// between isolated ports and is no trust level for the port.
    #[new]
    #[pyo3(signature = (network_number, *, interface="0.0.0.0", port=47808, broadcast_address="255.255.255.255", bbmd=None, foreign_device=None, isolated=false))]
    #[expect(
        clippy::too_many_arguments,
        reason = "the Python signature: one positional and six keyword-only options"
    )]
    fn new(
        py: Python<'_>,
        network_number: u16,
        interface: &str,
        port: u16,
        broadcast_address: &str,
        bbmd: Option<PyRef<'_, PyBbmd>>,
        foreign_device: Option<PyRef<'_, PyForeignDevice>>,
        isolated: bool,
    ) -> PyResult<Self> {
        if matches!(network_number, 0 | 65535) {
            return Err(PyValueError::new_err(
                "network_number must be between 1 and 65534",
            ));
        }
        Ok(Self(PortConfig {
            network_number,
            interface: crate::bip_options::interface_ipv4(py, interface)?,
            port,
            broadcast: parse_ip(broadcast_address, "broadcast_address")?,
            mode: bip_mode("bip", bbmd, foreign_device)?,
            isolated,
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
    /// Whether the router keeps this port apart from its other isolated
    /// ports.
    #[getter]
    fn isolated(&self) -> bool {
        self.0.isolated
    }
    fn __repr__(&self) -> String {
        format!(
            "BipRouterPort(network_number={}, interface={:?}, port={}, mode={:?}, isolated={})",
            self.0.network_number,
            self.interface(),
            self.0.port,
            self.mode(),
            if self.0.isolated { "True" } else { "False" }
        )
    }
}

/// The router's own options (#2725), `RouterOptions` in Rust: the keywords
/// `remote_table_updates`, `next_hop_rejects` and `control_policy`, fixed
/// at construction and applied at each start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RouterSettings {
    /// Take routing-table updates that arrive from another network.
    pub remote_table_updates: bool,
    /// Apply a reject to a learned route only from its next router.
    pub next_hop_rejects: bool,
    /// The wire-control policy.
    pub control_policy: ControlPolicy,
}

impl RouterSettings {
    /// The keywords, checked: `control_policy` is `"permissive"` or
    /// `"hardened"`, anything else ValueError. With `"hardened"` and no
    /// authorizer, which Python can't give, the router denies every
    /// routing-table update and every reject, so `remote_table_updates` or
    /// `next_hop_rejects` beside it could never apply: ValueError too.
    pub fn parse(
        remote_table_updates: bool,
        next_hop_rejects: bool,
        control_policy: &str,
    ) -> PyResult<Self> {
        let control_policy = match control_policy {
            "permissive" => ControlPolicy::Permissive,
            "hardened" => ControlPolicy::Hardened,
            other => {
                return Err(PyValueError::new_err(format!(
                    "control_policy must be 'permissive' or 'hardened', not {other:?}"
                )));
            }
        };
        let idle = match (remote_table_updates, next_hop_rejects) {
            (true, _) => Some(("remote_table_updates", "routing-table update")),
            (_, true) => Some(("next_hop_rejects", "reject")),
            _ => None,
        };
        if let (ControlPolicy::Hardened, Some((option, what))) = (control_policy, idle) {
            return Err(PyValueError::new_err(format!(
                "{option}=True does nothing with control_policy='hardened': with no \
                 authorizer the router denies every {what}"
            )));
        }
        Ok(Self {
            remote_table_updates,
            next_hop_rejects,
            control_policy,
        })
    }

    /// The `control_policy` keyword's value.
    pub fn policy_name(&self) -> &'static str {
        match self.control_policy {
            ControlPolicy::Permissive => "permissive",
            ControlPolicy::Hardened => "hardened",
        }
    }

    /// These settings as the options `BACnetRouter::start` takes.
    pub fn router_options(&self) -> RouterOptions {
        let mut options = RouterOptions::new().control_policy(self.control_policy);
        if self.remote_table_updates {
            options = options.remote_table_updates();
        }
        if self.next_hop_rejects {
            options = options.next_hop_rejects();
        }
        options
    }
}
