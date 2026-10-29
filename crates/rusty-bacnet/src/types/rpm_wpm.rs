use super::*;
use bacnet_services::rpm::ReadAccessResult;

// ---------------------------------------------------------------------------
// Python-side RPM/WPM shapes, as PyO3 extracts them from method arguments
// ---------------------------------------------------------------------------

/// Python `(property, array_index)` reference in a ReadPropertyMultiple spec.
type PyPropertyReference = (PyPropertyIdentifier, Option<u32>);
/// Python `(object, [property_reference, ...])` ReadPropertyMultiple spec.
pub(crate) type PyReadAccessSpec = (PyObjectIdentifier, Vec<PyPropertyReference>);
/// Python `(property, value, priority, array_index)` write, used by
/// WritePropertyMultiple specs and CreateObject initial values.
pub(crate) type PyPropertyWrite = (
    PyPropertyIdentifier,
    PyPropertyValue,
    Option<u8>,
    Option<u32>,
);
/// Python `(object, [property_write, ...])` WritePropertyMultiple spec.
pub(crate) type PyWriteAccessSpec = (PyObjectIdentifier, Vec<PyPropertyWrite>);
/// Python `(device_instance, object, property, value, priority, array_index)`
/// row for `write_property_to_devices`.
pub(crate) type PyDeviceWrite = (
    u32,
    PyObjectIdentifier,
    PyPropertyIdentifier,
    PyPropertyValue,
    Option<u8>,
    Option<u32>,
);

// ---------------------------------------------------------------------------
// RPM/WPM conversion helpers (crate-internal)
// ---------------------------------------------------------------------------

/// Convert Python RPM specs to Rust ReadAccessSpecification list.
pub(crate) fn py_to_rpm_specs(specs: Vec<PyReadAccessSpec>) -> Vec<ReadAccessSpecification> {
    specs
        .into_iter()
        .map(|(oid, props)| ReadAccessSpecification {
            object_identifier: oid.to_rust(),
            list_of_property_references: props
                .into_iter()
                .map(|(pid, idx)| PropertyReference {
                    property_identifier: pid.to_rust(),
                    property_array_index: idx,
                })
                .collect(),
        })
        .collect()
}

/// Convert a ReadPropertyMultipleACK to Python `list[dict]`.
pub(crate) fn rpm_ack_to_py(py: Python<'_>, ack: ReadPropertyMultipleACK) -> PyResult<Py<PyAny>> {
    let outer = PyList::empty(py);
    for result in ack.list_of_read_access_results {
        outer.append(read_access_result_to_py(py, result)?)?;
    }
    Ok(outer.into_any().unbind())
}

/// One object's results, as a `dict` with `object_id` and `results`. A
/// ReadPropertyMultiple ACK is a list of these, and so is a Group's
/// Present_Value.
pub(crate) fn read_access_result_to_py(
    py: Python<'_>,
    result: ReadAccessResult,
) -> PyResult<Bound<'_, PyDict>> {
    let obj_dict = PyDict::new(py);
    obj_dict.set_item(
        "object_id",
        PyObjectIdentifier::from_rust(result.object_identifier),
    )?;
    let results_list = PyList::empty(py);
    for elem in result.list_of_results {
        let elem_dict = PyDict::new(py);
        elem_dict.set_item(
            "property_id",
            PyPropertyIdentifier {
                inner: elem.property_identifier,
            },
        )?;
        elem_dict.set_item("array_index", elem.property_array_index)?;
        if let Some(value_bytes) = &elem.property_value {
            match decode_read_value(
                result.object_identifier.object_type(),
                elem.property_identifier,
                elem.property_array_index,
                value_bytes,
            ) {
                Ok(val) => {
                    elem_dict.set_item("value", val)?;
                }
                Err(_) => {
                    elem_dict.set_item("value", PyBytes::new(py, value_bytes))?;
                }
            }
            elem_dict.set_item("error", py.None())?;
        } else if let Some((ec, ev)) = elem.error {
            elem_dict.set_item("value", py.None())?;
            let err_tuple = (IntEnumValue(ec), IntEnumValue(ev));
            elem_dict.set_item("error", err_tuple)?;
        } else {
            elem_dict.set_item("value", py.None())?;
            elem_dict.set_item("error", py.None())?;
        }
        results_list.append(elem_dict)?;
    }
    obj_dict.set_item("results", results_list)?;
    Ok(obj_dict)
}

/// Convert Python WPM specs to Rust WriteAccessSpecification list.
pub(crate) fn py_to_wpm_specs(
    specs: Vec<PyWriteAccessSpec>,
) -> PyResult<Vec<WriteAccessSpecification>> {
    let specs = specs
        .into_iter()
        .map(|(oid, props)| {
            let list_of_properties = props
                .into_iter()
                .map(|(pid, val, priority, array_index)| {
                    let mut buf = BytesMut::new();
                    encode_property_value(&mut buf, &val.inner)
                        .map_err(crate::errors::to_py_err)?;
                    Ok(BACnetPropertyValue {
                        property_identifier: pid.to_rust(),
                        property_array_index: array_index,
                        value: buf.to_vec(),
                        priority,
                    })
                })
                .collect::<PyResult<Vec<_>>>()?;
            Ok(WriteAccessSpecification {
                object_identifier: oid.to_rust(),
                list_of_properties,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: specs,
    };
    request
        .validate()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(request.list_of_write_access_specs)
}

// ---------------------------------------------------------------------------
