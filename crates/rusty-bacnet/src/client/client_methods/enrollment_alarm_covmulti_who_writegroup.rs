use super::super::*;

#[pymethods]
impl BACnetClient {
    // -----------------------------------------------------------------------
    // GetEnrollmentSummary
    // -----------------------------------------------------------------------

    /// Get enrollment summary from a remote device.
    ///
    /// `acknowledgment_filter`: 0=all, 1=acked, 2=not-acked.
    /// Returns a list of dicts with `object_id`, `event_type`, `event_state`, `priority`, `notification_class`.
    #[pyo3(signature = (address, acknowledgment_filter=0, event_state_filter=None, event_type_filter=None, min_priority=None, max_priority=None, notification_class_filter=None))]
    #[allow(clippy::too_many_arguments)]
    fn get_enrollment_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
        acknowledgment_filter: u32,
        event_state_filter: Option<PyEventState>,
        event_type_filter: Option<PyEventType>,
        min_priority: Option<u8>,
        max_priority: Option<u8>,
        notification_class_filter: Option<u16>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let es = event_state_filter.map(|e| e.to_rust());
        let et = event_type_filter.map(|e| e.to_rust());
        let pf = match (min_priority, max_priority) {
            (Some(min), Some(max)) => Some(PriorityFilter {
                min_priority: min,
                max_priority: max,
            }),
            _ => None,
        };

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = GetEnrollmentSummaryRequest {
                acknowledgment_filter,
                enrollment_filter: None, // not exposed in Python API
                event_state_filter: es,
                event_type_filter: et,
                priority_filter: pf,
                notification_class_filter,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY, &buf)
                .await
                .map_err(to_py_err)?;
            let ack = GetEnrollmentSummaryAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "event_type",
                        PyEventType {
                            inner: entry.event_type,
                        },
                    )?;
                    dict.set_item(
                        "event_state",
                        PyEventState {
                            inner: entry.event_state,
                        },
                    )?;
                    dict.set_item("priority", entry.priority)?;
                    dict.set_item("notification_class", entry.notification_class)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // GetAlarmSummary
    // -----------------------------------------------------------------------

    /// Get alarm summary from a remote device (deprecated service).
    ///
    /// Returns a list of dicts with `object_id`, `alarm_state`, `acknowledged_transitions`.
    #[pyo3(signature = (address,))]
    fn get_alarm_summary<'py>(
        &self,
        py: Python<'py>,
        address: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mac = parse_address(&address)?;
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let resp = c
                .confirmed_request(&mac, ConfirmedServiceChoice::GET_ALARM_SUMMARY, &[])
                .await
                .map_err(to_py_err)?;
            let ack = GetAlarmSummaryAck::decode(&resp).map_err(to_py_err)?;
            Python::attach(|py| {
                let list = pyo3::types::PyList::empty(py);
                for entry in &ack.entries {
                    let dict = PyDict::new(py);
                    dict.set_item(
                        "object_id",
                        PyObjectIdentifier::from_rust(entry.object_identifier),
                    )?;
                    dict.set_item(
                        "alarm_state",
                        PyEventState {
                            inner: entry.alarm_state,
                        },
                    )?;
                    let (unused_bits, ref data) = entry.acknowledged_transitions;
                    let trans_dict = PyDict::new(py);
                    trans_dict.set_item("unused_bits", unused_bits)?;
                    trans_dict.set_item("data", PyBytes::new(py, data))?;
                    dict.set_item("acknowledged_transitions", trans_dict)?;
                    list.append(dict)?;
                }
                Ok(list.into_any().unbind())
            })
        })
    }

    // -----------------------------------------------------------------------
    // SubscribeCOVPropertyMultiple
    // -----------------------------------------------------------------------

    /// Subscribe to COV notifications for multiple properties on multiple objects.
    ///
    /// `specs` is a list of `(ObjectIdentifier, [(PropertyIdentifier, array_index, cov_increment, timestamped), ...])`.
    /// `cov_increment` is an optional float; `timestamped` is a bool.
    #[pyo3(signature = (address, subscriber_process_identifier, specs, max_notification_delay=None, issue_confirmed_notifications=None))]
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn subscribe_cov_property_multiple<'py>(
        &self,
        py: Python<'py>,
        address: String,
        subscriber_process_identifier: u32,
        specs: Vec<(
            PyObjectIdentifier,
            Vec<(PyPropertyIdentifier, Option<u32>, Option<f32>, bool)>,
        )>,
        max_notification_delay: Option<u32>,
        issue_confirmed_notifications: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let rust_specs: Vec<COVSubscriptionSpecification> = specs
            .into_iter()
            .map(|(oid, refs)| COVSubscriptionSpecification {
                monitored_object_identifier: oid.to_rust(),
                list_of_cov_references: refs
                    .into_iter()
                    .map(|(pid, idx, inc, ts)| COVReference {
                        monitored_property: bacnet_services::common::PropertyReference {
                            property_identifier: pid.to_rust(),
                            property_array_index: idx,
                        },
                        cov_increment: inc,
                        timestamped: ts,
                    })
                    .collect(),
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
            let req = SubscribeCOVPropertyMultipleRequest {
                subscriber_process_identifier,
                max_notification_delay,
                issue_confirmed_notifications,
                list_of_cov_subscription_specifications: rust_specs,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.confirmed_request(
                &mac,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                &buf,
            )
            .await
            .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Who-Am-I
    // -----------------------------------------------------------------------

    /// Broadcast a Who-Am-I request.
    fn who_am_i<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let c = {
                let guard = inner.lock().await;
                Arc::clone(guard.as_ref().ok_or_else(|| {
                    PyRuntimeError::new_err("client not started — use 'async with'")
                })?)
            };
            let req = WhoAmIRequest;
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_AM_I, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // WriteGroup
    // -----------------------------------------------------------------------

    /// Send a WriteGroup request (unconfirmed).
    ///
    /// `change_list` is a list of `(channel_oid_or_none, override_priority_or_none, value_bytes)` tuples.
    #[pyo3(signature = (address, group_number, write_priority, change_list, inhibit_delay=None))]
    #[allow(clippy::too_many_arguments)]
    fn write_group<'py>(
        &self,
        py: Python<'py>,
        address: String,
        group_number: u32,
        write_priority: u8,
        change_list: Vec<(Option<PyObjectIdentifier>, Option<u8>, Vec<u8>)>,
        inhibit_delay: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if !(1..=16).contains(&write_priority) {
            return Err(PyValueError::new_err(format!(
                "write_priority must be 1-16, got {write_priority}"
            )));
        }
        let inner = self.inner.clone();
        let cl: Vec<GroupChannelValue> = change_list
            .into_iter()
            .map(|(ch, prio, val)| GroupChannelValue {
                channel: ch.map(|o| o.to_rust()),
                override_priority: prio,
                value: val,
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
            let req = WriteGroupRequest {
                group_number,
                write_priority,
                change_list: cl,
                inhibit_delay,
            };
            let mut buf = BytesMut::new();
            req.encode(&mut buf);
            c.unconfirmed_request(&mac, UnconfirmedServiceChoice::WRITE_GROUP, &buf)
                .await
                .map_err(to_py_err)?;
            Ok(())
        })
    }
}
