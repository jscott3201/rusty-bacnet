use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // Object management
    // -----------------------------------------------------------------------

    /// Delete an object on a remote device.
    #[pyo3(signature = (address, object_id))]
    fn delete_object<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.delete_object(&mac, oid).await.map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Create an object on a remote device.
    ///
    /// `object_specifier` accepts either an `ObjectType` (server picks instance)
    /// or an `ObjectIdentifier` (specific instance).
    /// `initial_values` is an optional list of `(PropertyIdentifier, PropertyValue, priority, array_index)` tuples.
    #[pyo3(signature = (address, object_specifier, initial_values=None))]
    #[allow(clippy::type_complexity)]
    fn create_object<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_specifier: Bound<'py, PyAny>,
        initial_values: Option<
            Vec<(
                PyPropertyIdentifier,
                PyPropertyValue,
                Option<u8>,
                Option<u32>,
            )>,
        >,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();

        // Determine specifier type
        let specifier = if let Ok(ot) = object_specifier.extract::<PyObjectType>() {
            ObjectSpecifier::Type(ot.to_rust())
        } else if let Ok(oid) = object_specifier.extract::<PyObjectIdentifier>() {
            ObjectSpecifier::Identifier(oid.to_rust())
        } else {
            return Err(PyValueError::new_err(
                "object_specifier must be ObjectType or ObjectIdentifier",
            ));
        };

        let init_vals: Vec<BACnetPropertyValue> = initial_values
            .unwrap_or_default()
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

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .create_object(&mac, specifier, init_vals)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // Device management
    // -----------------------------------------------------------------------

    /// Send a DeviceCommunicationControl request.
    #[pyo3(signature = (address, enable_disable, time_duration=None, password=None))]
    fn device_communication_control<'py>(
        &self,
        py: Python<'py>,
        address: String,
        enable_disable: PyEnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let ed = enable_disable.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.device_communication_control(&mac, ed, time_duration, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Send a ReinitializeDevice request.
    #[pyo3(signature = (address, reinitialized_state, password=None))]
    fn reinitialize_device<'py>(
        &self,
        py: Python<'py>,
        address: String,
        reinitialized_state: PyReinitializedState,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let state = reinitialized_state.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.reinitialize_device(&mac, state, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Alarms / Events
    // -----------------------------------------------------------------------

    /// Acknowledge an alarm on a remote device.
    #[pyo3(signature = (address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, acknowledgment_source))]
    #[allow(clippy::too_many_arguments)]
    fn acknowledge_alarm<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledging_process_identifier: u32,
        event_object_identifier: PyObjectIdentifier,
        event_state_acknowledged: u32,
        acknowledgment_source: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = event_object_identifier.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            c.acknowledge_alarm(
                &mac,
                acknowledging_process_identifier,
                oid,
                event_state_acknowledged,
                &acknowledgment_source,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    /// Get event information from a remote device.
    #[pyo3(signature = (address, last_received_object_identifier=None))]
    fn get_event_information<'py>(
        &self,
        py: Python<'py>,
        address: String,
        last_received_object_identifier: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let last_oid = last_received_object_identifier.map(|o| o.to_rust());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let raw = c
                .get_event_information(&mac, last_oid)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| Ok(PyBytes::new(py, &raw).into_any().unbind()))
        })
    }

    // -----------------------------------------------------------------------
    // ReadRange
    // -----------------------------------------------------------------------

    /// Read a range of items from a list or log object.
    ///
    /// `range_type` is `"position"`, `"sequence"`, or `None` (no range).
    #[pyo3(signature = (address, object_id, property_id, array_index=None, range_type=None, reference_index=None, reference_seq=None, count=None))]
    #[allow(clippy::too_many_arguments)]
    fn read_range<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_id: PyObjectIdentifier,
        property_id: PyPropertyIdentifier,
        array_index: Option<u32>,
        range_type: Option<String>,
        reference_index: Option<u32>,
        reference_seq: Option<u32>,
        count: Option<i32>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let oid = object_id.to_rust();
        let pid = property_id.to_rust();

        let range = match range_type.as_deref() {
            Some("position") => Some(RangeSpec::ByPosition {
                reference_index: reference_index.unwrap_or(0),
                count: count.unwrap_or(0),
            }),
            Some("sequence") => Some(RangeSpec::BySequenceNumber {
                reference_seq: reference_seq.unwrap_or(0),
                count: count.unwrap_or(0),
            }),
            Some(other) => {
                return Err(PyValueError::new_err(format!(
                    "range_type must be 'position', 'sequence', or None, got '{other}'"
                )));
            }
            None => None,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let ack = c
                .read_range(&mac, oid, pid, array_index, range)
                .await
                .map_err(to_py_err)?;
            Python::attach(|py| {
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
                Ok(dict.into_any().unbind())
            })
        })
    }
}
