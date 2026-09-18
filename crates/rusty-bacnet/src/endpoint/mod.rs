//! Python endpoint owners: one transport above both roles (RB-19).
//!
//! - `BipEndpoint`: one B/IP UDP socket.
//! - `ScEndpoint`: one SC hub connection (single UUID for transport+identity).
//! - `MstpEndpoint`: one serial owner.
//! - `EndpointClient` / `EndpointServer`: cloned role handles, no lifecycle.
//!
//! BIPv6/Ethernet have no endpoint owner by design; use the standalone
//! `BACnetClient`/`BACnetServer` path there.

use pyo3::types::PyModuleMethods;

mod bip;
mod common;
mod mstp;
mod roles;
mod sc;

pub(crate) use bip::PyBipEndpoint;
pub(crate) use mstp::PyMstpEndpoint;
pub(crate) use roles::{PyEndpointClient, PyEndpointServer};
pub(crate) use sc::PyScEndpoint;

/// Register endpoint classes with the module.
pub fn register(m: &pyo3::Bound<'_, pyo3::types::PyModule>) -> pyo3::PyResult<()> {
    m.add_class::<PyBipEndpoint>()?;
    m.add_class::<PyScEndpoint>()?;
    m.add_class::<PyMstpEndpoint>()?;
    m.add_class::<PyEndpointClient>()?;
    m.add_class::<PyEndpointServer>()?;
    Ok(())
}
