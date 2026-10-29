use super::*;
use pyo3::exceptions::PyValueError;
use pyo3::types::PyDict;
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

#[test]
fn exit_gate_drains_attached_threads_then_refuses_entry() {
    let gate = ExitGate::new();
    let pass = gate.enter().expect("an open gate admits");
    gate.close();
    assert!(gate.is_closed());
    assert!(gate.enter().is_none(), "a closed gate admits nobody");
    let (drained_tx, drained_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            gate.drain();
            drained_tx.send(()).unwrap();
        });
        assert!(
            drained_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "drain returned while a thread was still attached"
        );
        drop(pass);
        drained_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("drain returns once the last pass is dropped");
    });
}

#[test]
fn attach_through_a_closed_gate_fails_without_attaching() {
    Python::initialize();
    let gate = ExitGate::new();
    gate.close();
    let err = attach_through(&gate, |_| -> PyResult<()> {
        unreachable!("attached through a closed gate")
    })
    .unwrap_err();
    Python::attach(|py| {
        assert!(err.is_instance_of::<PyRuntimeError>(py));
        assert_eq!(
            err.value(py).to_string(),
            "the Python interpreter is exiting"
        );
    });
}

#[pyfunction]
fn resolve(py: Python<'_>, value: i64) -> PyResult<Bound<'_, PyAny>> {
    future_into_py(py, async move {
        if value < 0 {
            Err(PyValueError::new_err("negative"))
        } else {
            Ok(value + 1)
        }
    })
}

async fn explode() -> PyResult<()> {
    panic!("bridge test panic")
}

#[pyfunction]
fn panicking(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    future_into_py(py, explode())
}

/// Sets its flag when the Rust future that owns it is dropped.
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[pyclass]
struct DropProbe(Arc<AtomicBool>);

#[pymethods]
impl DropProbe {
    fn pending<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let guard = SetOnDrop(self.0.clone());
        future_into_py(py, async move {
            let _guard = guard;
            std::future::pending::<PyResult<()>>().await
        })
    }

    fn dropped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn run(code: &CStr, globals: impl FnOnce(&Bound<'_, PyDict>) -> PyResult<()>) {
    Python::initialize();
    Python::attach(|py| {
        let scope = PyDict::new(py);
        globals(&scope).unwrap();
        if let Err(err) = py.run(code, Some(&scope), None) {
            err.print(py);
            panic!("Python check failed: {err}");
        }
    });
}

#[test]
fn futures_resolve_to_results_and_exceptions() {
    run(
        cr#"
import asyncio
async def main():
    future = resolve(41)
    assert asyncio.isfuture(future)
    assert await future == 42
    try:
        await resolve(-1)
    except ValueError as error:
        assert str(error) == "negative"
    else:
        raise AssertionError("no ValueError")
asyncio.run(main())
"#,
        |scope| scope.set_item("resolve", wrap_pyfunction!(resolve, scope.py())?),
    );
}

#[test]
fn a_panicking_future_raises_panic_exception() {
    run(
        cr#"
import asyncio
async def main():
    try:
        await panicking()
    except BaseException as error:
        assert type(error).__name__ == "PanicException", repr(error)
        assert "bridge test panic" in str(error), str(error)
    else:
        raise AssertionError("no PanicException")
asyncio.run(main())
"#,
        |scope| scope.set_item("panicking", wrap_pyfunction!(panicking, scope.py())?),
    );
}

#[test]
fn cancelling_the_asyncio_future_drops_the_rust_future() {
    run(
        cr#"
import asyncio
async def main():
    future = probe.pending()
    await asyncio.sleep(0)
    future.cancel()
    try:
        await future
    except asyncio.CancelledError:
        pass
    else:
        raise AssertionError("not cancelled")
    for _ in range(500):
        if probe.dropped():
            return
        await asyncio.sleep(0.01)
    raise AssertionError("the Rust future outlived its cancelled asyncio future")
asyncio.run(main())
"#,
        |scope| {
            let probe = DropProbe(Arc::new(AtomicBool::new(false)));
            scope.set_item("probe", Py::new(scope.py(), probe)?)
        },
    );
}
