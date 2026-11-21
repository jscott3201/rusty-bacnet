//! Immutable Python fields from the shared offline decoder.

use std::hash::{Hash, Hasher};

use bacnet_ops::decode::tree::{Field, decode_tree};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyString, PyTuple};

/// A decoded label and the half-open input span that supplied it. Two nodes
/// are equal, and hash alike, when their labels, spans, device-text flags
/// and children all are.
#[pyclass(name = "FieldNode", module = "rusty_bacnet", frozen)]
struct PyFieldNode {
    /// The native decoder's display label.
    #[pyo3(get)]
    label: String,
    /// The start and exclusive end offsets in the supplied frame.
    #[pyo3(get)]
    span: (usize, usize),
    /// Ordered child fields.
    children: Vec<Py<PyFieldNode>>,
    /// Whether the label contains a character string supplied by the device.
    #[pyo3(get)]
    device_text: bool,
}

impl PyFieldNode {
    fn from_field(py: Python<'_>, field: Field) -> PyResult<Py<Self>> {
        let children = field
            .children
            .into_iter()
            .map(|child| Self::from_field(py, child))
            .collect::<PyResult<_>>()?;
        Py::new(
            py,
            Self {
                label: field.label,
                span: (field.range.start, field.range.end),
                children,
                device_text: field.device_text,
            },
        )
    }

    /// The same label, span, flag and children, all the way down.
    fn same(&self, other: &Self) -> bool {
        self.label == other.label
            && self.span == other.span
            && self.device_text == other.device_text
            && self.children.len() == other.children.len()
            && self
                .children
                .iter()
                .zip(&other.children)
                .all(|(one, two)| one.get().same(two.get()))
    }

    fn digest<H: Hasher>(&self, state: &mut H) {
        self.label.hash(state);
        self.span.hash(state);
        self.device_text.hash(state);
        self.children.len().hash(state);
        for child in &self.children {
            child.get().digest(state);
        }
    }
}

#[pymethods]
impl PyFieldNode {
    /// The fields this one is made of, in frame order.
    #[getter]
    fn children<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.children.iter().map(|child| child.clone_ref(py)))
    }

    /// A short description, not code: `<FieldNode 'Invoke ID: 7' 8..9, 0
    /// children>`. A long label is cut short; no octets are shown.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let label = match self.label.char_indices().nth(59) {
            Some((cut, _)) => format!("{}…", &self.label[..cut]),
            None => self.label.clone(),
        };
        let count = match self.children.len() {
            1 => "1 child".to_owned(),
            count => format!("{count} children"),
        };
        Ok(format!(
            "<FieldNode {} {}..{}, {count}>",
            PyString::new(py, &label).repr()?,
            self.span.0,
            self.span.1
        ))
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.same(other)
    }

    fn __hash__(&self) -> u64 {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        self.digest(&mut state);
        state.finish()
    }

    /// What `copy` and `pickle` call: `_rebuild` with the label, span,
    /// children and device-text flag.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let node = slf.get();
        let arguments =
            (&node.label, node.span, node.children(py)?, node.device_text).into_pyobject(py)?;
        Ok((slf.get_type().getattr("_rebuild")?, arguments))
    }

    /// A node from a pickle's fields, as [`Self::__reduce__`] lists them.
    /// Nothing is decoded; a span that ends before it starts raises
    /// ValueError.
    #[staticmethod]
    fn _rebuild(
        label: String,
        span: (usize, usize),
        children: Vec<Py<PyFieldNode>>,
        device_text: bool,
    ) -> PyResult<Self> {
        if span.0 > span.1 {
            return Err(PyValueError::new_err("span ends before it starts"));
        }
        Ok(Self {
            label,
            span,
            children,
            device_text,
        })
    }
}

/// Decode ordered fields for one link-layer frame. Each field's span indexes
/// the input. Malformed frames return the fields and diagnostics available.
/// Input longer than the link's largest frame raises ValueError.
#[pyfunction]
fn decode_frame_tree<'py>(
    py: Python<'py>,
    octets: &[u8],
    link: &str,
) -> PyResult<Bound<'py, PyTuple>> {
    let framing = crate::frame_decode::framing(link)?;
    let limit = framing.max_frame_length();
    if octets.len() > limit {
        return Err(PyValueError::new_err(format!(
            "{} octets is longer than the largest {link} frame ({limit} octets)",
            octets.len()
        )));
    }
    let roots = decode_tree(framing, octets)
        .into_iter()
        .map(|field| PyFieldNode::from_field(py, field))
        .collect::<PyResult<Vec<_>>>()?;
    PyTuple::new(py, roots)
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyFieldNode>()?;
    module.add_function(wrap_pyfunction!(decode_frame_tree, module)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "frame_tree_tests.rs"]
mod tests;
