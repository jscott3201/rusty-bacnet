//! The endpoint's client whose own stop fails (#2617), through the Python
//! wrapper: a stray reference to the low-level client, which the bindings
//! never hold, makes its stop fail as nothing else can. And the endpoint's
//! server, which each call borrows through the session lock: a close and a
//! call in flight, and a call while another thread holds the interpreter.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
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

/// Start a B/IP endpoint with AI 1 at 4.0 and take `server`, its
/// `BACnetServer` (#2617): the globals, and the host the server's calls go
/// through.
fn started_server(instance: u32) -> (Py<PyDict>, crate::server::ServerHost) {
    Python::initialize();
    let globals = Python::attach(|py| {
        let globals = PyDict::new(py);
        globals
            .set_item("BipEndpoint", py.get_type::<PyBipEndpoint>())
            .unwrap();
        globals.set_item("instance", instance).unwrap();
        globals.unbind()
    });
    run(
        &globals,
        pyo3::ffi::c_str!(
            r#"
import asyncio
async def begin():
    global endpoint, server
    endpoint = BipEndpoint(instance, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
    endpoint.add_analog_input(1, "AI 1", present_value=4.0)
    await endpoint.start()
    server = await endpoint.server()
asyncio.run(begin())
"#
        ),
    );
    let host = Python::attach(|py| {
        let server = globals.bind(py).get_item("server").unwrap().unwrap();
        server
            .cast::<crate::server::BACnetServer>()
            .unwrap()
            .borrow()
            .host_for_test()
            .clone()
    });
    (globals, host)
}

/// Close the endpoint `started_server` started.
fn close_endpoint(globals: &Py<PyDict>) {
    run(
        globals,
        pyo3::ffi::c_str!(
            "import asyncio\nasync def end():\n    await endpoint.close()\nasyncio.run(end())\n"
        ),
    );
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// AI 1's Present_Value, read through `host` as a server call reads it.
async fn read_ai1(host: &crate::server::ServerHost) -> PyResult<PropertyValue> {
    let guard = host.read().await;
    let server = guard.server()?;
    server
        .read_local(&ai1(), PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .map_err(crate::errors::to_py_err)
}

/// Wait up to ten seconds for `thread`, failing the test past that: a
/// thread that never ends is the deadlock these tests look for.
fn join_within<T>(thread: std::thread::JoinHandle<T>, what: &str) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !thread.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what} never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    thread.join().unwrap()
}

/// A server call and the endpoint's close race without a deadlock
/// (#2617): the close waits for the call in flight, as a standalone
/// server's `stop()` does, then stops the server, and the next call raises
/// at once rather than waiting.
#[test]
fn a_close_waits_for_a_server_call_in_flight_and_the_next_call_raises() {
    let (globals, host) = started_server(2619);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // A call in flight: it holds its guard, and reads.
    let in_flight = runtime.block_on(host.read());
    assert!(in_flight.server().is_ok());
    let closing = std::thread::spawn({
        let globals = Python::attach(|py| globals.clone_ref(py));
        move || close_endpoint(&globals)
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !closing.is_finished(),
        "the close waits for the call in flight"
    );
    // The call in flight keeps its server while the close waits.
    let value = runtime
        .block_on(in_flight.server().unwrap().read_local(
            &ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ))
        .unwrap();
    assert_eq!(value, PropertyValue::Real(4.0));
    drop(in_flight);
    join_within(closing, "the close");
    let error = runtime.block_on(read_ai1(&host)).unwrap_err();
    Python::attach(|py| {
        assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
        assert!(error.to_string().contains("came from is closed"), "{error}");
    });
}

/// A server call touches no Python (#2617): it runs to its end on another
/// thread while this one holds the interpreter; only the bindings'
/// conversion of its result attaches. A call that waited for the
/// interpreter would never finish here.
#[test]
fn a_server_call_runs_while_another_thread_holds_the_interpreter() {
    let (globals, host) = started_server(2620);
    Python::attach(|_py| {
        let call = std::thread::spawn({
            let host = host.clone();
            move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(read_ai1(&host))
                    .map_err(drop)
            }
        });
        assert_eq!(
            join_within(call, "the server call"),
            Ok(PropertyValue::Real(4.0))
        );
    });
    close_endpoint(&globals);
}

/// `endpoint.client()` shares the session lock with the server's calls
/// (#2617): it is handed out while a server call is in flight, rather than
/// waiting for it as it would behind a write lock.
#[test]
fn the_client_is_handed_out_beside_a_server_call_in_flight() {
    let (globals, host) = started_server(2621);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let in_flight = runtime.block_on(host.read());
    assert!(in_flight.server().is_ok());
    let asking = std::thread::spawn({
        let globals = Python::attach(|py| globals.clone_ref(py));
        move || {
            run(
                &globals,
                pyo3::ffi::c_str!(
                    "import asyncio\nasync def ask():\n    global client\n    client = await endpoint.client()\nasyncio.run(ask())\n"
                ),
            );
        }
    });
    join_within(asking, "endpoint.client() beside a server call");
    drop(in_flight);
    close_endpoint(&globals);
}
