//! A value with no typed form read as its tags (#2618): `PropertyValue` tag
//! `"tagged"`, whose `.value` is a list of `TagNode` records built here from
//! `bacnet_encoding::tags::tree`.
//!
//! A read value that holds a context tag, and has no typed constructed form,
//! used to come back as one opaque `application_data` blob. Its tags are
//! readable without knowing its datatype (Clause 20.2.1.6), so it now comes
//! back as that tree: each application primitive as the `PropertyValue` a
//! bare read of it gives, each context primitive's contents as `bytes`
//! (their datatype is the field's, which only the property's production
//! knows), and each constructed element with its members. The value keeps
//! its octets exactly, as `application_data` did.

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::TagClass;
use bacnet_encoding::tags::tree::{TagBody, TagNode, decode_tag_tree};
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyTuple};

use super::PyPropertyValue;

/// Whether `octets` read as a tree the binding can show: well framed,
/// within the decoder's bounds, and every application primitive one that
/// `PropertyValue` holds (not a character set it can't convert, an
/// ENUMERATED past 32 bits, or contents that don't decode).
pub(crate) fn is_tree(octets: &[u8]) -> bool {
    decode_tag_tree(octets).is_ok_and(|tree| tree.iter().all(holds))
}

/// Whether every application primitive in `node` has a `PropertyValue`.
fn holds(node: &TagNode<'_>) -> bool {
    match &node.body {
        TagBody::Application(_) => owned(node).is_some(),
        TagBody::Context(_) => true,
        TagBody::Constructed(members) => members.iter().all(holds),
        _ => false,
    }
}

/// An application node's value as `PropertyValue` holds it, read from its
/// octets as a bare read of it is; `None` for a value it can't hold.
fn owned(node: &TagNode<'_>) -> Option<PropertyValue> {
    decode_application_value(node.octets, 0)
        .ok()
        .map(|(value, _)| value)
}

/// The top-level nodes of a tagged value's octets, as `TagNode`s. The
/// octets were checked with [`is_tree`] when the value was made; octets
/// that somehow don't read come back as `bytes`, as a typed element's do.
pub(crate) fn nodes_to_py(py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
    let Ok(tree) = decode_tag_tree(octets) else {
        return Ok(PyBytes::new(py, octets).into_any().unbind());
    };
    let list = PyList::empty(py);
    for node in &tree {
        list.append(PyTagNode::from_node(py, node)?)?;
    }
    Ok(list.into_any().unbind())
}

/// What a [`PyTagNode`] holds.
enum Held {
    Application(PyPropertyValue),
    Context(Vec<u8>),
    Constructed(Vec<Py<PyTagNode>>),
}

/// One tagged element of a value read as its tags, frozen.
#[pyclass(name = "TagNode", module = "rusty_bacnet", frozen)]
pub struct PyTagNode {
    class: TagClass,
    number: u8,
    octets: Vec<u8>,
    held: Held,
}

impl PyTagNode {
    fn from_node(py: Python<'_>, node: &TagNode<'_>) -> PyResult<Py<Self>> {
        let held = match &node.body {
            TagBody::Application(_) => Held::Application(PyPropertyValue::from_rust(
                owned(node).unwrap_or_else(|| PropertyValue::ApplicationData(node.octets.to_vec())),
            )),
            TagBody::Constructed(members) => Held::Constructed(
                members
                    .iter()
                    .map(|member| Self::from_node(py, member))
                    .collect::<PyResult<_>>()?,
            ),
            _ => Held::Context(node.contents().to_vec()),
        };
        Py::new(
            py,
            Self {
                class: node.class,
                number: node.number,
                octets: node.octets.to_vec(),
                held,
            },
        )
    }
}

#[pymethods]
impl PyTagNode {
    /// `"application"` or `"context"` (Clause 20.2.1.1).
    #[getter]
    fn tag_class(&self) -> &'static str {
        match self.class {
            TagClass::Application => "application",
            TagClass::Context => "context",
        }
    }

    /// The tag number: an application tag's datatype (Clause 20.2.1.4), a
    /// context tag's field.
    #[getter]
    fn tag_number(&self) -> u8 {
        self.number
    }

    /// Whether it is a constructed element, the members between an opening
    /// and a closing tag (Clause 20.2.1.3.2).
    #[getter]
    fn constructed(&self) -> bool {
        matches!(self.held, Held::Constructed(_))
    }

    /// What it holds: an application primitive's `PropertyValue`, a context
    /// primitive's contents as `bytes`, or a constructed element's members
    /// as a list of `TagNode`.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match &self.held {
            Held::Application(value) => Py::new(py, value.clone())?.into_any(),
            Held::Context(contents) => PyBytes::new(py, contents).into_any().unbind(),
            Held::Constructed(members) => {
                let list = PyList::empty(py);
                for member in members {
                    list.append(member.clone_ref(py))?;
                }
                list.into_any().unbind()
            }
        })
    }

    /// The element's octets exactly as read: its tag header and contents,
    /// or its opening tag through its closing tag.
    #[getter]
    fn octets<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.octets)
    }

    /// A short description, not code: `<TagNode application 4 real 1.5>`,
    /// `<TagNode context 0, 1 octet>` or `<TagNode context 1 constructed, 3
    /// members>`. A long value is cut short, and contents are counted, not
    /// shown.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let plural = |count: usize, one: &str| match count {
            1 => format!("1 {one}"),
            count => format!("{count} {one}s"),
        };
        Ok(match &self.held {
            Held::Application(value) => {
                let shown = Py::new(py, value.clone())?
                    .bind(py)
                    .getattr("value")?
                    .repr()?
                    .to_string();
                format!(
                    "<TagNode application {} {} {}>",
                    self.number,
                    value.tag_name(),
                    shorten(&shown)
                )
            }
            Held::Context(contents) => format!(
                "<TagNode context {}, {}>",
                self.number,
                plural(contents.len(), "octet")
            ),
            Held::Constructed(members) => format!(
                "<TagNode context {} constructed, {}>",
                self.number,
                plural(members.len(), "member")
            ),
        })
    }

    /// Equal when they were read from the same octets, which decide
    /// everything else about them.
    fn __eq__(&self, other: &Self) -> bool {
        self.octets == other.octets
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut state = std::collections::hash_map::DefaultHasher::new();
        self.octets.hash(&mut state);
        state.finish()
    }

    /// What `copy` and `pickle` call: `_from_octets` with the octets.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let arguments = (PyBytes::new(py, &slf.get().octets),).into_pyobject(py)?;
        Ok((slf.get_type().getattr("_from_octets")?, arguments))
    }

    /// The node `octets` hold, which must be exactly one element the binding
    /// shows; anything else raises ValueError.
    #[staticmethod]
    fn _from_octets(py: Python<'_>, octets: &[u8]) -> PyResult<Py<Self>> {
        match decode_tag_tree(octets).as_deref() {
            Ok([node]) if holds(node) => Self::from_node(py, node),
            _ => Err(PyValueError::new_err(
                "octets are not exactly one tagged element",
            )),
        }
    }
}

/// `text` cut to at most 32 characters, an ellipsis marking the cut.
fn shorten(text: &str) -> String {
    match text.char_indices().nth(31) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
#[path = "tag_tree_tests.rs"]
mod tests;
