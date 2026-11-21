//! Where a successful ReadProperty or ReadPropertyMultiple value came from.
//!
//! The context names the object, property and array index the ACK names,
//! which can differ from the request (a wildcard Device instance, for one).
//! It holds no copy of the value's octets: `decode_property_as` takes the
//! value's `.octets`, which for a primitive are a re-encoding, so the
//! device's character set and any non-minimal integer form are not kept.

use super::{PyObjectIdentifier, PyPropertyValue};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// The object, property and optional array index the ACK names. A value
/// class: equal when all three are, hashable, and picklable on its own.
#[pyclass(
    name = "PropertyReadContext",
    module = "rusty_bacnet",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PyPropertyReadContext {
    object: ObjectIdentifier,
    #[pyo3(get)]
    pub(super) property_id: u32,
    #[pyo3(get)]
    pub(super) array_index: Option<u32>,
}

#[pymethods]
impl PyPropertyReadContext {
    /// The object identifier the ACK names.
    #[getter]
    pub(super) fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    fn __repr__(&self) -> String {
        let index = self
            .array_index
            .map_or_else(|| "None".to_owned(), |index| index.to_string());
        format!(
            "PropertyReadContext(object_id=ObjectIdentifier({}, {}), property_id={}, array_index={index})",
            self.object.object_type(),
            self.object.instance_number(),
            self.property_id,
        )
    }

    /// What `copy` and `pickle` call: `_restore` with the three fields.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let this = slf.get();
        let arguments =
            (this.object_id(), this.property_id, this.array_index).into_pyobject(slf.py())?;
        Ok((slf.get_type().getattr("_restore")?, arguments))
    }

    /// Rebuild a pickled context.
    #[staticmethod]
    fn _restore(object_id: PyObjectIdentifier, property_id: u32, array_index: Option<u32>) -> Self {
        Self {
            object: object_id.to_rust(),
            property_id,
            array_index,
        }
    }
}

impl PyPropertyValue {
    /// This value as read by an RP or RPM whose ACK names `object`,
    /// `property_id` and `array_index`.
    pub(super) fn with_read_context(
        mut self,
        object: ObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Self {
        self.context = Some(PyPropertyReadContext {
            object,
            property_id,
            array_index,
        });
        self
    }
}

#[pymethods]
impl PyPropertyValue {
    /// The object, property and array index the ACK of a network
    /// ReadProperty or ReadPropertyMultiple named; None for any other value.
    #[getter]
    fn read_context(&self) -> Option<PyPropertyReadContext> {
        self.context
    }

    /// What `__reduce__` rebuilds a read value with, so a copy keeps its
    /// context.
    #[staticmethod]
    fn _with_read_context(
        value: Self,
        object_id: PyObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
    ) -> Self {
        value.with_read_context(object_id.to_rust(), property_id, array_index)
    }
}
