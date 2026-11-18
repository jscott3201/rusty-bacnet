use super::*;
use crate::event_stream::LossyReceiver;
use pyo3::IntoPyObjectExt;

// ---------------------------------------------------------------------------
// COV Notification — read-only wrapper
// ---------------------------------------------------------------------------

/// `(date, time)` as Python reads them.
type DateTimeValue = ((u16, u8, u8, u8), (u8, u8, u8, u8));

/// `delivery` as Python reads it.
fn delivery_name(delivery: COVNotificationDelivery) -> &'static str {
    match delivery {
        COVNotificationDelivery::Confirmed => "confirmed",
        COVNotificationDelivery::Unconfirmed => "unconfirmed",
    }
}

/// One reported value of an object of type `object_type`, as a dict with
/// `property_id`, `array_index` and `value`. The value decodes as a
/// `read_property` result does; octets whose framing is broken come back as
/// `bytes`.
fn value_dict<'py>(
    py: Python<'py>,
    object_type: bacnet_enums::ObjectType,
    property: bacnet_enums::PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("property_id", PyPropertyIdentifier { inner: property })?;
    dict.set_item("array_index", array_index)?;
    match decode_read_value(object_type, property, array_index, octets) {
        Ok(value) => dict.set_item("value", value)?,
        Err(_) => dict.set_item("value", PyBytes::new(py, octets))?,
    }
    Ok(dict)
}

/// An incoming COV notification from a server, about one object.
#[pyclass(name = "CovNotification", module = "rusty_bacnet", frozen)]
pub struct PyCovNotification {
    inner: ReceivedCOVNotification,
}

#[pymethods]
impl PyCovNotification {
    /// `"object"`: a ConfirmedCOVNotification or UnconfirmedCOVNotification,
    /// about one object.
    #[getter]
    fn kind(&self) -> &'static str {
        "object"
    }

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
        delivery_name(self.inner.delivery)
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
        let object_type = self
            .inner
            .notification
            .monitored_object_identifier
            .object_type();
        let list = PyList::empty(py);
        for pv in &self.inner.notification.list_of_values {
            list.append(value_dict(
                py,
                object_type,
                pv.property_identifier,
                pv.property_array_index,
                &pv.value,
            )?)?;
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

/// An incoming COV notification about several objects: a
/// ConfirmedCOVNotificationMultiple or UnconfirmedCOVNotificationMultiple,
/// which a SubscribeCOVPropertyMultiple subscription brings (#2654).
#[pyclass(name = "CovNotificationMultiple", module = "rusty_bacnet", frozen)]
pub struct PyCovNotificationMultiple {
    inner: ReceivedCOVNotificationMultiple,
}

#[pymethods]
impl PyCovNotificationMultiple {
    /// `"multiple"`: a COV notification about several objects.
    #[getter]
    fn kind(&self) -> &'static str {
        "multiple"
    }

    #[getter]
    fn subscriber_process_identifier(&self) -> u32 {
        self.inner.notification.subscriber_process_identifier
    }

    #[getter]
    fn initiating_device_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.notification.initiating_device_identifier)
    }

    #[getter]
    fn time_remaining(&self) -> u32 {
        self.inner.notification.time_remaining
    }

    /// When the newest change the notification reports happened, as
    /// `((year, month, day, day_of_week), (hour, minute, second,
    /// hundredths))`; `None` when no value carries a time of change.
    #[getter]
    fn timestamp(&self) -> Option<DateTimeValue> {
        self.inner
            .notification
            .timestamp
            .as_ref()
            .map(|(date, time)| (date_value(date), time_value(time)))
    }

    #[getter]
    fn delivery(&self) -> &'static str {
        delivery_name(self.inner.delivery)
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

    /// The objects reported, in order, as dicts with
    /// `monitored_object_identifier` and `values`: each value a dict with
    /// `property_id`, `array_index`, `value` (decoded as for
    /// `CovNotification.values`) and `time_of_change`, an
    /// `(hour, minute, second, hundredths)` tuple, or `None` when the
    /// subscription didn't ask for timestamps.
    #[getter]
    fn objects(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let objects = PyList::empty(py);
        for item in &self.inner.notification.list_of_cov_notifications {
            let object = item.monitored_object_identifier;
            let values = PyList::empty(py);
            for value in &item.list_of_values {
                let dict = value_dict(
                    py,
                    object.object_type(),
                    value.property_identifier,
                    value.property_array_index,
                    &value.value,
                )?;
                dict.set_item(
                    "time_of_change",
                    value.time_of_change.as_ref().map(time_value),
                )?;
                values.append(dict)?;
            }
            let dict = PyDict::new(py);
            dict.set_item(
                "monitored_object_identifier",
                PyObjectIdentifier::from_rust(object),
            )?;
            dict.set_item("values", values)?;
            objects.append(dict)?;
        }
        Ok(objects.into_any().unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "CovNotificationMultiple(device={}, objects={}, delivery={}, remaining={})",
            self.inner
                .notification
                .initiating_device_identifier
                .instance_number(),
            self.inner.notification.list_of_cov_notifications.len(),
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

/// A notification as the iterator yields it: the class for its kind.
enum PyCovEvent {
    Object(PyCovNotification),
    Multiple(PyCovNotificationMultiple),
}

impl From<COVEvent> for PyCovEvent {
    fn from(event: COVEvent) -> Self {
        match event {
            COVEvent::Object(inner) => Self::Object(PyCovNotification { inner }),
            COVEvent::Multiple(inner) => Self::Multiple(PyCovNotificationMultiple { inner }),
        }
    }
}

impl<'py> IntoPyObject<'py> for PyCovEvent {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Object(notification) => notification.into_bound_py_any(py),
            Self::Multiple(notification) => notification.into_bound_py_any(py),
        }
    }
}

// ---------------------------------------------------------------------------
// COV Notification async iterator
// ---------------------------------------------------------------------------

/// Async iterator yielding COV notifications of both kinds from a broadcast
/// channel, in the order they arrived. A notification whose `__anext__` was
/// cancelled while it was being delivered comes back on the next call
/// (#1858).
#[pyclass(name = "CovNotificationIterator", module = "rusty_bacnet", frozen)]
pub struct PyCovNotificationIterator(LossyReceiver<COVEvent>);

impl PyCovNotificationIterator {
    pub fn new(rx: broadcast::Receiver<COVEvent>) -> Self {
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
                    .map(|next| next.map(PyCovEvent::from))
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
