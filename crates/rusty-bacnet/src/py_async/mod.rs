//! The bridge from Tokio futures to asyncio futures, and the exit gate that
//! keeps binding threads out of a finalizing interpreter (#1002).
//!
//! Binding futures run on this module's Tokio runtime, and its threads touch
//! Python only through [`attach`]: to build a result, and to complete the
//! asyncio future. An `atexit` hook, which CPython runs before it starts
//! finalizing, closes the gate and waits, with the GIL released, until no
//! binding thread is attached. After that a binding thread leaves Python alone
//! and drops what it holds (PyO3 defers those decrefs), so once finalization
//! begins no binding thread is inside the interpreter or about to re-enter it.
//!
//! Checking for finalization when attaching (`Python::try_attach`) is not
//! enough. Completing an asyncio future from another thread calls
//! `loop.call_soon_threadsafe`, which queues the callback and then writes the
//! loop's self-pipe with the GIL released. In that window the loop thread can
//! run the callback, finish the program and start finalizing. CPython 3.12 and
//! 3.13 end a non-main thread that takes the GIL back during finalization with
//! `pthread_exit`, and its forced unwind drops the thread's Python references
//! without the GIL while the interpreter is torn down: a segfault at exit.

use std::any::Any;
use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

use pyo3::exceptions::PyRuntimeError;
use pyo3::panic::PanicException;
use pyo3::prelude::*;
use pyo3::{IntoPyObjectExt, intern};
use tokio::runtime::Runtime;
use tokio::task::AbortHandle;

#[cfg(test)]
mod tests;

static GATE: ExitGate = ExitGate::new();

/// Register the `atexit` hook that closes the exit gate.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let hook = wrap_pyfunction!(close_exit_gate, m)?;
    py.import(intern!(py, "atexit"))?
        .call_method1(intern!(py, "register"), (hook,))?;
    Ok(())
}

/// `atexit` hook: close the gate while holding the GIL, so no Python thread
/// that runs after it can start a future, then wait with the GIL released so
/// that attached binding threads can finish.
#[pyfunction]
fn close_exit_gate(py: Python<'_>) {
    GATE.close();
    py.detach(|| GATE.drain());
}

/// Attach a binding thread to the interpreter for `f`, unless the interpreter
/// is exiting. Every Python access off the Python threads goes through here.
pub(crate) fn attach<R>(f: impl for<'py> FnOnce(Python<'py>) -> PyResult<R>) -> PyResult<R> {
    attach_through(&GATE, f)
}

fn attach_through<R>(
    gate: &ExitGate,
    f: impl for<'py> FnOnce(Python<'py>) -> PyResult<R>,
) -> PyResult<R> {
    // The pass is taken before and returned after attaching, so the gate's
    // lock is never held while waiting for the GIL.
    let _pass = gate.enter().ok_or_else(exiting)?;
    Python::try_attach(f).unwrap_or_else(|| Err(exiting()))
}

fn exiting() -> PyErr {
    PyRuntimeError::new_err("the Python interpreter is exiting")
}

thread_local! {
    /// Set while this thread runs a Python callback inside a server's
    /// dispatch: a ReinitializeDevice handler (#1566), which a native lock
    /// waits on, or a `mutation_policy` or `device_write_policy` callable.
    static UNDER_NATIVE_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f`, a synchronous Python callback called inside a server's dispatch,
/// with [`future_into_py`] refused on this thread. Under a native lock (a
/// ReinitializeDevice handler), a binding operation the callback started
/// would wait for that same lock, and one it then blocked on (with
/// `asyncio.run`, say) would never finish. The policy callables run with no
/// lock held but keep the same refusal.
pub(crate) fn under_native_lock<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UNDER_NATIVE_LOCK.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(UNDER_NATIVE_LOCK.with(|flag| flag.replace(true)));
    f()
}

/// The Tokio runtime binding futures run on. It is never shut down: the exit
/// gate, not the runtime, keeps its threads out of a finalizing interpreter.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build the Tokio runtime for the Python bindings")
    })
}

/// Schedule pure Rust best-effort destruction on the shared binding runtime.
/// The cleanup future must never attach Python or block its dropping thread.
pub(crate) fn spawn_cleanup(future: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(future);
}

/// Run `fut` on the bindings' runtime and return an asyncio future, on the
/// running event loop, that resolves to its output.
///
/// Once the asyncio future is done, by cancellation or otherwise, the Rust
/// future is aborted. A panic resolves it with `PanicException`, as a panic in
/// a synchronous method raises. Once the interpreter is exiting this raises
/// `RuntimeError`, because nothing would complete the future.
pub(crate) fn future_into_py<F, T>(py: Python<'_>, fut: F) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: for<'py> IntoPyObject<'py> + Send + 'static,
{
    spawn_into_py(py, fut, None)
}

/// Takes back a value whose asyncio future was already done, cancelled
/// most often, when the value reached it (#1858). An event iterator keeps
/// it for its next `__anext__`, so a cancelled wait loses no item.
pub(crate) trait PutBack: Send + Sync + 'static {
    /// Called on the thread that found the future done, attached to the
    /// interpreter.
    fn put_back(&self, py: Python<'_>, value: Py<PyAny>, via: Via);

    /// Called once a value has reached its future or gone to `put_back`,
    /// so the iterator can hand out the next one in order.
    fn settled(&self);

    /// Called once the `set_result` callback is queued on the loop.
    #[cfg(test)]
    fn queued(&self) {}
}

/// Where a value that could no longer reach its future was handed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    /// The binding thread found the future already done.
    BeforeDelivery,
    /// The `set_result` callback found it done on the loop.
    OnLoop,
    /// The loop was closed, so no callback could be queued.
    ClosedLoop,
}

/// [`future_into_py`], except that a successful value that finds its
/// asyncio future already done goes to `put_back` instead of being dropped.
/// That happens when the future is cancelled after the Rust future finished
/// but before the event loop set the result: while the value waits for the
/// GIL, or while its `set_result` callback is queued on the loop.
pub(crate) fn future_into_py_or_put_back<F, T>(
    py: Python<'_>,
    fut: F,
    put_back: Arc<dyn PutBack>,
) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: for<'py> IntoPyObject<'py> + Send + 'static,
{
    spawn_into_py(py, fut, Some(put_back))
}

fn spawn_into_py<F, T>(
    py: Python<'_>,
    fut: F,
    put_back: Option<Arc<dyn PutBack>>,
) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: for<'py> IntoPyObject<'py> + Send + 'static,
{
    if GATE.is_closed() {
        return Err(exiting());
    }
    if UNDER_NATIVE_LOCK.with(std::cell::Cell::get) {
        return Err(PyRuntimeError::new_err(
            "a server callback (on_reinitialize, mutation_policy, device_write_policy) \
             cannot start a binding operation",
        ));
    }
    let event_loop = py
        .import(intern!(py, "asyncio"))?
        .call_method0(intern!(py, "get_running_loop"))?;
    let future = event_loop.call_method0(intern!(py, "create_future"))?;
    let task = runtime().spawn(fut);
    if let Err(err) = future.call_method1(
        intern!(py, "add_done_callback"),
        (AbortOnDone(task.abort_handle()),),
    ) {
        task.abort();
        return Err(err);
    }
    let completion = Completion {
        event_loop: event_loop.unbind(),
        future: future.clone().unbind(),
        put_back,
    };
    runtime().spawn(async move {
        let result = match task.await {
            Ok(result) => result,
            Err(error) => match error.try_into_panic() {
                Ok(payload) => Err(panic_error(payload)),
                // Aborted: the asyncio future is already done.
                Err(_) => return,
            },
        };
        // On a blocking thread, waiting for the GIL never stalls a runtime worker.
        tokio::task::spawn_blocking(move || completion.deliver(result));
    });
    Ok(future)
}

fn panic_error(payload: Box<dyn Any + Send>) -> PyErr {
    let message = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic from Rust code".to_owned()
    };
    PanicException::new_err(message)
}

/// Where a finished Rust future's result goes.
struct Completion {
    event_loop: Py<PyAny>,
    future: Py<PyAny>,
    /// Takes back a successful value the future can no longer take.
    put_back: Option<Arc<dyn PutBack>>,
}

impl Completion {
    /// Hand `result` to the event loop through the exit gate. With the gate
    /// closed nobody is waiting, and the result is dropped.
    fn deliver<T>(self, result: PyResult<T>)
    where
        T: for<'py> IntoPyObject<'py>,
    {
        let _ = attach(|py| {
            if let Err(err) = self.schedule(py, result) {
                err.write_unraisable(py, Some(self.future.bind(py)));
            }
            Ok(())
        });
    }

    fn schedule<T>(&self, py: Python<'_>, result: PyResult<T>) -> PyResult<()>
    where
        T: for<'py> IntoPyObject<'py>,
    {
        let future = self.future.bind(py);
        let done = future.call_method0(intern!(py, "done"))?.is_truthy()?;
        // Only a value goes back; an error has nobody left to raise to.
        let (setter, value, put_back) = match result.and_then(|value| value.into_bound_py_any(py)) {
            Ok(value) => (intern!(py, "set_result"), value, self.put_back.clone()),
            Err(_) if done => return Ok(()),
            Err(err) => (
                intern!(py, "set_exception"),
                err.into_value(py).into_bound(py).into_any(),
                None,
            ),
        };
        if done {
            if let Some(put_back) = put_back {
                put_back.put_back(py, value.unbind(), Via::BeforeDelivery);
                put_back.settled();
            }
            return Ok(());
        }
        let setter = future.getattr(setter)?;
        let event_loop = self.event_loop.bind(py);
        if let Err(err) = event_loop.call_method1(
            intern!(py, "call_soon_threadsafe"),
            (SetUnlessDone(put_back.clone()), future, setter, &value),
        ) {
            // A closed loop has nobody left to wake.
            if !event_loop
                .call_method0(intern!(py, "is_closed"))?
                .is_truthy()?
            {
                return Err(err);
            }
            if let Some(put_back) = put_back {
                put_back.put_back(py, value.unbind(), Via::ClosedLoop);
                put_back.settled();
            }
            return Ok(());
        }
        #[cfg(test)]
        if let Some(put_back) = &put_back {
            put_back.queued();
        }
        Ok(())
    }
}

/// Runs on the event loop: set the result unless the future was cancelled
/// while the result was on its way; a value it can no longer take goes to
/// the put-back hook, when there is one. Either way the hook then hears that
/// the value settled; one that errors out settles when the hook drops.
#[pyclass(frozen)]
struct SetUnlessDone(Option<Arc<dyn PutBack>>);

#[pymethods]
impl SetUnlessDone {
    fn __call__(
        &self,
        future: &Bound<'_, PyAny>,
        setter: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let py = future.py();
        if !future.call_method0(intern!(py, "done"))?.is_truthy()? {
            setter.call1((value,))?;
        } else if let Some(put_back) = &self.0 {
            put_back.put_back(py, value.clone().unbind(), Via::OnLoop);
        }
        if let Some(put_back) = &self.0 {
            put_back.settled();
        }
        Ok(())
    }
}

/// Done callback on the asyncio future: once it is done nobody wants the Rust
/// future's output, so abort it. Aborting a finished task does nothing.
#[pyclass(frozen)]
struct AbortOnDone(AbortHandle);

#[pymethods]
impl AbortOnDone {
    fn __call__(&self, _future: &Bound<'_, PyAny>) {
        self.0.abort();
    }
}

/// Counts binding threads that may be attached to the interpreter and, once
/// closed, admits no more.
struct ExitGate {
    state: Mutex<GateState>,
    idle: Condvar,
}

struct GateState {
    closed: bool,
    attached: usize,
}

impl ExitGate {
    const fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                closed: false,
                attached: 0,
            }),
            idle: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A pass that lets its holder attach, or `None` once the gate is closed.
    fn enter(&self) -> Option<Pass<'_>> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        state.attached += 1;
        Some(Pass(self))
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// Admit no more passes.
    fn close(&self) {
        self.lock().closed = true;
    }

    /// Wait until every outstanding pass is dropped.
    fn drain(&self) {
        let mut state = self.lock();
        while state.attached > 0 {
            state = self
                .idle
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Held while a binding thread may be attached.
struct Pass<'a>(&'a ExitGate);

impl Drop for Pass<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.attached -= 1;
        if state.attached == 0 {
            self.0.idle.notify_all();
        }
    }
}
