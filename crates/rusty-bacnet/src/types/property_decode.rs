//! Offline interpretation of property octets under a declaration supplied by the caller.

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

/// A forced decode and the original input, including when the declaration does not fit.
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

/// The external primitive token and its native application tag must agree exactly.
fn primitive(token: &str) -> PyResult<(Datatype, u8)> {
    Ok(match token {
        "Null" => (Datatype::Null, app_tag::NULL),
        "Boolean" => (Datatype::Boolean, app_tag::BOOLEAN),
        "Unsigned" => (Datatype::Unsigned, app_tag::UNSIGNED),
        "Integer" => (Datatype::Integer, app_tag::SIGNED),
        "Real" => (Datatype::Real, app_tag::REAL),
        "Double" => (Datatype::Double, app_tag::DOUBLE),
        "OctetString" => (Datatype::OctetString, app_tag::OCTET_STRING),
        "CharacterString" => (Datatype::CharacterString, app_tag::CHARACTER_STRING),
        "BitString" => (Datatype::BitString, app_tag::BIT_STRING),
        "Enumerated" => (Datatype::Enumerated, app_tag::ENUMERATED),
        "Date" => (Datatype::Date, app_tag::DATE),
        "Time" => (Datatype::Time, app_tag::TIME),
        "ObjectIdentifier" => (Datatype::ObjectIdentifier, app_tag::OBJECT_IDENTIFIER),
        _ => {
            return Err(PyValueError::new_err(format!(
                "unknown primitive {token:?}"
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
                    .map_err(|_| PyValueError::new_err("array_length must be in 0..65535"))?,
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

/// Finish the bounded framing and payload check before allocating owned values.
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
        if item.tag.class == TagClass::Context || item.tag.number > app_tag::OBJECT_IDENTIFIER {
            unsupported
                .get_or_insert_with(|| format!("context or reserved tag at octet {}", item.offset));
            continue;
        }
        if let Some(Err(error)) = item.application_value() {
            if item.tag.number == app_tag::ENUMERATED && error.kind == DecodingKind::OutOfRange {
                unsupported.get_or_insert_with(|| error.to_string());
                continue;
            }
            return Err(Refusal::new("malformed", error));
        }
        if item.tag.number != decl.expected_tag {
            wrong_tag.get_or_insert((item.offset, item.tag.number));
        }
        starts.push(item.offset);
    }
    if let Some(detail) = unsupported {
        return Err(Refusal::new("unsupported_encoding", detail));
    }
    if let Some((at, tag)) = wrong_tag {
        return Err(Refusal::new(
            "type_mismatch",
            format!(
                "expected application tag {}, found {tag} at octet {at}",
                decl.expected_tag
            ),
        ));
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
    let read = |at| {
        decode_application_value(raw, at)
            .map(|(value, _)| value)
            .map_err(|error| Refusal::new("unsupported_encoding", error))
    };
    if whole {
        return starts
            .into_iter()
            .map(read)
            .collect::<Result<Vec<_>, _>>()
            .map(PropertyValue::List);
    }
    let value = read(starts[0])?;
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
        let key = match value {
            PropertyValue::Unsigned(n) => i128::from(*n),
            PropertyValue::Signed(n) => i128::from(*n),
            PropertyValue::Enumerated(n) => i128::from(*n),
            _ => unreachable!("validated integer application tag"),
        };
        labels.push(match mapping.get_item(key) {
            Ok(label) => Some(label.extract::<String>()?),
            Err(error) if error.is_instance_of::<PyKeyError>(py) => None,
            Err(error) => return Err(error),
        });
    }
    Ok(Some(PyTuple::new(py, labels)?.unbind()))
}

/// Decode property value octets using the supplied primitive and shape. Each result keeps the input.
#[pyfunction]
#[pyo3(signature = (octets, primitive, *, shape="scalar", array_index=None, array_length=None, value_names=None))]
fn decode_property_as(
    octets: &Bound<'_, PyBytes>,
    primitive: &str,
    shape: &str,
    array_index: Option<&Bound<'_, PyInt>>,
    array_length: Option<&Bound<'_, PyInt>>,
    value_names: Option<&Bound<'_, PyMapping>>,
) -> PyResult<PyPropertyDecodeResult> {
    let decl = Declaration::parse(primitive, shape, array_index, array_length)?;
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
