//! A Load Control's application route from Python (#2479): the `shed` and
//! `stop` callables `BACnetServer.add_load_control` takes stand in for the
//! Rust `LoadShedder`, and `ShedRequest` is what `shed` receives.
//!
//! The object asks `shed` how the loads stand each time it looks at a
//! request: on a write that changes it, and on the server's once-a-second
//! pass while one is pending or under way. It calls `stop` once a request
//! `shed` was asked about ends. Both run with the object database
//! write-locked, so they are entered through [`crate::py_async::attach`]
//! and must return at once: they may not await, and
//! [`crate::py_async::under_native_lock`] refuses any binding operation
//! they start, since it would wait for the lock the server holds. As for
//! `on_reinitialize`, a callable that waits on the event loop or another
//! thread for something that needs the database deadlocks the server.
//!
//! A failed `shed` counts as nothing shed, as a Rust shedder's panic does:
//! an exception or an answer of the wrong shape (shown through
//! `sys.unraisablehook`), and an exiting interpreter, leave the loads
//! waiting with both levels at the requested choice's no-shed default. An
//! exception from `stop` is shown the same way, and the request ends anyway.

use bacnet_objects::load_control::{LoadShedder, ShedProgress, ShedRequest, ShedStatus};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple};

use crate::types::{date_value, shed_level_from_py, shed_level_to_py, time_value};

/// A date and time as Python reads them: `((year, month, day, day_of_week),
/// (hour, minute, second, hundredths))`.
type DateTimeTuple = ((u16, u8, u8, u8), (u8, u8, u8, u8));

/// A shed request handed to a Load Control's Python `shed` callable.
#[pyclass(
    name = "ShedRequest",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyShedRequest {
    inner: ShedRequest,
}

#[pymethods]
impl PyShedRequest {
    /// Requested_Shed_Level, as a shed-level mapping.
    #[getter]
    fn requested<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        shed_level_to_py(py, &self.inner.requested)
    }

    /// For a level request, the index in the `shed_levels` list of the
    /// level whose action it selects: the requested one, or the nearest
    /// below it. None for a percent or an amount, or when no level is at or
    /// below the one requested.
    #[getter]
    fn shed_level(&self) -> Option<usize> {
        self.inner.shed_level
    }

    /// Start_Time, as `((year, month, day, day_of_week), (hour, minute,
    /// second, hundredths))`.
    #[getter]
    fn start_time(&self) -> DateTimeTuple {
        let (date, time) = self.inner.start_time;
        (date_value(&date), time_value(&time))
    }

    /// Shed_Duration, in minutes.
    #[getter]
    fn shed_duration(&self) -> u64 {
        self.inner.shed_duration
    }

    /// Duty_Window, in minutes.
    #[getter]
    fn duty_window(&self) -> u64 {
        self.inner.duty_window
    }

    /// Full_Duty_Baseline in kilowatts, or None while the object serves
    /// none.
    #[getter]
    fn full_duty_baseline(&self) -> Option<f64> {
        self.inner.full_duty_baseline.map(f64::from)
    }

    /// Whether Start_Time has come.
    #[getter]
    fn started(&self) -> bool {
        self.inner.started
    }

    fn __repr__(&self) -> String {
        format!(
            "ShedRequest(requested={:?}, shed_duration={}, duty_window={}, started={})",
            self.inner.requested,
            self.inner.shed_duration,
            self.inner.duty_window,
            if self.inner.started { "True" } else { "False" }
        )
    }

    /// Refuses `copy` and `pickle`, like the other values the bindings hand
    /// to callbacks. See [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A Load Control's Python callables, as its `LoadShedder`.
pub(crate) struct PyLoadShedder {
    shed: Py<PyAny>,
    stop: Option<Py<PyAny>>,
}

impl PyLoadShedder {
    /// The shedder for `add_load_control`'s `shed` and `stop` keywords, or
    /// None without `shed`. A keyword that isn't callable raises TypeError,
    /// and `stop` without `shed`, which nothing would call, ValueError.
    pub(crate) fn parse(
        shed: Option<&Bound<'_, PyAny>>,
        stop: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<Self>> {
        let callable = |value: Option<&Bound<'_, PyAny>>, keyword: &str| match value
            .filter(|value| !value.is_none())
        {
            Some(value) if value.is_callable() => Ok(Some(value.clone().unbind())),
            Some(_) => Err(PyTypeError::new_err(format!("{keyword} must be callable"))),
            None => Ok(None),
        };
        let (shed, stop) = (callable(shed, "shed")?, callable(stop, "stop")?);
        match shed {
            Some(shed) => Ok(Some(Self { shed, stop })),
            None if stop.is_some() => Err(PyValueError::new_err("stop needs shed")),
            None => Ok(None),
        }
    }
}

impl LoadShedder for PyLoadShedder {
    fn shed(&self, request: &ShedRequest) -> ShedStatus {
        let argument = PyShedRequest {
            inner: request.clone(),
        };
        crate::py_async::attach(|py| {
            let shed = self.shed.bind(py);
            let returned = crate::py_async::under_native_lock(|| shed.call1((argument,)));
            Ok(match returned.and_then(|value| status(&value)) {
                Ok(status) => Some(status),
                Err(error) => {
                    // Shown through sys.unraisablehook, as asyncio shows a
                    // callback's exception.
                    error.write_unraisable(py, Some(shed));
                    None
                }
            })
        })
        .ok()
        .flatten()
        // A failure, or an exiting interpreter: nothing shed.
        .unwrap_or_else(|| ShedStatus::idle(&request.requested))
    }

    fn stop(&self) {
        let Some(stop) = &self.stop else {
            return;
        };
        // An exiting interpreter has nobody to tell.
        let _ = crate::py_async::attach(|py| {
            let stop = stop.bind(py);
            if let Err(error) = crate::py_async::under_native_lock(|| stop.call0()) {
                error.write_unraisable(py, Some(stop));
            }
            Ok(())
        });
    }
}

/// The `shed` callable's answer: `(progress, expected, actual)`, the
/// progress `"waiting"`, `"shedding"` or `"compliant"` and the two levels
/// shed-level mappings. Anything else, a coroutine included, raises.
fn status(value: &Bound<'_, PyAny>) -> PyResult<ShedStatus> {
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
            "the shed callable must be a plain function, not a coroutine",
        ));
    }
    let shape = || {
        PyTypeError::new_err("the shed callable must return a (progress, expected, actual) tuple")
    };
    let answer = value.cast::<PyTuple>().map_err(|_| shape())?;
    if answer.len() != 3 {
        return Err(shape());
    }
    let progress = answer.get_item(0)?;
    let progress = match progress.cast::<PyString>().map(|text| text.to_cow()) {
        Ok(Ok(text)) if text == "waiting" => ShedProgress::Waiting,
        Ok(Ok(text)) if text == "shedding" => ShedProgress::Shedding,
        Ok(Ok(text)) if text == "compliant" => ShedProgress::Compliant,
        _ => {
            return Err(PyValueError::new_err(
                "the shed progress must be 'waiting', 'shedding' or 'compliant'",
            ));
        }
    };
    Ok(ShedStatus::new(
        progress,
        shed_level_from_py(&answer.get_item(1)?, "expected")?,
        shed_level_from_py(&answer.get_item(2)?, "actual")?,
    ))
}

/// Register `ShedRequest` with the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyShedRequest>()
}
