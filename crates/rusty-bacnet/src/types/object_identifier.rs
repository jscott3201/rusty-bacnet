use super::*;

// ---------------------------------------------------------------------------
// ObjectIdentifier
// ---------------------------------------------------------------------------

/// BACnet Object Identifier (type + instance).
///
/// Usage: `ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)`
#[pyclass(name = "ObjectIdentifier", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyObjectIdentifier {
    inner: primitives::ObjectIdentifier,
}

#[pymethods]
impl PyObjectIdentifier {
    #[new]
    fn new(object_type: &PyObjectType, instance: u32) -> PyResult<Self> {
        let oid = primitives::ObjectIdentifier::new(object_type.to_rust(), instance)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner: oid })
    }

    /// The object type.
    #[getter]
    fn object_type(&self) -> PyObjectType {
        PyObjectType {
            inner: self.inner.object_type(),
        }
    }

    /// The instance number.
    #[getter]
    fn instance(&self) -> u32 {
        self.inner.instance_number()
    }

    fn __repr__(&self) -> String {
        format!(
            "ObjectIdentifier({}, {})",
            self.inner.object_type(),
            self.inner.instance_number()
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        (self.inner.object_type().to_raw() as u64) << 22 | self.inner.instance_number() as u64
    }
}

impl PyObjectIdentifier {
    pub fn to_rust(&self) -> primitives::ObjectIdentifier {
        self.inner
    }

    pub fn from_rust(oid: primitives::ObjectIdentifier) -> Self {
        Self { inner: oid }
    }
}
