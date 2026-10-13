//! Python bindings for rusty-bacnet via PyO3.

// Each #[pyo3(signature = ...)] method mirrors its Python keyword arguments, so
// argument count is part of the Python API; a Rust parameter struct would
// change what Python callers write (#902).
#![allow(clippy::too_many_arguments)]

use pyo3::prelude::*;

mod bip_options;
mod client;
mod endpoint;
mod errors;
mod group_members;
mod hub;
mod hub_bindings;
mod log_records;
mod mstp_py;
mod object_audit_policy;
mod py_async;
mod read_range;
mod sc_identity;
mod server;
mod tls;
mod types;

/// Project a unit-only async success to Python None, after the operation future
/// (and its Rust guards) has finished. PyO3's IntoPyObject maps Rust () to tuple(),
/// unlike synchronous pymethod unit returns. Preserve the original PyErr on failure.
async fn unit_result(
    future: impl std::future::Future<Output = PyResult<()>>,
) -> PyResult<Py<PyAny>> {
    future.await?;
    py_async::attach(|py| Ok(py.None()))
}

/// The `rusty_bacnet` Python module.
#[pymodule]
fn rusty_bacnet(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Keep binding threads out of the interpreter once it starts exiting (#1002).
    py_async::register(m)?;

    // Register exception types
    errors::register(m)?;

    // Register type wrappers
    types::register(m)?;

    // Register client and server classes
    m.add_class::<client::BACnetClient>()?;
    m.add_class::<server::BACnetServer>()?;
    m.add_class::<hub::PyScHub>()?;
    m.add_class::<hub_bindings::PyScHubCertificateBinding>()?;
    m.add_class::<bip_options::PyForeignDevice>()?;
    m.add_class::<bip_options::PyBbmd>()?;
    endpoint::register(m)?;
    m.add_function(wrap_pyfunction!(mstp_py::list_serial_ports, m)?)?;
    m.add_function(wrap_pyfunction!(log_records::decode_log_records, m)?)?;

    Ok(())
}
