//! Python bindings for rusty-bacnet via PyO3.

use pyo3::prelude::*;

mod client;
mod errors;
mod hub;
mod server;
mod tls;
mod types;

/// The `rusty_bacnet` Python module.
#[pymodule]
fn rusty_bacnet(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register exception types
    errors::register(m)?;

    // Register type wrappers
    types::register(m)?;

    // Register client and server classes
    m.add_class::<client::BACnetClient>()?;
    m.add_class::<server::BACnetServer>()?;
    m.add_class::<hub::PyScHub>()?;

    Ok(())
}
