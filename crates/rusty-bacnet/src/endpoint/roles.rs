//! The server role's Python handle above the endpoint's Rust roles.
//!
//! `EndpointServer` holds a cloned `ServerRoleHandle` and exposes no
//! lifecycle: start/stop/close exist only on the owning endpoint
//! (`BipEndpoint` / `ScEndpoint` / `MstpEndpoint`). After the owner closes
//! or drops, every call fails closed via `BacnetError` ("endpoint
//! shutdown"). The client role is a `BACnetClient` on the endpoint's link
//! (#2617), from `endpoint.client()`.
//!
//! The role exposes no callbacks; `is_session_alive` polling is the same
//! poll pattern as COV iterators. The only Python callable a native lock
//! waits on is the owner's `on_reinitialize` (see [`crate::reinitialize`]);
//! the owner's `device_write_policy` runs with no lock held (see
//! [`crate::mutation_policy`]).

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::errors::to_py_err;

/// Server role: the owner's responder liveness + deferred-reply seam.
///
/// The responder executes ReadProperty automatically; this handle exposes
/// no request callback (poll via `is_session_alive`).
/// `suspend_next_reply` arms the one-shot MS/TP deferred-reply path.
#[pyclass(
    name = "EndpointServer",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub struct PyEndpointServer {
    inner: bacnet_endpoint::roles::ServerRoleHandle,
    /// Whether the endpoint was built with `on_reinitialize` (#1566).
    reinitialize: bool,
    /// Whether the endpoint was built with `device_write_policy` (#2033).
    device_writes: bool,
    /// Whether the endpoint was built with `cov` (#2490).
    cov: bool,
}

impl PyEndpointServer {
    pub(crate) fn new(
        inner: bacnet_endpoint::roles::ServerRoleHandle,
        reinitialize: bool,
        device_writes: bool,
        cov: bool,
    ) -> Self {
        Self {
            inner,
            reinitialize,
            device_writes,
            cov,
        }
    }

    /// The services the server role executes.
    fn executes(&self) -> Vec<&'static str> {
        let mut services = vec!["read_property"];
        if self.device_writes {
            services.push("write_property");
        }
        if self.reinitialize {
            services.push("reinitialize_device");
        }
        if self.cov {
            services.extend([
                "subscribe_cov",
                "subscribe_cov_property",
                "subscribe_cov_property_multiple",
            ]);
        }
        services
    }
}

#[pymethods]
impl PyEndpointServer {
    /// Returns true while the owning endpoint is alive and open.
    ///
    /// Cloned handles survive the owner as values but report false and fail
    /// closed after stop/drop.
    fn is_session_alive(&self) -> bool {
        self.inner.is_session_alive()
    }

    /// Arms one-shot deferred-reply suspension (MS/TP ReplyPostponed wiring).
    ///
    /// The next `reply_tx`-bearing inbound request answers token-owned via
    /// egress instead of promptly. Fails closed with `BacnetError` after
    /// owner close. Broadcasts never consume the arm.
    fn suspend_next_reply(&self) -> PyResult<()> {
        self.inner.suspend_next_reply().map_err(to_py_err)
    }

    /// Narrow service scope: the endpoint server executes ReadProperty,
    /// WriteProperty to its local Device when the endpoint was built with
    /// `device_write_policy`, ReinitializeDevice when it was built with
    /// `on_reinitialize`, and the three COV subscribe services when it was
    /// built with `cov`.
    fn service_scope<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(py);
        dict.set_item("initiates", Vec::<String>::new())?;
        dict.set_item("executes", self.executes())?;
        Ok(dict.into_any())
    }

    fn __repr__(&self) -> String {
        format!("EndpointServer({} responder)", self.executes().join(" "))
    }

    /// Refuses `copy` and `pickle`: a handle on a live endpoint. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
