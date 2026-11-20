//! Provenance of a successful ReadProperty or ReadPropertyMultiple value.

use super::{PyObjectIdentifier, PyPropertyValue};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// The object, property, optional array index and exact value octets returned in the ACK.
#[pyclass(
    name = "PropertyReadContext",
    module = "rusty_bacnet",
    frozen,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
pub(super) struct PyPropertyReadContext {
    object: ObjectIdentifier,
    #[pyo3(get)]
    pub(super) property_id: u32,
    #[pyo3(get)]
    pub(super) array_index: Option<u32>,
    octets: Bytes,
}

#[pymethods]
impl PyPropertyReadContext {
    /// Object identifier returned by the responder.
    #[getter]
    pub(super) fn object_id(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.object)
    }

    /// Exact value octets before the ordinary read decoder interpreted them.
    #[getter]
    pub(super) fn raw<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.octets)
    }
}

impl PyPropertyValue {
    pub(super) fn with_read_context(
        mut self,
        object: ObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        raw: &[u8],
    ) -> Self {
        self.context = Some(PyPropertyReadContext {
            object,
            property_id,
            array_index,
            octets: Bytes::copy_from_slice(raw),
        });
        self
    }
}

#[pymethods]
impl PyPropertyValue {
    /// The returned address and original octets of a successful network property read.
    #[getter]
    fn read_context(&self) -> Option<PyPropertyReadContext> {
        self.context.clone()
    }

    /// Restore ACK provenance after the existing reducer rebuilds the value and its form.
    #[staticmethod]
    fn _with_read_context(
        value: Self,
        object_id: PyObjectIdentifier,
        property_id: u32,
        array_index: Option<u32>,
        raw: &[u8],
    ) -> Self {
        value.with_read_context(object_id.to_rust(), property_id, array_index, raw)
    }
}
