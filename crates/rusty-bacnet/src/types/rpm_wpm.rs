use super::*;

// ---------------------------------------------------------------------------
// RPM/WPM conversion helpers (crate-internal)
// ---------------------------------------------------------------------------

/// Convert Python RPM specs to Rust ReadAccessSpecification list.
#[allow(clippy::type_complexity)]
pub(crate) fn py_to_rpm_specs(
    specs: Vec<(PyObjectIdentifier, Vec<(PyPropertyIdentifier, Option<u32>)>)>,
) -> Vec<ReadAccessSpecification> {
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

/// Convert a ReadPropertyMultipleACK to Python list[dict].
pub(crate) fn rpm_ack_to_py(py: Python<'_>, ack: ReadPropertyMultipleACK) -> PyResult<Py<PyAny>> {
    let outer = PyList::empty(py);
    for result in ack.list_of_read_access_results {
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
                match decode_application_value(value_bytes, 0) {
                    Ok((val, _)) => {
                        elem_dict.set_item("value", PyPropertyValue::from_rust(val))?;
                    }
                    Err(_) => {
                        elem_dict.set_item("value", PyBytes::new(py, value_bytes))?;
                    }
                }
                elem_dict.set_item("error", py.None())?;
            } else if let Some((ec, ev)) = elem.error {
                elem_dict.set_item("value", py.None())?;
                let err_tuple = (PyErrorClass { inner: ec }, PyErrorCode { inner: ev });
                elem_dict.set_item("error", err_tuple)?;
            } else {
                elem_dict.set_item("value", py.None())?;
                elem_dict.set_item("error", py.None())?;
            }
            results_list.append(elem_dict)?;
        }
        obj_dict.set_item("results", results_list)?;
        outer.append(obj_dict)?;
    }
    Ok(outer.into_any().unbind())
}

/// Convert Python WPM specs to Rust WriteAccessSpecification list.
#[allow(clippy::type_complexity)]
pub(crate) fn py_to_wpm_specs(
    specs: Vec<(
        PyObjectIdentifier,
        Vec<(
            PyPropertyIdentifier,
            PyPropertyValue,
            Option<u8>,
            Option<u32>,
        )>,
    )>,
) -> Vec<WriteAccessSpecification> {
    specs
        .into_iter()
        .map(|(oid, props)| {
            let list_of_properties = props
                .into_iter()
                .map(|(pid, val, priority, array_index)| {
                    let mut buf = BytesMut::new();
                    let _ = encode_property_value(&mut buf, &val.inner);
                    BACnetPropertyValue {
                        property_identifier: pid.to_rust(),
                        property_array_index: array_index,
                        value: buf.to_vec(),
                        priority,
                    }
                })
                .collect();
            WriteAccessSpecification {
                object_identifier: oid.to_rust(),
                list_of_properties,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
