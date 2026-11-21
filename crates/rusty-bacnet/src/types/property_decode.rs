//! Offline decoding of property value octets as a datatype and shape the
//! caller declares (#2618), for a vendor property whose datatype the stack
//! does not know. The caller picks the declaration (by the station's vendor
//! identifier, the object type and the property number); nothing here infers
//! one.

use super::PyPropertyValue;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::tree::{MAX_TAG_TREE_NODES, TagReader, TagTreeFault};
use bacnet_encoding::tags::{TagClass, app_tag};
use bacnet_types::datatype::{ArraySize, Datatype, ValueShape, ValueType};
use bacnet_types::error::DecodingKind;
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyInt, PyMapping, PyTuple};

/// A declared decode and its input, kept whether or not the declaration fit.
/// A value class: two results are equal when every field is.
#[pyclass(
    name = "PropertyDecodeResult",
    module = "rusty_bacnet",
    frozen,
    get_all
)]
struct PyPropertyDecodeResult {
    status: &'static str,
    value: Option<PyPropertyValue>,
    raw: Py<PyBytes>,
    diagnostic: Option<String>,
    value_names: Option<Py<PyTuple>>,
}

/// The statuses a result can carry.
const STATUSES: [&str; 5] = [
    "decoded",
    "type_mismatch",
    "unsupported_encoding",
    "malformed",
    "limit_exceeded",
];

#[pymethods]
impl PyPropertyDecodeResult {
    fn __eq__(&self, py: Python<'_>, other: PyRef<'_, Self>) -> PyResult<bool> {
        let names = match (&self.value_names, &other.value_names) {
            (Some(a), Some(b)) => a.bind(py).eq(b.bind(py))?,
            (None, None) => true,
            _ => false,
        };
        Ok(names
            && self.status == other.status
            && self.value == other.value
            && self.raw.bind(py).as_bytes() == other.raw.bind(py).as_bytes()
            && self.diagnostic == other.diagnostic)
    }

    /// Hashes the status, input and diagnostic, which equal results share.
    fn __hash__(&self, py: Python<'_>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut state = std::collections::hash_map::DefaultHasher::new();
        (self.status, self.raw.bind(py).as_bytes(), &self.diagnostic).hash(&mut state);
        state.finish()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let detail = match (&self.value, &self.diagnostic) {
            (Some(value), _) => format!("value={}", Bound::new(py, value.clone())?.repr()?),
            (None, Some(diagnostic)) => format!(
                "diagnostic={}",
                pyo3::types::PyString::new(py, diagnostic).repr()?
            ),
            (None, None) => String::new(),
        };
        Ok(format!(
            "PropertyDecodeResult(status='{}', {detail}, raw=<{} bytes>)",
            self.status,
            self.raw.bind(py).as_bytes().len()
        ))
    }

    /// What `copy` and `pickle` call: `_restore` with every field.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let this = slf.get();
        let arguments = (
            this.status,
            this.value.clone(),
            this.raw.clone_ref(py),
            this.diagnostic.clone(),
            this.value_names.as_ref().map(|names| names.clone_ref(py)),
        )
            .into_pyobject(py)?;
        Ok((slf.get_type().getattr("_restore")?, arguments))
    }

    /// Rebuild a pickled result.
    #[staticmethod]
    fn _restore(
        status: &str,
        value: Option<PyPropertyValue>,
        raw: Py<PyBytes>,
        diagnostic: Option<String>,
        value_names: Option<Py<PyTuple>>,
    ) -> PyResult<Self> {
        let status = STATUSES
            .into_iter()
            .find(|known| *known == status)
            .ok_or_else(|| PyValueError::new_err(format!("unknown status {status:?}")))?;
        Ok(Self {
            status,
            value,
            raw,
            diagnostic,
            value_names,
        })
    }
}

/// The datatype a declaration names, by the `PropertyValue.tag` name of the
/// value it decodes to, and that datatype's application tag (Clause 20.2.1.4).
fn primitive(token: &str) -> PyResult<(Datatype, u8)> {
    Ok(match token {
        "null" => (Datatype::Null, app_tag::NULL),
        "boolean" => (Datatype::Boolean, app_tag::BOOLEAN),
        "unsigned" => (Datatype::Unsigned, app_tag::UNSIGNED),
        "signed" => (Datatype::Integer, app_tag::SIGNED),
        "real" => (Datatype::Real, app_tag::REAL),
        "double" => (Datatype::Double, app_tag::DOUBLE),
        "octet_string" => (Datatype::OctetString, app_tag::OCTET_STRING),
        "character_string" => (Datatype::CharacterString, app_tag::CHARACTER_STRING),
        "bit_string" => (Datatype::BitString, app_tag::BIT_STRING),
        "enumerated" => (Datatype::Enumerated, app_tag::ENUMERATED),
        "date" => (Datatype::Date, app_tag::DATE),
        "time" => (Datatype::Time, app_tag::TIME),
        "object_identifier" => (Datatype::ObjectIdentifier, app_tag::OBJECT_IDENTIFIER),
        _ => {
            return Err(PyValueError::new_err(format!(
                "unknown datatype {token:?}: expected a primitive PropertyValue tag such as \
                 \"unsigned\" or \"octet_string\""
            )));
        }
    })
}

struct Declaration {
    value_type: ValueType,
    index: Option<u32>,
    expected_tag: u8,
}

impl Declaration {
    fn parse(
        token: &str,
        shape: &str,
        index: Option<&Bound<'_, PyInt>>,
        length: Option<&Bound<'_, PyInt>>,
    ) -> PyResult<Self> {
        let (datatype, tag) = primitive(token)?;
        if shape != "array" && (index.is_some() || length.is_some()) {
            return Err(PyValueError::new_err(
                "array_index and array_length require shape='array'",
            ));
        }
        let value_type = match (shape, length) {
            ("scalar", _) => ValueType::single(datatype),
            ("list", _) => ValueType::list(datatype),
            ("array", None) => ValueType::array(datatype),
            ("array", Some(length)) => ValueType::fixed_array(
                datatype,
                length
                    .extract::<u16>()
                    .map_err(|_| PyValueError::new_err("array_length must be 0 to 65535"))?,
            ),
            _ => return Err(PyValueError::new_err(format!("unknown shape {shape:?}"))),
        };
        let index = index
            .map(|i| {
                i.extract::<u32>().map_err(|_| {
                    PyValueError::new_err("array_index must fit an unsigned 32-bit integer")
                })
            })
            .transpose()?;
        Ok(Self {
            value_type,
            index,
            expected_tag: if index == Some(0) {
                app_tag::UNSIGNED
            } else {
                tag
            },
        })
    }

    fn whole_collection(&self) -> bool {
        self.index.is_none() && (self.value_type.is_array() || self.value_type.is_list())
    }
}

struct Refusal {
    status: &'static str,
    detail: String,
}

impl Refusal {
    fn new(status: &'static str, detail: impl ToString) -> Self {
        Self {
            status,
            detail: detail.to_string(),
        }
    }
}

/// Whether an application element the decoder refused is an integer with
/// more contents octets than the model's integer holds: Clauses 20.2.4,
/// 20.2.5 and 20.2.11 set no upper length, but `PropertyValue` keeps an
/// Unsigned in 64 bits and a Signed or Enumerated in 32. Such an encoding is
/// `unsupported_encoding` whether or not it is minimal; a non-minimal
/// encoding that fits the model decodes.
fn wider_than_model(tag: u8, contents: usize) -> bool {
    match tag {
        app_tag::UNSIGNED => contents > 8,
        app_tag::SIGNED | app_tag::ENUMERATED => contents > 4,
        _ => false,
    }
}

/// Walk the framing and check each element before allocating owned values.
///
/// A broken encoding is `malformed` wherever it is. Otherwise a shape or tag
/// that differs from the declaration (context-tagged or constructed content
/// under a primitive included) is `type_mismatch`, and only then is an
/// encoding the model cannot hold (an unconverted character set, an integer
/// wider than the model) `unsupported_encoding`.
fn decode(raw: &[u8], decl: &Declaration) -> Result<PropertyValue, Refusal> {
    let mut starts = Vec::new();
    let mut nodes = 0;
    let mut unsupported = None;
    let mut wrong_tag = None;
    for item in TagReader::new(raw) {
        let item = item.map_err(|e| {
            Refusal::new(
                match e.fault {
                    TagTreeFault::TooDeep | TagTreeFault::TooManyNodes => "limit_exceeded",
                    _ => "malformed",
                },
                e,
            )
        })?;
        if item.tag.is_closing {
            continue;
        }
        nodes += 1;
        if nodes > MAX_TAG_TREE_NODES {
            return Err(Refusal::new("limit_exceeded", TagTreeFault::TooManyNodes));
        }
        if item.tag.class == TagClass::Context {
            wrong_tag.get_or_insert_with(|| {
                format!(
                    "expected application tag {}, found context tag {} at octet {}",
                    decl.expected_tag, item.tag.number, item.offset
                )
            });
            continue;
        }
        if item.tag.number > app_tag::OBJECT_IDENTIFIER {
            return Err(Refusal::new(
                "malformed",
                format!(
                    "reserved application tag {} at octet {}",
                    item.tag.number, item.offset
                ),
            ));
        }
        // Clause 20.2.10: a bit string with no data octets has no unused
        // bits. Checked here, not in the shared decoder, which every read,
        // write and COV path uses and which stays tolerant of older devices.
        if item.tag.class == TagClass::Application
            && item.tag.number == app_tag::BIT_STRING
            && matches!(item.contents, [unused] if *unused != 0)
        {
            return Err(Refusal::new(
                "malformed",
                format!(
                    "empty bit string with {} unused bits at octet {}",
                    item.contents[0], item.offset
                ),
            ));
        }
        if let Some(Err(error)) = item.application_value() {
            if error.kind == DecodingKind::Unsupported
                || wider_than_model(item.tag.number, item.contents.len())
            {
                unsupported.get_or_insert_with(|| error.to_string());
            } else {
                return Err(Refusal::new("malformed", error));
            }
        }
        if item.tag.number != decl.expected_tag {
            wrong_tag.get_or_insert_with(|| {
                format!(
                    "expected application tag {}, found {} at octet {}",
                    decl.expected_tag, item.tag.number, item.offset
                )
            });
        }
        // Only top-level elements are values; anything nested is inside
        // context-tagged content, already a mismatch.
        if item.depth == 0 {
            starts.push(item.offset);
        }
    }
    if let Some(detail) = wrong_tag {
        return Err(Refusal::new("type_mismatch", detail));
    }
    let whole = decl.whole_collection();
    if !whole && starts.len() != 1 {
        return Err(Refusal::new(
            "type_mismatch",
            format!("expected one value, received {}", starts.len()),
        ));
    }
    let fixed = match decl.value_type.shape {
        ValueShape::Array(ArraySize::Fixed(n)) => Some(n),
        _ => None,
    };
    if let Some(n) = fixed
        && (whole && starts.len() != usize::from(n) || decl.index.is_some_and(|i| i > u32::from(n)))
    {
        return Err(Refusal::new(
            "type_mismatch",
            format!("array count or index does not fit declared length {n}"),
        ));
    }
    if let Some(detail) = unsupported {
        return Err(Refusal::new("unsupported_encoding", detail));
    }
    // The borrowed view accepted every element; what the owned decoder
    // still refuses is a character set the model does not convert.
    let read = |at| {
        decode_application_value(raw, at)
            .map(|(value, _)| value)
            .map_err(|error| {
                let status = match error.decode_fault() {
                    Some((_, DecodingKind::Unsupported)) => "unsupported_encoding",
                    _ => "malformed",
                };
                Refusal::new(status, error)
            })
    };
    let Some(&first) = starts.first() else {
        return Ok(PropertyValue::List(Vec::new()));
    };
    if whole {
        return starts
            .into_iter()
            .map(read)
            .collect::<Result<Vec<_>, _>>()
            .map(PropertyValue::List);
    }
    let value = read(first)?;
    if let (Some(0), Some(n), PropertyValue::Unsigned(count)) = (decl.index, fixed, &value)
        && *count != u64::from(n)
    {
        return Err(Refusal::new(
            "type_mismatch",
            format!("array length is {count}, declaration specifies {n}"),
        ));
    }
    Ok(value)
}

fn names(
    py: Python<'_>,
    mapping: Option<&Bound<'_, PyMapping>>,
    decl: &Declaration,
    value: &PropertyValue,
) -> PyResult<Option<Py<PyTuple>>> {
    let Some(mapping) = mapping else {
        return Ok(None);
    };
    if decl.index == Some(0) {
        return Ok(Some(PyTuple::empty(py).unbind()));
    }
    if !matches!(
        decl.value_type.datatype,
        Datatype::Unsigned | Datatype::Integer | Datatype::Enumerated
    ) {
        return Ok(None);
    }
    let values = match value {
        PropertyValue::List(items) => &items[..],
        scalar => std::slice::from_ref(scalar),
    };
    let mut labels = Vec::with_capacity(values.len());
    for value in values {
        // Every element carries the declared integer tag; anything else
        // would have no name rather than a panic.
        let key = match value {
            PropertyValue::Unsigned(n) => i128::from(*n),
            PropertyValue::Signed(n) => i128::from(*n),
            PropertyValue::Enumerated(n) => i128::from(*n),
            _ => {
                labels.push(None);
                continue;
            }
        };
        labels.push(match mapping.get_item(key) {
            Ok(label) => Some(label.extract::<String>()?),
            Err(error) if error.is_instance_of::<PyKeyError>(py) => None,
            Err(error) => return Err(error),
        });
    }
    Ok(Some(PyTuple::new(py, labels)?.unbind()))
}

/// Decode application-tagged property value octets (a value's `.octets`)
/// as the `datatype` and `shape` a caller declares. Each result keeps the
/// input.
#[pyfunction]
#[pyo3(signature = (octets, datatype, shape="scalar", *, array_index=None, array_length=None, value_names=None))]
fn decode_property_as(
    octets: &Bound<'_, PyBytes>,
    datatype: &str,
    shape: &str,
    array_index: Option<&Bound<'_, PyInt>>,
    array_length: Option<&Bound<'_, PyInt>>,
    value_names: Option<&Bound<'_, PyMapping>>,
) -> PyResult<PyPropertyDecodeResult> {
    let decl = Declaration::parse(datatype, shape, array_index, array_length)?;
    let raw = octets.clone().unbind();
    Ok(match decode(octets.as_bytes(), &decl) {
        Ok(value) => PyPropertyDecodeResult {
            status: "decoded",
            value_names: names(octets.py(), value_names, &decl, &value)?,
            value: Some(PyPropertyValue::from_rust(value)),
            raw,
            diagnostic: None,
        },
        Err(refusal) => PyPropertyDecodeResult {
            status: refusal.status,
            diagnostic: Some(refusal.detail),
            raw,
            value_names: None,
            value: None,
        },
    })
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyPropertyDecodeResult>()?;
    module.add_function(wrap_pyfunction!(decode_property_as, module)?)?;
    Ok(())
}
