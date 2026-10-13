//! Shared Python subset and return shape for standalone and endpoint ReadRange.
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};
use bacnet_services::read_range::{
    RangeSpec, ReadRangeReply, ReadRangeRequest, ReadRangeValidation,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

pub(crate) fn request(
    object: &PyObjectIdentifier,
    property: &PyPropertyIdentifier,
    array_index: Option<u32>,
    range_type: Option<&str>,
    reference_index: Option<u64>,
    reference_seq: Option<u64>,
    // The count is an INTEGER16 on the wire, so the parameter is an i16 and
    // a count outside it raises OverflowError (#1360).
    count: Option<i16>,
    reference_time: Option<&Bound<'_, PyAny>>,
) -> PyResult<ReadRangeRequest> {
    let count = count.map(i32::from);
    let range = match range_type {
        Some("position") => Some(RangeSpec::ByPosition {
            reference_index: reference_index.unwrap_or(0),
            count: count.unwrap_or(0),
        }),
        Some("sequence") => Some(RangeSpec::BySequenceNumber {
            reference_seq: reference_seq.unwrap_or(0),
            count: count.unwrap_or(0),
        }),
        Some("time") => {
            let reference_time = reference_time
                .filter(|time| !time.is_none())
                .ok_or_else(|| PyValueError::new_err("range_type 'time' needs a reference_time"))?;
            Some(RangeSpec::ByTime {
                reference_time: crate::log_records::reference_time(reference_time)?,
                count: count.unwrap_or(0),
            })
        }
        Some(_) => {
            return Err(PyValueError::new_err(
                "range_type must be 'position', 'sequence', 'time', or None",
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

/// `"strict"` (refuse an answer that breaks a rule) or `"lenient"` (keep it
/// and list the rules in `violations`).
pub(crate) fn validation(validation: &str) -> PyResult<ReadRangeValidation> {
    match validation {
        "strict" => Ok(ReadRangeValidation::Strict),
        "lenient" => Ok(ReadRangeValidation::Lenient),
        other => Err(PyValueError::new_err(format!(
            "validation must be 'strict' or 'lenient', got '{other}'"
        ))),
    }
}

pub(crate) fn reply_to_dict(py: Python<'_>, reply: ReadRangeReply) -> PyResult<Py<PyAny>> {
    let ReadRangeReply { ack, violations } = reply;
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
    dict.set_item(
        "violations",
        crate::log_records::violation_names(&violations),
    )?;
    Ok(dict.into_any().unbind())
}
