//! The endpoint's client whose own stop fails (#2617), through the Python
//! wrapper: a stray reference to the low-level client, which the bindings
//! never hold, makes its stop fail as nothing else can.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::client::BACnetClient;
use crate::endpoint::PyBipEndpoint;

/// Run `script` with `globals`, failing the test on a Python error.
fn run(globals: &Py<PyDict>, script: &std::ffi::CStr) {
    Python::attach(|py| {
        if let Err(error) = py.run(script, Some(globals.bind(py)), None) {
            panic!("{error}");
        }
    });
}

/// A failed client stop leaves the cause where `endpoint.client()` reports
/// it, and the endpoint's close still stops and releases its link, saying
/// the client keeps its own state; once the reference goes, the client's
/// stop finishes.
#[test]
fn a_client_whose_stop_fails_names_the_cause_and_the_endpoint_still_closes() {
    Python::initialize();
    let globals = Python::attach(|py| {
        let globals = PyDict::new(py);
        globals
            .set_item("BipEndpoint", py.get_type::<PyBipEndpoint>())
            .unwrap();
        globals.unbind()
    });
    run(
        &globals,
        pyo3::ffi::c_str!(
            r#"
import asyncio
async def begin():
    global endpoint, client, port
    endpoint = BipEndpoint(2617, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    await endpoint.start()
    client = await endpoint.client()
    port = int((await endpoint.local_address()).rsplit(":", 1)[1])
asyncio.run(begin())
"#
        ),
    );
    let state = Python::attach(|py| {
        let client = globals.bind(py).get_item("client").unwrap().unwrap();
        client.cast::<BACnetClient>().unwrap().borrow().state()
    });
    let held = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(state.hold_core_for_test())
        .expect("started");
    run(
        &globals,
        pyo3::ffi::c_str!(
            r#"
async def failing():
    try:
        await client.stop()
    except Exception as error:
        assert "references remain" in str(error), repr(error)
    else:
        raise AssertionError("the stop succeeded")
    try:
        await endpoint.client()
    except RuntimeError as error:
        assert "failed to stop" in str(error), repr(error)
        assert "references remain" in str(error), repr(error)
    else:
        raise AssertionError("client() handed out another")
    try:
        await endpoint.close()
    except RuntimeError as error:
        assert "released its link" in str(error), repr(error)
        assert "references remain" in str(error), repr(error)
    else:
        raise AssertionError("the close hid the failure")
    # The link is released: another endpoint binds the same port.
    again = BipEndpoint(2618, interface="127.0.0.1", port=port, broadcast_address="127.0.0.1")
    await again.start()
    await again.close()
asyncio.run(failing())
"#
        ),
    );
    drop(held);
    run(
        &globals,
        pyo3::ffi::c_str!(
            r#"
async def finishing():
    await client.stop()
    try:
        await client.who_is()
    except RuntimeError as error:
        assert "ran on an endpoint" in str(error), repr(error)
    else:
        raise AssertionError("a stopped client sent")
asyncio.run(finishing())
"#
        ),
    );
}
