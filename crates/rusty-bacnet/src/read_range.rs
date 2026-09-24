//! Shared Python subset and return shape for standalone and endpoint ReadRange.
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

#[allow(clippy::too_many_arguments)]
pub(crate) fn request(
    object: &PyObjectIdentifier,
    property: &PyPropertyIdentifier,
    array_index: Option<u32>,
    range_type: Option<&str>,
    reference_index: Option<u32>,
    reference_seq: Option<u32>,
    count: Option<i32>,
) -> PyResult<ReadRangeRequest> {
    let range = match range_type {
        Some("position") => Some(RangeSpec::ByPosition {
            reference_index: reference_index.unwrap_or(0),
            count: count.unwrap_or(0),
        }),
        Some("sequence") => Some(RangeSpec::BySequenceNumber {
            reference_seq: reference_seq.unwrap_or(0),
            count: count.unwrap_or(0),
        }),
        Some(_) => {
            return Err(PyValueError::new_err(
                "range_type must be 'position', 'sequence', or None",
            ))
        }
        None => None,
    };
    let request = ReadRangeRequest {
        object_identifier: object.to_rust(),
        property_identifier: property.to_rust(),
        property_array_index: array_index,
        range,
    };
    request
        .validate()
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(request)
}

pub(crate) fn ack_to_dict(py: Python<'_>, ack: ReadRangeAck) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item(
        "object_id",
        PyObjectIdentifier::from_rust(ack.object_identifier),
    )?;
    dict.set_item(
        "property_id",
        PyPropertyIdentifier {
            inner: ack.property_identifier,
        },
    )?;
    dict.set_item("array_index", ack.property_array_index)?;
    dict.set_item("result_flags", ack.result_flags)?;
    dict.set_item("item_count", ack.item_count)?;
    dict.set_item("item_data", PyBytes::new(py, &ack.item_data))?;
    dict.set_item("first_sequence_number", ack.first_sequence_number)?;
    Ok(dict.into_any().unbind())
}
