//! Where a successful ReadProperty or ReadPropertyMultiple value came from.
//!
//! The context names the object, property and array index the ACK names,
//! which can differ from the request (a wildcard Device instance, for one).
//! It holds no copy of the value's octets: `decode_property_as` takes the
//! value's `.octets`.

use super::{PyObjectIdentifier, PyPropertyValue};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::prelude::*;

/// The object, property and optional array index the ACK names.
#[pyclass(
    name = "PropertyReadContext",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Debug, Clone, Copy)]
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
