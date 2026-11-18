//! Python endpoint owners: one transport above both roles (RB-19).
//!
//! - `BipEndpoint`: one B/IP UDP socket.
//! - `ScEndpoint`: one SC hub connection (single UUID for transport+identity).
//! - `MstpEndpoint`: one serial owner.
//! - `endpoint.client()` is a `BACnetClient` on the endpoint's link, and
//!   `endpoint.server()` a `BACnetServer` on the full server the session
//!   owns (#2617); the endpoint starts and stops both.
//!
//! BIPv6/Ethernet have no endpoint owner by design; use the standalone
//! `BACnetClient`/`BACnetServer` path there.

use pyo3::types::PyModuleMethods;

mod bip;
mod builders;
mod common;
mod lifecycle;
mod mstp;
mod running;
mod sc;
mod server_role;
mod source_audit;
mod table_reads;

pub(crate) use bip::PyBipEndpoint;
pub(crate) use mstp::PyMstpEndpoint;
pub(crate) use running::Running;
pub(crate) use sc::PyScEndpoint;

/// Register endpoint classes with the module.
pub fn register(m: &pyo3::Bound<'_, pyo3::types::PyModule>) -> pyo3::PyResult<()> {
    m.add_class::<PyBipEndpoint>()?;
    m.add_class::<PyScEndpoint>()?;
    m.add_class::<PyMstpEndpoint>()?;
    // A request's target behind a router, on `BACnetClient`'s typed calls
    // (#2612, #2617).
    m.add_class::<crate::types::PyRoutedTarget>()?;
    Ok(())
}
