//! Frozen owned snapshots; none retain a router, table guard, or socket.
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::types::{IntEnumValue, int_enums};

/// Actual address of one started B/IP router port.
#[pyclass(name = "RouterPortInfo", module = "rusty_bacnet", frozen)]
pub(crate) struct PyRouterPortInfo {
    pub(super) index: u8,
    pub(super) network_number: u16,
    pub(super) mac: Vec<u8>,
}
#[pymethods]
impl PyRouterPortInfo {
    /// Zero-based configured port index.
    #[getter]
    fn index(&self) -> u8 {
        self.index
    }
    /// The directly attached network.
    #[getter]
    fn network_number(&self) -> u16 {
        self.network_number
    }
    /// Actual six-byte post-start B/IP MAC, copied from the native router.
    #[getter]
    fn mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.mac)
    }
    /// Actual IPv4 address and UDP port, including the assigned ephemeral port.
    #[getter]
    fn address(&self) -> String {
        format!(
            "{}.{}.{}.{}:{}",
            self.mac[0],
            self.mac[1],
            self.mac[2],
            self.mac[3],
            u16::from_be_bytes([self.mac[4], self.mac[5]])
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "RouterPortInfo(index={}, network_number={}, address={:?})",
            self.index,
            self.network_number,
            self.address()
        )
    }
}

/// One copied routing-table row, with reachability evaluated at capture time.
#[pyclass(name = "RouterRoute", module = "rusty_bacnet", frozen)]
pub(crate) struct PyRouterRoute {
    pub(super) network_number: u16,
    pub(super) port_index: u8,
    pub(super) direct: bool,
    pub(super) next_hop: Option<Vec<u8>>,
    pub(super) reachability: &'static str,
}
#[pymethods]
impl PyRouterRoute {
    /// Destination BACnet network number.
    #[getter]
    fn network_number(&self) -> u16 {
        self.network_number
    }
    /// Zero-based egress index into the configured ports.
    #[getter]
    fn port_index(&self) -> u8 {
        self.port_index
    }
    /// Whether the network is directly attached.
    #[getter]
    fn is_direct(&self) -> bool {
        self.direct
    }
    /// Copied next-router MAC for a learned route; None for a direct route.
    #[getter]
    fn next_hop<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.next_hop.as_ref().map(|mac| PyBytes::new(py, mac))
    }
    /// Effective state at capture: reachable, busy, or unreachable.
    #[getter]
    fn reachability(&self) -> &'static str {
        self.reachability
    }
    fn __repr__(&self) -> String {
        format!(
            "RouterRoute(network_number={}, port_index={}, is_direct={}, reachability={:?})",
            self.network_number, self.port_index, self.direct, self.reachability
        )
    }
}

/// One retained router cleanup failure, copied without consuming its Rust cause.
#[pyclass(name = "RouterCleanupFailure", module = "rusty_bacnet", frozen)]
pub(crate) struct PyRouterCleanupFailure {
    #[pyo3(get)]
    pub(super) network_number: Option<u16>,
    #[pyo3(get)]
    pub(super) port_index: Option<usize>,
    #[pyo3(get)]
    pub(super) kind: &'static str,
    #[pyo3(get)]
    pub(super) message: String,
    #[pyo3(get)]
    pub(super) errno: Option<i32>,
    pub(super) error_class: Option<ErrorClass>,
    pub(super) error_code: Option<ErrorCode>,
    pub(super) reason: Option<RemoteReason>,
    #[pyo3(get)]
    pub(super) worker: Option<&'static str>,
}

/// A remote Reject's or Abort's reason, kept with its kind so Python gets a
/// member of the right class.
#[derive(Clone, Copy)]
pub(super) enum RemoteReason {
    Reject(RejectReason),
    Abort(AbortReason),
}

#[pymethods]
impl PyRouterCleanupFailure {
    /// The remote error's class, an `ErrorClass` member, else None.
    #[getter]
    fn error_class(&self) -> Option<IntEnumValue<ErrorClass>> {
        self.error_class.map(IntEnumValue)
    }
    /// The remote error's code, an `ErrorCode` member, else None.
    #[getter]
    fn error_code(&self) -> Option<IntEnumValue<ErrorCode>> {
        self.error_code.map(IntEnumValue)
    }
    /// A remote Reject's `RejectReason` or Abort's `AbortReason`, else None.
    #[getter]
    fn reason<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.reason
            .map(|reason| match reason {
                RemoteReason::Reject(reason) => int_enums::to_py(py, reason),
                RemoteReason::Abort(reason) => int_enums::to_py(py, reason),
            })
            .transpose()
    }
}
