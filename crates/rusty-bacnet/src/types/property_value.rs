use super::*;

// ---------------------------------------------------------------------------
// PropertyValue — typed wrapper with explicit variant constructors.
// ---------------------------------------------------------------------------

/// BACnet application-layer value.
///
/// Use typed constructors to create values:
/// ```python
/// PropertyValue.real(72.5)
/// PropertyValue.unsigned(42)
/// PropertyValue.boolean(True)
/// PropertyValue.character_string("hello")
/// PropertyValue.null()
/// ```
///
/// Read results with `.value` (native Python type) and `.tag` (type name).
#[pyclass(name = "PropertyValue", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyPropertyValue {
    pub(crate) inner: primitives::PropertyValue,
}

impl PyPropertyValue {
    pub fn to_rust(&self) -> &primitives::PropertyValue {
        &self.inner
    }

    pub fn from_rust(value: primitives::PropertyValue) -> Self {
        Self { inner: value }
    }
}

// ---------------------------------------------------------------------------
// PropertyValue -> native Python conversion (used by PropertyValue.value getter)
// ---------------------------------------------------------------------------

fn property_value_to_py(py: Python<'_>, value: &primitives::PropertyValue) -> PyResult<Py<PyAny>> {
    Ok(match value {
        primitives::PropertyValue::Null => py.None(),
        primitives::PropertyValue::Boolean(b) => {
            b.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Unsigned(u) => u.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Signed(i) => i.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Real(f) => (*f as f64).into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Double(f) => f.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::CharacterString(s) => {
            s.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Enumerated(e) => e.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::OctetString(b) => PyBytes::new(py, b).into_any().unbind(),
        primitives::PropertyValue::ObjectIdentifier(oid) => {
            Py::new(py, PyObjectIdentifier::from_rust(*oid))?.into_any()
        }
        primitives::PropertyValue::BitString { unused_bits, data } => {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("unused_bits", unused_bits)?;
            dict.set_item("data", PyBytes::new(py, data))?;
            dict.into_any().unbind()
        }
        primitives::PropertyValue::Date(d) => (d.year, d.month, d.day, d.day_of_week)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        primitives::PropertyValue::Time(t) => (t.hour, t.minute, t.second, t.hundredths)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        primitives::PropertyValue::List(elements) => {
            let list = pyo3::types::PyList::empty(py);
            for elem in elements {
                list.append(property_value_to_py(py, elem)?)?;
            }
            list.into_any().unbind()
        }
    })
}

#[pymethods]
impl PyPropertyValue {
    // -- Typed constructors --------------------------------------------------

    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: primitives::PropertyValue::Null,
        }
    }

    #[staticmethod]
    fn boolean(value: bool) -> Self {
        Self {
            inner: primitives::PropertyValue::Boolean(value),
        }
    }

    #[staticmethod]
    fn unsigned(value: u64) -> Self {
        Self {
            inner: primitives::PropertyValue::Unsigned(value),
        }
    }

    #[staticmethod]
    fn signed(value: i32) -> Self {
        Self {
            inner: primitives::PropertyValue::Signed(value),
        }
    }

    #[staticmethod]
    fn real(value: f32) -> Self {
        Self {
            inner: primitives::PropertyValue::Real(value),
        }
    }

    #[staticmethod]
    fn double(value: f64) -> Self {
        Self {
            inner: primitives::PropertyValue::Double(value),
        }
    }

    #[staticmethod]
    fn character_string(value: String) -> Self {
        Self {
            inner: primitives::PropertyValue::CharacterString(value),
        }
    }

    #[staticmethod]
    fn octet_string(value: Vec<u8>) -> Self {
        Self {
            inner: primitives::PropertyValue::OctetString(value),
        }
    }

    #[staticmethod]
    fn enumerated(value: u32) -> Self {
        Self {
            inner: primitives::PropertyValue::Enumerated(value),
        }
    }

    #[staticmethod]
    fn object_identifier(oid: &PyObjectIdentifier) -> Self {
        Self {
            inner: primitives::PropertyValue::ObjectIdentifier(oid.to_rust()),
        }
    }

    /// Create a Date property value.
    ///
    /// `year` is the full year (e.g. 2026; 255 for unspecified encodes as 0xFF internally).
    /// `month` is 1-12 (or 255 for unspecified).
    /// `day` is 1-31 (or 255 for unspecified).
    /// `day_of_week` is 1=Monday..7=Sunday (or 255 for unspecified).
    #[staticmethod]
    fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Self {
        Self {
            inner: primitives::PropertyValue::Date(primitives::Date {
                year: year.saturating_sub(1900) as u8,
                month,
                day,
                day_of_week,
            }),
        }
    }

    /// Create a Time property value.
    ///
    /// `hour` is 0-23 (or 255 for unspecified).
    /// `minute` is 0-59 (or 255 for unspecified).
    /// `second` is 0-59 (or 255 for unspecified).
    /// `hundredths` is 0-99 (or 255 for unspecified).
    #[staticmethod]
    fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Self {
        Self {
            inner: primitives::PropertyValue::Time(primitives::Time {
                hour,
                minute,
                second,
                hundredths,
            }),
        }
    }

    /// Create a BitString property value.
    ///
    /// `unused_bits` is the number of unused bits in the last byte (0-7).
    /// `data` is the raw bit data bytes.
    #[staticmethod]
    fn bit_string(unused_bits: u8, data: Vec<u8>) -> Self {
        Self {
            inner: primitives::PropertyValue::BitString { unused_bits, data },
        }
    }

    /// Create a List (array) property value from a list of PropertyValue items.
    #[staticmethod]
    fn list(items: Vec<PyPropertyValue>) -> Self {
        Self {
            inner: primitives::PropertyValue::List(items.into_iter().map(|pv| pv.inner).collect()),
        }
    }

    // -- Accessors -----------------------------------------------------------

    /// The BACnet type tag (e.g. "real", "unsigned", "boolean").
    #[getter]
    fn tag(&self) -> &str {
        match &self.inner {
            primitives::PropertyValue::Null => "null",
            primitives::PropertyValue::Boolean(_) => "boolean",
            primitives::PropertyValue::Unsigned(_) => "unsigned",
            primitives::PropertyValue::Signed(_) => "signed",
            primitives::PropertyValue::Real(_) => "real",
            primitives::PropertyValue::Double(_) => "double",
            primitives::PropertyValue::OctetString(_) => "octet_string",
            primitives::PropertyValue::CharacterString(_) => "character_string",
            primitives::PropertyValue::BitString { .. } => "bit_string",
            primitives::PropertyValue::Enumerated(_) => "enumerated",
            primitives::PropertyValue::Date(_) => "date",
            primitives::PropertyValue::Time(_) => "time",
            primitives::PropertyValue::ObjectIdentifier(_) => "object_identifier",
            primitives::PropertyValue::List(_) => "list",
        }
    }

    /// The value as a native Python type (float, int, str, bool, bytes, etc.).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        property_value_to_py(py, &self.inner)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            primitives::PropertyValue::Null => "PropertyValue.null()".to_string(),
            primitives::PropertyValue::Boolean(b) => format!("PropertyValue.boolean({b})"),
            primitives::PropertyValue::Unsigned(u) => format!("PropertyValue.unsigned({u})"),
            primitives::PropertyValue::Signed(i) => format!("PropertyValue.signed({i})"),
            primitives::PropertyValue::Real(f) => format!("PropertyValue.real({f})"),
            primitives::PropertyValue::Double(f) => format!("PropertyValue.double({f})"),
            primitives::PropertyValue::CharacterString(s) => {
                format!("PropertyValue.character_string({s:?})")
            }
            primitives::PropertyValue::OctetString(b) => {
                format!("PropertyValue.octet_string(<{} bytes>)", b.len())
            }
            primitives::PropertyValue::BitString { data, .. } => {
                format!("PropertyValue.bit_string(<{} bytes>)", data.len())
            }
            primitives::PropertyValue::Enumerated(e) => format!("PropertyValue.enumerated({e})"),
            primitives::PropertyValue::Date(d) => {
                format!("PropertyValue.date({}/{}/{})", d.year, d.month, d.day)
            }
            primitives::PropertyValue::Time(t) => {
                format!("PropertyValue.time({}:{}:{})", t.hour, t.minute, t.second)
            }
            primitives::PropertyValue::ObjectIdentifier(oid) => {
                format!(
                    "PropertyValue.object_identifier({}, {})",
                    oid.object_type(),
                    oid.instance_number()
                )
            }
            primitives::PropertyValue::List(elements) => {
                format!("PropertyValue.list(<{} elements>)", elements.len())
            }
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.inner).hash(&mut h);
        format!("{:?}", self.inner).hash(&mut h);
        h.finish()
    }
}
