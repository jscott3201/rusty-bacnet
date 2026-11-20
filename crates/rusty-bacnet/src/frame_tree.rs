//! Immutable Python fields from the shared offline decoder.

use bacnet_ops::decode::tree::{Field, decode_tree};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

/// A decoded label and the half-open input span that supplied it.
#[pyclass(name = "FieldNode", module = "rusty_bacnet", frozen, get_all)]
struct PyFieldNode {
    /// The native decoder's display label.
    label: String,
    /// The start and exclusive end offsets in the supplied frame.
    span: (usize, usize),
    /// Ordered child fields.
    children: Py<PyTuple>,
    /// Whether the label contains a character string supplied by the device.
    device_text: bool,
}

fn nodes<'py>(py: Python<'py>, fields: Vec<Field>) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        py,
        fields
            .into_iter()
            .map(|field| -> PyResult<_> {
                Py::new(
                    py,
                    PyFieldNode {
                        label: field.label,
                        span: (field.range.start, field.range.end),
                        children: nodes(py, field.children)?.unbind(),
                        device_text: field.device_text,
                    },
                )
            })
            .collect::<PyResult<Vec<_>>>()?,
    )
}

/// Decode ordered fields for one link-layer frame. Each field's span indexes
/// the input. Malformed frames return the fields and diagnostics available.
#[pyfunction]
fn decode_frame_tree<'py>(
    py: Python<'py>,
    octets: &[u8],
    link: &str,
) -> PyResult<Bound<'py, PyTuple>> {
    nodes(py, decode_tree(crate::frame_decode::framing(link)?, octets))
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyFieldNode>()?;
    module.add_function(wrap_pyfunction!(decode_frame_tree, module)?)?;
    Ok(())
}
