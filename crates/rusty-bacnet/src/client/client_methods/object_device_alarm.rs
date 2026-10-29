use super::super::*;
use bacnet_services::alarm_event::GetEventInformationAck;

fn build_acknowledge_alarm_request(
    acknowledging_process_identifier: u32,
    event_object_identifier: bacnet_types::primitives::ObjectIdentifier,
    event_state_acknowledged: bacnet_types::enums::EventState,
    timestamp: BACnetTimeStamp,
    acknowledgment_source: String,
    time_of_acknowledgment: BACnetTimeStamp,
) -> AcknowledgeAlarmRequest {
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier,
        event_object_identifier,
        event_state_acknowledged,
        timestamp,
        acknowledgment_source,
        time_of_acknowledgment,
    }
}

/// A GetEventInformation answer as the dict `get_event_information`
/// returns.
fn event_information_dict(py: Python<'_>, ack: &GetEventInformationAck) -> PyResult<Py<PyAny>> {
    let summaries = pyo3::types::PyList::empty(py);
    for summary in &ack.list_of_event_summaries {
        let dict = PyDict::new(py);
        dict.set_item(
            "object_id",
            PyObjectIdentifier::from_rust(summary.object_identifier),
        )?;
        dict.set_item(
            "event_state",
            PyEventState {
                inner: summary.event_state,
            },
        )?;
        dict.set_item(
            "acknowledged_transitions",
            crate::types::PyBitString::event_transitions(summary.acknowledged_transitions),
        )?;
        let timestamps = summary
            .event_timestamps
            .iter()
            .map(|stamp| Py::new(py, PyBACnetTimeStamp::from_rust(stamp.clone())))
            .collect::<PyResult<Vec<_>>>()?;
        dict.set_item("event_timestamps", timestamps)?;
        dict.set_item(
            "notify_type",
            crate::types::PyNotifyType {
                inner: summary.notify_type,
            },
        )?;
        dict.set_item(
            "event_enable",
            crate::types::PyBitString::event_transitions(summary.event_enable),
        )?;
        dict.set_item("event_priorities", summary.event_priorities.to_vec())?;
        summaries.append(dict)?;
    }
    let result = PyDict::new(py);
    result.set_item("summaries", summaries)?;
    result.set_item("more_events", ack.more_events)?;
    Ok(result.into_any().unbind())
}

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
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = object_id.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.delete_object(&mac, oid).await.map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Create an object on a remote device.
    ///
    /// `object_specifier` accepts either an `ObjectType` (server picks instance)
    /// or an `ObjectIdentifier` (specific instance).
    /// `initial_values` is an optional list of `(PropertyIdentifier, PropertyValue, priority, array_index)` tuples.
    /// Returns the `ObjectIdentifier` of the created object from the acknowledgment.
    #[pyo3(signature = (address, object_specifier, initial_values=None))]
    fn create_object<'py>(
        &self,
        py: Python<'py>,
        address: String,
        object_specifier: Bound<'py, PyAny>,
        initial_values: Option<Vec<PyPropertyWrite>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);

        // Determine specifier type
        let specifier = match object_specifier.extract::<PyObjectType>() {
            Ok(ot) => ObjectSpecifier::Type(ot.to_rust()),
            _ => match object_specifier.extract::<PyObjectIdentifier>() {
                Ok(oid) => ObjectSpecifier::Identifier(oid.to_rust()),
                _ => {
                    return Err(PyValueError::new_err(
                        "object_specifier must be ObjectType or ObjectIdentifier",
                    ));
                }
            },
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

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            // The client decodes the ACK; a reply that doesn't decode raises,
            // though the server may already have created the object.
            let created = c
                .create_object(&mac, specifier, init_vals)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| {
                Ok(Py::new(py, PyObjectIdentifier::from_rust(created))?.into_any())
            })
        })
    }

    // -----------------------------------------------------------------------
    // Device management
    // -----------------------------------------------------------------------

    /// Send a DeviceCommunicationControl request. A password over 20
    /// characters (#2085), or an empty one (#2094), raises ValueError before
    /// anything is sent.
    #[pyo3(signature = (address, enable_disable, time_duration=None, password=None))]
    fn device_communication_control<'py>(
        &self,
        py: Python<'py>,
        address: String,
        enable_disable: PyEnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::reinitialize::password_limit("password", &password)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let ed = enable_disable.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.device_communication_control(&mac, ed, time_duration, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Send a ReinitializeDevice request. A password over 20 characters
    /// (#2085), or an empty one (#2094), raises ValueError before anything
    /// is sent.
    #[pyo3(signature = (address, reinitialized_state, password=None))]
    fn reinitialize_device<'py>(
        &self,
        py: Python<'py>,
        address: String,
        reinitialized_state: PyReinitializedState,
        password: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        crate::reinitialize::password_limit("password", &password)?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let state = reinitialized_state.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            c.reinitialize_device(&mac, state, password)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    // -----------------------------------------------------------------------
    // Alarms / Events
    // -----------------------------------------------------------------------

    /// Acknowledge an alarm on a remote device with both caller-supplied timestamps.
    ///
    /// `timestamp` must exactly echo the original event-notification timestamp;
    /// `time_of_acknowledgment` is the caller-selected acknowledgment time.
    #[pyo3(signature = (address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, timestamp, acknowledgment_source, time_of_acknowledgment))]
    fn acknowledge_alarm_request<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledging_process_identifier: u32,
        event_object_identifier: PyObjectIdentifier,
        event_state_acknowledged: PyEventState,
        timestamp: PyBACnetTimeStamp,
        acknowledgment_source: String,
        time_of_acknowledgment: PyBACnetTimeStamp,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let mac = peers.parse(&address)?;
        let request = build_acknowledge_alarm_request(
            acknowledging_process_identifier,
            event_object_identifier.to_rust(),
            event_state_acknowledged.to_rust(),
            timestamp.to_rust().clone(),
            acknowledgment_source,
            time_of_acknowledgment.to_rust().clone(),
        );

        let future = async move {
            let c = started(&inner).await?;
            c.acknowledge_alarm_request(&mac, &request)
                .await
                .map_err(to_py_err)?;
            Ok(())
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Deprecated compatibility helper for acknowledging an alarm.
    ///
    /// This method fabricates SequenceNumber(0) for both timestamps. Use
    /// `acknowledge_alarm_request` with exact caller-supplied timestamps.
    #[pyo3(signature = (address, acknowledging_process_identifier, event_object_identifier, event_state_acknowledged, acknowledgment_source))]
    #[allow(deprecated)]
    fn acknowledge_alarm<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledging_process_identifier: u32,
        event_object_identifier: PyObjectIdentifier,
        event_state_acknowledged: PyEventState,
        acknowledgment_source: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        PyErr::warn(
            py,
            &py.get_type::<PyDeprecationWarning>(),
            c"BACnetClient.acknowledge_alarm() is deprecated; use acknowledge_alarm_request() with exact event and acknowledgment timestamps",
            2,
        )?;
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let oid = event_object_identifier.to_rust();
        let event_state_acknowledged = event_state_acknowledged.to_rust();

        let future = async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
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
        };
        crate::py_async::future_into_py(py, crate::unit_result(future))
    }

    /// Get event information from a remote device: one GetEventInformation
    /// answer, decoded.
    ///
    /// Returns `{"summaries": [...], "more_events": bool}`. Each summary is
    /// a dict with `object_id`, `event_state` (an `EventState`),
    /// `acknowledged_transitions` and `event_enable` (`BitString`s naming
    /// `TO_OFFNORMAL`, `TO_FAULT`, `TO_NORMAL`), `event_timestamps` (three
    /// `BACnetTimeStamp`s), `notify_type` (a `NotifyType`) and
    /// `event_priorities` (three ints). While `more_events` is true, call
    /// again with the last summary's `object_id`.
    #[pyo3(signature = (address, last_received_object_identifier=None))]
    fn get_event_information<'py>(
        &self,
        py: Python<'py>,
        address: String,
        last_received_object_identifier: Option<PyObjectIdentifier>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let peers = PeerAddresses::for_transport(&self.transport_type);
        let last_oid = last_received_object_identifier.map(|o| o.to_rust());

        crate::py_async::future_into_py(py, async move {
            let mac = peers.parse(&address)?;
            let c = started(&inner).await?;
            let ack = c
                .get_event_information(&mac, last_oid)
                .await
                .map_err(to_py_err)?;
            crate::py_async::attach(|py| event_information_dict(py, &ack))
        })
    }
}

#[cfg(test)]
mod acknowledgment_request_tests {
    use super::*;
    use bacnet_types::enums::{EventState, ObjectType};
    use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

    #[test]
    fn python_request_builder_preserves_mixed_timestamp_choices_and_metadata() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 91).unwrap();
        let event_timestamp = BACnetTimeStamp::Time(Time {
            hour: 7,
            minute: 8,
            second: 9,
            hundredths: 10,
        });
        let acknowledgment_timestamp = BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 14,
                day: 34,
                day_of_week: Date::UNSPECIFIED,
            },
            time: Time {
                hour: 11,
                minute: 12,
                second: 13,
                hundredths: 14,
            },
        };

        let request = build_acknowledge_alarm_request(
            0x1020_3040,
            oid,
            EventState::HIGH_LIMIT,
            event_timestamp.clone(),
            "operator-console".into(),
            acknowledgment_timestamp.clone(),
        );

        assert_eq!(request.acknowledging_process_identifier, 0x1020_3040);
        assert_eq!(request.event_object_identifier, oid);
        assert_eq!(request.event_state_acknowledged, EventState::HIGH_LIMIT);
        assert_eq!(request.timestamp, event_timestamp);
        assert_eq!(request.acknowledgment_source, "operator-console");
        assert_eq!(request.time_of_acknowledgment, acknowledgment_timestamp);
    }
}
