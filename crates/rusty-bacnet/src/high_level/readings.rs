//! What the `Device` reads return (#2561): a [`PyReading`] a property,
//! gathered into [`PyReadings`]; the Object_List ([`PyObjectList`]); and a
//! device's summary ([`PyDeviceSummary`]).
//!
//! A property the device refused is data, not an exception: its reading's
//! `error` holds the exception a typed read of that property would raise,
//! unraised, and the other readings stand. The same goes for a device that
//! stopped answering part way: `Readings.stopped` holds what a single read
//! would have raised.

use bacnet_ops::client::{DeviceSummary, ListRead, ReadVia};
use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::target::AddressStyle;
use bacnet_ops::{PropertyError, Reading, Readings};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};

use super::errors::ops_err;
use super::specs::{object_arg, property_arg};
use super::value::PyValue;
use crate::errors::{BacnetError, to_py_err};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

/// How a read went on the wire, as Python names it.
fn via_label(via: ReadVia) -> &'static str {
    match via {
        ReadVia::Rpm => "rpm",
        ReadVia::Rp => "rp",
        ReadVia::Mixed => "mixed",
    }
}

/// One property as read: `value`, or `error` when the device gave none.
/// Exactly one of the two is None. Refuses `copy` and `pickle`; its
/// `value` copies and pickles.
#[pyclass(name = "Reading", module = "rusty_bacnet", frozen)]
pub(crate) struct PyReading {
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Option<Py<PyValue>>,
    /// The exception a typed read of the property would raise, unraised.
    error: Option<Py<PyAny>>,
}

impl PyReading {
    fn new(py: Python<'_>, reading: &Reading) -> PyResult<Self> {
        let (value, error) = match &reading.value {
            Ok(value) => {
                let value = PyValue::new(
                    reading.object.object_type(),
                    reading.property,
                    reading.index,
                    value,
                );
                (Some(Py::new(py, value)?), None)
            }
            Err(error) => (None, Some(row_error(py, error))),
        };
        Ok(Self {
            object: reading.object,
            property: reading.property,
            index: reading.index,
            value,
            error,
        })
    }

    /// The property as text: `analog-input 1 present-value`.
    fn label(&self) -> String {
        let index = self.index.map(|i| format!("[{i}]")).unwrap_or_default();
        format!(
            "{} {}{index}",
            object_label(self.object),
            kebab(self.property)
        )
    }
}

/// A row's error as the exception a typed read would raise: the device's
/// Error, Reject or Abort as the protocol, reject or abort error, and
/// anything else as a BacnetError with its text.
pub(super) fn row_error(py: Python<'_>, error: &PropertyError) -> Py<PyAny> {
    let raised = match error {
        PropertyError::Remote(remote) => to_py_err(Error::Remote(*remote)),
        other => BacnetError::new_err(other.to_string()),
    };
    raised.into_value(py).into_any()
}

#[pymethods]
impl PyReading {
    /// The object read.
    #[getter]
    fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// The property read.
    #[getter]
    fn property_id(&self) -> PyPropertyIdentifier {
        PyPropertyIdentifier {
            inner: self.property,
        }
    }

    /// The array element read, None for the whole property.
    #[getter]
    fn array_index(&self) -> Option<u32> {
        self.index
    }

    /// The value, a `Value`; None when the device gave none (see `error`).
    #[getter]
    fn value(&self, py: Python<'_>) -> Option<Py<PyValue>> {
        self.value.as_ref().map(|value| value.clone_ref(py))
    }

    /// Why the device gave no value, as the exception a typed read of the
    /// property would raise (a BacnetProtocolError with `error_class` and
    /// `error_code`, say), never raised here; None when it gave one.
    #[getter]
    fn error(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.error.as_ref().map(|error| error.clone_ref(py))
    }

    /// The value's text, None when the device gave no value.
    #[getter]
    fn text(&self) -> Option<&str> {
        self.value.as_ref().map(|value| value.get().text())
    }

    /// True when the device gave a value.
    #[getter]
    fn ok(&self) -> bool {
        self.value.is_some()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let shown = match (&self.value, &self.error) {
            (Some(value), _) => PyString::new(py, value.get().text()).repr()?.to_string(),
            (None, Some(error)) => format!("error={}", error.bind(py).repr()?),
            (None, None) => "None".to_owned(),
        };
        Ok(format!("Reading({}, {shown})", self.label()))
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// Each reading as a Python object, in order.
fn rows(py: Python<'_>, readings: &[Reading]) -> PyResult<Py<PyTuple>> {
    let rows = readings
        .iter()
        .map(|reading| Py::new(py, PyReading::new(py, reading)?))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyTuple::new(py, rows)?.unbind())
}

/// What `Device.read_many` or `Device.read_object` read: a `Reading` each
/// property, in the order asked for, and whether the device stopped
/// answering part way. Iterating it gives the readings. Refuses `copy` and
/// `pickle`.
#[pyclass(name = "Readings", module = "rusty_bacnet", frozen)]
pub(crate) struct PyReadings {
    readings: Py<PyTuple>,
    via: Option<ReadVia>,
    stopped: Option<Py<PyAny>>,
}

impl PyReadings {
    /// A bulk read's result; `stopped`'s address in `style`.
    pub(crate) fn new(py: Python<'_>, read: Readings, style: AddressStyle) -> PyResult<Self> {
        Ok(Self {
            readings: rows(py, &read.readings)?,
            via: read.via,
            stopped: read
                .stopped
                .map(|error| ops_err(error, style).into_value(py).into_any()),
        })
    }

    /// An object's read: every row, read one way, never stopped part way.
    pub(crate) fn of_object(py: Python<'_>, readings: &[Reading], via: ReadVia) -> PyResult<Self> {
        Ok(Self {
            readings: rows(py, readings)?,
            via: Some(via),
            stopped: None,
        })
    }
}

#[pymethods]
impl PyReadings {
    /// The readings as a tuple, in the order asked for; after `stopped`,
    /// those read before the device stopped answering.
    #[getter]
    fn readings(&self, py: Python<'_>) -> Py<PyTuple> {
        self.readings.clone_ref(py)
    }

    /// How the values were read: `"rpm"` (ReadPropertyMultiple), `"rp"`
    /// (ReadProperty each) or `"mixed"`; None when nothing was read.
    #[getter]
    fn via(&self) -> Option<&'static str> {
        self.via.map(via_label)
    }

    /// Set when the read ended after it began: the device stopped
    /// answering (the BacnetTimeoutError or BacnetTransportError a single
    /// read would have raised), or the client's own link failed a later
    /// request (a BacnetError). Unraised. The properties after `readings`
    /// weren't read.
    #[getter]
    fn stopped(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.stopped.as_ref().map(|error| error.clone_ref(py))
    }

    /// The first reading of `property` of `object` (its Present_Value when
    /// `property` is left out), in the forms `Device.read` takes; None
    /// when the read holds none.
    #[pyo3(signature = (object, property=None, *, index=None))]
    fn get(
        &self,
        py: Python<'_>,
        object: &Bound<'_, PyAny>,
        property: Option<&Bound<'_, PyAny>>,
        index: Option<u32>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let wanted = property_arg(object, property, index)?;
        for row in self.readings.bind(py).iter() {
            let reading = row.cast::<PyReading>()?.get();
            if (reading.object, reading.property, reading.index)
                == (wanted.object, wanted.property, wanted.index)
            {
                return Ok(Some(row.unbind()));
            }
        }
        Ok(None)
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.readings.bind(py).len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.readings
            .bind(py)
            .as_any()
            .try_iter()
            .map(Bound::into_any)
    }

    fn __getitem__<'py>(
        &self,
        py: Python<'py>,
        index: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.readings.bind(py).as_any().get_item(index)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let stopped = if self.stopped.is_some() {
            ", stopped"
        } else {
            ""
        };
        format!(
            "Readings({} readings, via={}{stopped})",
            self.readings.bind(py).len(),
            self.via.map_or("None", via_label)
        )
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A device's Object_List as read (`Device.objects`). Iterating it gives
/// the `ObjectIdentifier`s. Refuses `copy` and `pickle`.
#[pyclass(name = "ObjectList", module = "rusty_bacnet", frozen)]
pub(crate) struct PyObjectList {
    objects: Py<PyTuple>,
    indexed: bool,
    stopped: Option<String>,
}

impl PyObjectList {
    pub(crate) fn new(py: Python<'_>, read: &ListRead) -> PyResult<Self> {
        let objects = read
            .objects
            .iter()
            .map(|object| Py::new(py, PyObjectIdentifier::from_rust(*object)))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            objects: PyTuple::new(py, objects)?.unbind(),
            indexed: read.indexed,
            stopped: read.stopped.clone(),
        })
    }
}

#[pymethods]
impl PyObjectList {
    /// The objects as a tuple of `ObjectIdentifier`, in the device's order.
    #[getter]
    fn objects(&self, py: Python<'_>) -> Py<PyTuple> {
        self.objects.clone_ref(py)
    }

    /// True when the list was read an element at a time, because the
    /// device couldn't send it in one answer.
    #[getter]
    fn indexed(&self) -> bool {
        self.indexed
    }

    /// Why an element-by-element read ended early, as text; None when the
    /// whole list was read. `objects` then holds the elements before it.
    #[getter]
    fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.objects.bind(py).len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.objects
            .bind(py)
            .as_any()
            .try_iter()
            .map(Bound::into_any)
    }

    fn __getitem__<'py>(
        &self,
        py: Python<'py>,
        index: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.objects.bind(py).as_any().get_item(index)
    }

    fn __contains__(&self, py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<bool> {
        let wanted = object_arg(object)?;
        for held in self.objects.bind(py).iter() {
            if held.cast::<PyObjectIdentifier>()?.get().to_rust() == wanted {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let stopped = if self.stopped.is_some() {
            ", stopped"
        } else {
            ""
        };
        format!(
            "ObjectList({} objects{stopped})",
            self.objects.bind(py).len()
        )
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// A device's summary (`Device.summary`): its instance, and the Device
/// object's identifying properties as `readings` (name, vendor, model,
/// firmware, protocol revision, what it supports). Refuses `copy` and
/// `pickle`.
#[pyclass(name = "DeviceSummary", module = "rusty_bacnet", frozen)]
pub(crate) struct PyDeviceSummary {
    instance: u32,
    name: Option<String>,
    readings: Py<PyReadings>,
}

impl PyDeviceSummary {
    pub(crate) fn new(py: Python<'_>, summary: &DeviceSummary) -> PyResult<Self> {
        let readings = PyReadings::of_object(py, &summary.readings, summary.via)?;
        Ok(Self {
            instance: summary.device,
            name: summary.name(),
            readings: Py::new(py, readings)?,
        })
    }
}

#[pymethods]
impl PyDeviceSummary {
    /// The device instance.
    #[getter]
    fn instance(&self) -> u32 {
        self.instance
    }

    /// The device's Object_Name with control characters removed, for
    /// display; None when the device gave none. Text the device supplied.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The properties read, as `Readings`: one the device refused has its
    /// `error` set.
    #[getter]
    fn readings(&self, py: Python<'_>) -> Py<PyReadings> {
        self.readings.clone_ref(py)
    }

    /// The `Value` of `property` (a `PropertyIdentifier` or its text) of
    /// the Device object; None when the summary doesn't read it or the
    /// device gave none.
    fn value(&self, py: Python<'_>, property: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyValue>>> {
        let device = ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, self.instance)
            .map_err(to_py_err)?;
        let device = Bound::new(py, PyObjectIdentifier::from_rust(device))?;
        let found = self
            .readings
            .get()
            .get(py, device.as_any(), Some(property), None)?;
        match found {
            Some(row) => Ok(row.bind(py).cast::<PyReading>()?.get().value(py)),
            None => Ok(None),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let name = match &self.name {
            Some(name) => PyString::new(py, name).repr()?.to_string(),
            None => "None".to_owned(),
        };
        Ok(format!("DeviceSummary({}, name={name})", self.instance))
    }

    /// Refuses `copy` and `pickle`, as the other received results do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
