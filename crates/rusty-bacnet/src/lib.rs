//! Python bindings for rusty-bacnet via PyO3.

// Each #[pyo3(signature = ...)] method mirrors its Python keyword arguments, so
// argument count is part of the Python API; a Rust parameter struct would
// change what Python callers write (#902).
#![allow(clippy::too_many_arguments)]
#![forbid(unsafe_code)]

use pyo3::prelude::*;

mod bip_options;
mod client;
mod discovery;
mod endpoint;
mod errors;
mod event_stream;
mod group_members;
mod hub;
mod hub_bindings;
mod log_records;
mod logging_bridge;
mod mstp_py;
mod object_audit_policy;
mod object_options;
mod py_async;
mod read_range;
mod reinitialize;
mod sc_events;
mod sc_identity;
mod sc_inspection;
mod sc_npdu;
mod sc_peer;
mod sc_reconnect;
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
    // `logging` first, so that its atexit shutdown runs after the gate's hook.
    logging_bridge::import_logging(m.py())?;
    // Keep binding threads out of the interpreter once it starts exiting (#1002).
    py_async::register(m)?;
    // Forward the stack's tracing events to `logging` (#1724), and flush them
    // at exit before the gate closes.
    logging_bridge::register(m)?;

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
    m.add_class::<bip_options::PyMulticast>()?;
    m.add_class::<sc_reconnect::PyScReconnect>()?;
    sc_peer::register(m)?;
    sc_events::register(m)?;
    discovery::register(m)?;
    sc_inspection::register(m)?;
    endpoint::register(m)?;
    reinitialize::register(m)?;
    m.add_function(wrap_pyfunction!(mstp_py::list_serial_ports, m)?)?;
    m.add_function(wrap_pyfunction!(log_records::decode_log_records, m)?)?;

    Ok(())
}
