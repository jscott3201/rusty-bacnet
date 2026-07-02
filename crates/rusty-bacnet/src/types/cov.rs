use super::*;

// ---------------------------------------------------------------------------
// COV Notification — read-only wrapper
// ---------------------------------------------------------------------------

/// An incoming COV notification from a server.
#[pyclass(name = "CovNotification", frozen)]
pub struct PyCovNotification {
    inner: ReceivedCOVNotification,
}

#[pymethods]
impl PyCovNotification {
    #[getter]
    fn subscriber_process_identifier(&self) -> u32 {
        self.inner.notification.subscriber_process_identifier
    }

    #[getter]
    fn initiating_device_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.notification.initiating_device_identifier)
    }

    #[getter]
    fn monitored_object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.notification.monitored_object_identifier)
    }

    #[getter]
    fn time_remaining(&self) -> u32 {
        self.inner.notification.time_remaining
    }

    #[getter]
    fn delivery(&self) -> &'static str {
        match self.inner.delivery {
            COVNotificationDelivery::Confirmed => "confirmed",
            COVNotificationDelivery::Unconfirmed => "unconfirmed",
        }
    }

    #[getter]
    fn source_mac(&self, py: Python<'_>) -> Py<PyAny> {
        PyBytes::new(py, &self.inner.source_mac).into_any().unbind()
    }

    #[getter]
    fn source_network(&self) -> Option<u16> {
        self.inner.source_network
    }

    #[getter]
    fn source_address(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.inner
            .source_address
            .as_ref()
            .map(|mac| PyBytes::new(py, mac).into_any().unbind())
    }

    /// List of property values as dicts with `property_id`, `array_index`, `value`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for pv in &self.inner.notification.list_of_values {
            let dict = PyDict::new(py);
            dict.set_item(
                "property_id",
                PyPropertyIdentifier {
                    inner: pv.property_identifier,
                },
            )?;
            dict.set_item("array_index", pv.property_array_index)?;
            if !pv.value.is_empty() {
                match decode_application_value(&pv.value, 0) {
                    Ok((val, _)) => {
                        dict.set_item("value", PyPropertyValue::from_rust(val))?;
                    }
                    Err(_) => {
                        dict.set_item("value", PyBytes::new(py, &pv.value))?;
                    }
                }
            } else {
                dict.set_item("value", py.None())?;
            }
            list.append(dict)?;
        }
        Ok(list.into_any().unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "CovNotification(device={}, object={}, delivery={}, remaining={})",
            self.inner
                .notification
                .initiating_device_identifier
                .instance_number(),
            self.inner.notification.monitored_object_identifier,
            self.delivery(),
            self.inner.notification.time_remaining
        )
    }
}

// ---------------------------------------------------------------------------
// COV Notification async iterator
// ---------------------------------------------------------------------------

/// Async iterator yielding COV notifications from a broadcast channel.
#[pyclass(name = "CovNotificationIterator")]
pub struct PyCovNotificationIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<ReceivedCOVNotification>>>,
}

impl PyCovNotificationIterator {
    pub fn new(rx: broadcast::Receiver<ReceivedCOVNotification>) -> Self {
        Self {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

#[pymethods]
impl PyCovNotificationIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rx = self.rx.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = rx.lock().await;
            loop {
                match guard.recv().await {
                    Ok(notif) => {
                        return Ok(PyCovNotification { inner: notif });
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("COV notification iterator lagged, skipped {n} messages");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(PyStopAsyncIteration::new_err("channel closed"));
                    }
                }
            }
        })
    }
}
