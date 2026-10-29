//! ReinitializeDevice from Python (#1566): the `on_reinitialize` callable
//! that accepts or refuses a request, the `after_reinitialize` notification
//! that carries the restart out once the SimpleACK has been handed off, and
//! the `ReinitializeRequest` both receive.
//!
//! The Rust handler runs with the object database write-locked, so the
//! callable is entered through [`crate::py_async::attach`] and must return
//! at once: it may not await, and [`crate::py_async::under_native_lock`]
//! refuses any binding operation it starts, since that operation would wait
//! for the lock the handler holds. That guard is per thread: it can't see a
//! callable that waits on the event loop or on another thread (a
//! `run_coroutine_threadsafe(...).result()`, a `Thread.join`, any future's
//! `.result()`), and such a wait deadlocks the server if the other side
//! needs the database. The notification runs later, on the event loop that
//! started the server, with no lock held.

use std::sync::{Arc, Mutex, PoisonError};

use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::ReinitializeContext;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ErrorClass, ErrorCode, ReinitializedState};
use bacnet_types::error::Error;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PySet, PyTuple};

use crate::types::{PyReinitializedState, int_enums};

/// A ReinitializeDevice request a Python `on_reinitialize` callable is asked
/// to accept, and that `after_reinitialize` then carries out.
#[pyclass(
    name = "ReinitializeRequest",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyReinitializeRequest {
    state: ReinitializedState,
    source_mac: MacAddr,
    source_network: Option<(u16, MacAddr)>,
    invoke_id: u8,
}

impl PyReinitializeRequest {
    fn new(context: &ReinitializeContext) -> Self {
        Self {
            state: context.state,
            source_mac: context.source_mac.clone(),
            source_network: context
                .source_network
                .as_ref()
                .map(|source| (source.network, source.mac_address.clone())),
            invoke_id: context.invoke_id,
        }
    }
}

#[pymethods]
impl PyReinitializeRequest {
    /// The requested state.
    #[getter]
    fn state(&self) -> PyReinitializedState {
        PyReinitializedState { inner: self.state }
    }

    /// The immediate link source: the requester, or the last router.
    #[getter]
    fn source_mac<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.source_mac)
    }

    /// The originating network of a routed request, else None.
    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.source_network.as_ref().map(|(network, _)| *network)
    }

    /// The originating MAC of a routed request, else None.
    #[getter]
    fn source_address<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.source_network
            .as_ref()
            .map(|(_, mac)| PyBytes::new(py, mac))
    }

    /// The request's invoke ID.
    #[getter]
    fn invoke_id(&self) -> u8 {
        self.invoke_id
    }

    fn __repr__(&self) -> String {
        format!(
            "ReinitializeRequest(state=ReinitializedState.{}, invoke_id={})",
            self.state, self.invoke_id
        )
    }

    /// Refuses `copy` and `pickle`, like the other values the bindings hand
    /// to callbacks. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A server's or endpoint's Python ReinitializeDevice callables.
pub(crate) struct PyReinitialize {
    on_reinitialize: Py<PyAny>,
    after_reinitialize: Option<Arc<Py<PyAny>>>,
    /// The event loop the owner last started on, where the notification
    /// runs.
    after_reply: Mutex<Option<Arc<AfterReply>>>,
    /// The notification's unfinished tasks, from every start, held until
    /// each finishes so a restart never lets one be collected mid-run.
    tasks: Py<PySet>,
}

/// Validate the constructor's `on_reinitialize` and `after_reinitialize`;
/// `after_reinitialize` needs `on_reinitialize`.
pub(crate) fn parse(
    on_reinitialize: Option<&Bound<'_, PyAny>>,
    after_reinitialize: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<Arc<PyReinitialize>>> {
    for (name, value) in [
        ("on_reinitialize", on_reinitialize),
        ("after_reinitialize", after_reinitialize),
    ] {
        if value.is_some_and(|value| !value.is_callable()) {
            return Err(PyTypeError::new_err(format!("{name} must be callable")));
        }
    }
    match on_reinitialize {
        Some(handler) => Ok(Some(Arc::new(PyReinitialize {
            on_reinitialize: handler.clone().unbind(),
            after_reinitialize: after_reinitialize.map(|after| Arc::new(after.clone().unbind())),
            after_reply: Mutex::new(None),
            tasks: PySet::empty(handler.py())?.unbind(),
        }))),
        None if after_reinitialize.is_some() => Err(PyValueError::new_err(
            "after_reinitialize needs on_reinitialize",
        )),
        None => Ok(None),
    }
}

/// What the callable answered.
enum Answer {
    Accept,
    Refuse(ErrorClass, ErrorCode),
    /// It raised, or returned something else: SERVICES / OTHER.
    Failed,
}

impl PyReinitialize {
    /// Remember the running event loop for the notification. Called from
    /// `start()` on the Python thread, so the loop is the one the owner runs
    /// on.
    pub(crate) fn bind_event_loop(&self, py: Python<'_>) -> PyResult<()> {
        if self.after_reinitialize.is_none() {
            return Ok(());
        }
        let event_loop = py
            .import(intern!(py, "asyncio"))?
            .call_method0(intern!(py, "get_running_loop"))?;
        self.set_event_loop(py, event_loop);
        Ok(())
    }

    /// Point the notification at `event_loop`, keeping the one task set.
    fn set_event_loop(&self, py: Python<'_>, event_loop: Bound<'_, PyAny>) {
        let after = AfterReply {
            event_loop: event_loop.unbind(),
            tasks: self.tasks.clone_ref(py),
        };
        *self
            .after_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(after));
    }

    /// The Rust handler that runs the callable.
    pub(crate) fn handler(
        self: &Arc<Self>,
    ) -> impl Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error>
    + Send
    + Sync
    + 'static
    + use<> {
        let this = Arc::clone(self);
        move |context, _db| this.decide(context)
    }

    fn decide(&self, context: &ReinitializeContext) -> Result<(), Error> {
        let request = PyReinitializeRequest::new(context);
        let answer = crate::py_async::attach(|py| {
            let returned = crate::py_async::under_native_lock(|| {
                self.on_reinitialize.bind(py).call1((request.clone(),))
            });
            Ok(match returned.and_then(|value| answer(&value)) {
                Ok(answer) => answer,
                Err(error) => {
                    // Shown through sys.unraisablehook, as asyncio shows a
                    // callback's exception.
                    error.write_unraisable(py, Some(self.on_reinitialize.bind(py)));
                    Answer::Failed
                }
            })
        })
        // The interpreter is exiting: nobody can carry the request out.
        .unwrap_or(Answer::Failed);
        match answer {
            Answer::Accept => {
                let after = self
                    .after_reply
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let (Some(callback), Some(after)) = (self.after_reinitialize.clone(), after) {
                    context.after_reply(move || after.schedule(callback, request));
                }
                Ok(())
            }
            Answer::Refuse(class, code) => Err(Error::protocol(class, code)),
            // Answered SERVICES / OTHER, as a Rust handler's panic is.
            Answer::Failed => Err(Error::Encoding(
                "Python on_reinitialize raised or answered with an unexpected value".into(),
            )),
        }
    }
}

/// Read the callable's answer: None or True accepts, False refuses with
/// SERVICES / SERVICE_REQUEST_DENIED, and an `(ErrorClass, ErrorCode)` tuple
/// refuses with those. Anything else is a TypeError.
fn answer(value: &Bound<'_, PyAny>) -> PyResult<Answer> {
    if value.is_none() {
        return Ok(Answer::Accept);
    }
    if let Ok(accept) = value.cast::<PyBool>() {
        return Ok(if accept.is_true() {
            Answer::Accept
        } else {
            Answer::Refuse(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
        });
    }
    if let Ok(tuple) = value.cast::<PyTuple>()
        && tuple.len() == 2
        && let (Ok(class), Ok(code)) = (
            int_enums::from_py::<ErrorClass>(&tuple.get_item(0)?),
            int_enums::from_py::<ErrorCode>(&tuple.get_item(1)?),
        )
    {
        return Ok(Answer::Refuse(class, code));
    }
    let py = value.py();
    if py
        .import(intern!(py, "inspect"))?
        .call_method1(intern!(py, "isawaitable"), (value,))?
        .is_truthy()?
    {
        // Closed, so it never warns that it was never awaited.
        if value.hasattr(intern!(py, "close"))? {
            value.call_method0(intern!(py, "close"))?;
        }
        return Err(PyTypeError::new_err(
            "on_reinitialize must be a plain function; use after_reinitialize for async work",
        ));
    }
    Err(PyTypeError::new_err(
        "on_reinitialize must return None, a bool, or an (ErrorClass, ErrorCode) tuple",
    ))
}

/// Where an accepted request's `after_reinitialize` runs: the event loop the
/// owner started on, holding each coroutine's task until it finishes.
struct AfterReply {
    event_loop: Py<PyAny>,
    tasks: Py<PySet>,
}

impl AfterReply {
    /// Called by the server once the SimpleACK has been handed off, on the
    /// request's task. The loop is reached from a blocking thread, so a busy
    /// interpreter never stalls a runtime worker.
    fn schedule(self: Arc<Self>, callback: Arc<Py<PyAny>>, request: PyReinitializeRequest) {
        let run = move || {
            let _ = crate::py_async::attach(|py| {
                let tasks = self.tasks.bind(py);
                let result = self.event_loop.bind(py).call_method1(
                    intern!(py, "call_soon_threadsafe"),
                    (RunAfterReinitialize, callback.bind(py), request, tasks),
                );
                if let Err(error) = result {
                    // A closed loop has nobody left to run it.
                    error.write_unraisable(py, Some(callback.bind(py)));
                }
                Ok(())
            });
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(run)),
            Err(_) => run(),
        }
    }
}

/// Runs on the event loop: call `after_reinitialize(request)`, and when it
/// returns an awaitable, run that as a task, kept until it finishes.
#[pyclass(frozen)]
struct RunAfterReinitialize;

#[pymethods]
impl RunAfterReinitialize {
    fn __call__(
        &self,
        callback: &Bound<'_, PyAny>,
        request: &Bound<'_, PyAny>,
        tasks: &Bound<'_, PySet>,
    ) -> PyResult<()> {
        let py = callback.py();
        let returned = callback.call1((request,))?;
        let inspect = py.import(intern!(py, "inspect"))?;
        if inspect
            .call_method1(intern!(py, "isawaitable"), (&returned,))?
            .is_truthy()?
        {
            let task = py
                .import(intern!(py, "asyncio"))?
                .call_method1(intern!(py, "ensure_future"), (returned,))?;
            tasks.add(&task)?;
            task.call_method1(
                intern!(py, "add_done_callback"),
                (tasks.getattr(intern!(py, "discard"))?,),
            )?;
        }
        Ok(())
    }
}

/// Refuse a DCC or ReinitializeDevice password (`name` is its keyword) that
/// is empty (#2094) or over the 20-character limit (#2060) with ValueError,
/// at construction or before a request is sent, as the native server and
/// endpoint refuse it at start and the encoders refuse it.
pub(crate) fn password_limit(name: &str, password: &Option<String>) -> PyResult<()> {
    use bacnet_services::device_mgmt::{MAX_PASSWORD_CHARS, password_fits};
    if password.as_deref() == Some("") {
        return Err(PyValueError::new_err(format!(
            "an empty {name} is refused: give a nonempty {name}, or None for no password"
        )));
    }
    if password.as_deref().is_some_and(|p| !password_fits(p)) {
        return Err(PyValueError::new_err(format!(
            "{name} exceeds the {MAX_PASSWORD_CHARS}-character limit"
        )));
    }
    Ok(())
}

/// An endpoint's ReinitializeDevice settings: the callables and the
/// password a request must carry.
#[derive(Clone, Default)]
pub(crate) struct EndpointReinitialize {
    callables: Option<Arc<PyReinitialize>>,
    password: Option<String>,
}

impl EndpointReinitialize {
    /// Validate an endpoint constructor's keywords. A password needs
    /// `on_reinitialize`, as the native session requires.
    pub(crate) fn parse(
        on_reinitialize: Option<&Bound<'_, PyAny>>,
        after_reinitialize: Option<&Bound<'_, PyAny>>,
        password: Option<String>,
    ) -> PyResult<Self> {
        let callables = parse(on_reinitialize, after_reinitialize)?;
        if password.is_some() && callables.is_none() {
            return Err(PyValueError::new_err(
                "reinit_password needs on_reinitialize",
            ));
        }
        password_limit("reinit_password", &password)?;
        Ok(Self {
            callables,
            password,
        })
    }

    /// Whether the server role executes ReinitializeDevice.
    pub(crate) fn enabled(&self) -> bool {
        self.callables.is_some()
    }

    /// See [`PyReinitialize::bind_event_loop`]; called by `start()`.
    pub(crate) fn bind_event_loop(&self, py: Python<'_>) -> PyResult<()> {
        match &self.callables {
            Some(callables) => callables.bind_event_loop(py),
            None => Ok(()),
        }
    }

    /// Install the handler and password on a session about to start.
    pub(crate) fn install<T>(
        &self,
        session: bacnet_endpoint::session::EndpointSession<T>,
    ) -> bacnet_endpoint::session::EndpointSession<T>
    where
        T: bacnet_transport::port::TransportPort + 'static,
    {
        let Some(callables) = &self.callables else {
            return session;
        };
        let session = session.with_reinitialize(callables.handler());
        match &self.password {
            Some(password) => session.with_reinit_password(password.clone()),
            None => session,
        }
    }
}

/// Register `ReinitializeRequest` with the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyReinitializeRequest>()
}

#[cfg(test)]
#[path = "reinitialize_tests.rs"]
mod tests;
