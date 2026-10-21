use super::*;
use crate::event_stream::LossyReceiver;

// ---------------------------------------------------------------------------
// COV Notification — read-only wrapper
// ---------------------------------------------------------------------------

/// An incoming COV notification from a server.
#[pyclass(name = "CovNotification", module = "rusty_bacnet", frozen)]
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
    ///
    /// Each value decodes as a `read_property` result does; octets whose
    /// framing is broken come back as `bytes`.
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
            match decode_read_value(
                self.inner
                    .notification
                    .monitored_object_identifier
                    .object_type(),
                pv.property_identifier,
                pv.property_array_index,
                &pv.value,
            ) {
                Ok(val) => {
                    dict.set_item("value", val)?;
                }
                Err(_) => {
                    dict.set_item("value", PyBytes::new(py, &pv.value))?;
                }
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

    /// Refuses `copy` and `pickle`: a notification as received, with no
    /// constructor to rebuild it. See [`super::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(super::not_picklable(slf.as_any()))
    }
}

// ---------------------------------------------------------------------------
// COV Notification async iterator
// ---------------------------------------------------------------------------

/// Async iterator yielding COV notifications from a broadcast channel. A
/// notification whose `__anext__` was cancelled while it was being
/// delivered comes back on the next call (#1858).
#[pyclass(name = "CovNotificationIterator", module = "rusty_bacnet", frozen)]
pub struct PyCovNotificationIterator(LossyReceiver<ReceivedCOVNotification>);

impl PyCovNotificationIterator {
    pub fn new(rx: broadcast::Receiver<ReceivedCOVNotification>) -> Self {
        Self(LossyReceiver::new(rx))
    }
}

#[pymethods]
impl PyCovNotificationIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Refuses `copy` and `pickle`: a live subscription to this client's
    /// notifications. See [`super::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(super::not_picklable(slf.as_any()))
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.0.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(|inner| PyCovNotification { inner }))
                    .ok_or_else(|| crate::event_stream::ended("COV notifications"))
            },
            put_back,
        )
    }

    /// Notifications this iterator lost: because it fell behind the client's
    /// COV channel, or dropped on the way to a read, as when its event loop
    /// closed first.
    #[getter]
    fn missed(&self) -> u64 {
        self.0.missed()
    }
}
