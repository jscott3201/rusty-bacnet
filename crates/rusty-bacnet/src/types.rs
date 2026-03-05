//! Python-facing type wrappers for BACnet enums, ObjectIdentifier, and PropertyValue.

#![allow(non_snake_case)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Instant;

use bytes::BytesMut;
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::Py;
use tokio::sync::broadcast;

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_services::common::{BACnetPropertyValue, PropertyReference};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums;
use bacnet_types::primitives;

// ---------------------------------------------------------------------------
// Macro: generates a frozen pyclass wrapper for a `bacnet_enum!` newtype.
//
// Constants are registered dynamically from `ALL_NAMED` during module init,
// so there is zero constant duplication between bacnet-types and rusty-bacnet.
// ---------------------------------------------------------------------------

macro_rules! py_bacnet_enum {
    ($py_name:literal, $PyStruct:ident, $RustType:ty, $raw_ty:ty) => {
        #[pyclass(name = $py_name, frozen, from_py_object)]
        #[derive(Clone)]
        pub struct $PyStruct {
            pub(crate) inner: $RustType,
        }

        impl $PyStruct {
            pub fn to_rust(&self) -> $RustType {
                self.inner
            }

            /// Set every named constant as a class attribute (e.g. `ObjectType.DEVICE`).
            pub fn register_constants(cls: &Bound<'_, PyAny>) -> PyResult<()> {
                for &(name, val) in <$RustType>::ALL_NAMED {
                    cls.setattr(name, Self { inner: val })?;
                }
                Ok(())
            }
        }

        #[pymethods]
        impl $PyStruct {
            /// Create from a raw integer value.
            #[staticmethod]
            fn from_raw(value: $raw_ty) -> Self {
                Self {
                    inner: <$RustType>::from_raw(value),
                }
            }

            /// Get the raw integer value.
            fn to_raw(&self) -> $raw_ty {
                self.inner.to_raw()
            }

            fn __repr__(&self) -> String {
                format!(concat!($py_name, ".{}"), self.inner)
            }

            fn __eq__(&self, other: &Self) -> bool {
                self.inner == other.inner
            }

            fn __hash__(&self) -> u64 {
                self.inner.to_raw() as u64
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Enum wrappers — constants are populated at module init time via ALL_NAMED.
// ---------------------------------------------------------------------------

py_bacnet_enum!("ObjectType", PyObjectType, enums::ObjectType, u32);
py_bacnet_enum!(
    "PropertyIdentifier",
    PyPropertyIdentifier,
    enums::PropertyIdentifier,
    u32
);
py_bacnet_enum!("ErrorClass", PyErrorClass, enums::ErrorClass, u16);
py_bacnet_enum!("ErrorCode", PyErrorCode, enums::ErrorCode, u16);
py_bacnet_enum!("EnableDisable", PyEnableDisable, enums::EnableDisable, u32);
py_bacnet_enum!(
    "ReinitializedState",
    PyReinitializedState,
    enums::ReinitializedState,
    u32
);
py_bacnet_enum!("Segmentation", PySegmentation, enums::Segmentation, u8);
py_bacnet_enum!(
    "LifeSafetyOperation",
    PyLifeSafetyOperation,
    enums::LifeSafetyOperation,
    u32
);
py_bacnet_enum!("EventState", PyEventState, enums::EventState, u32);
py_bacnet_enum!("EventType", PyEventType, enums::EventType, u32);
py_bacnet_enum!(
    "MessagePriority",
    PyMessagePriority,
    enums::MessagePriority,
    u32
);

// ---------------------------------------------------------------------------
// ObjectIdentifier
// ---------------------------------------------------------------------------

/// BACnet Object Identifier (type + instance).
///
/// Usage: `ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)`
#[pyclass(name = "ObjectIdentifier", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyObjectIdentifier {
    inner: primitives::ObjectIdentifier,
}

#[pymethods]
impl PyObjectIdentifier {
    #[new]
    fn new(object_type: &PyObjectType, instance: u32) -> PyResult<Self> {
        let oid = primitives::ObjectIdentifier::new(object_type.to_rust(), instance)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self { inner: oid })
    }

    /// The object type.
    #[getter]
    fn object_type(&self) -> PyObjectType {
        PyObjectType {
            inner: self.inner.object_type(),
        }
    }

    /// The instance number.
    #[getter]
    fn instance(&self) -> u32 {
        self.inner.instance_number()
    }

    fn __repr__(&self) -> String {
        format!(
            "ObjectIdentifier({}, {})",
            self.inner.object_type(),
            self.inner.instance_number()
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        (self.inner.object_type().to_raw() as u64) << 22 | self.inner.instance_number() as u64
    }
}

impl PyObjectIdentifier {
    pub fn to_rust(&self) -> primitives::ObjectIdentifier {
        self.inner
    }

    pub fn from_rust(oid: primitives::ObjectIdentifier) -> Self {
        Self { inner: oid }
    }
}

// ---------------------------------------------------------------------------
// PropertyValue — typed wrapper with explicit variant constructors.
// ---------------------------------------------------------------------------

/// BACnet application-layer value.
///
/// Use typed constructors to create values:
/// ```python
/// PropertyValue.real(72.5)
/// PropertyValue.unsigned(42)
/// PropertyValue.boolean(True)
/// PropertyValue.character_string("hello")
/// PropertyValue.null()
/// ```
///
/// Read results with `.value` (native Python type) and `.tag` (type name).
#[pyclass(name = "PropertyValue", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyPropertyValue {
    pub(crate) inner: primitives::PropertyValue,
}

impl PyPropertyValue {
    pub fn to_rust(&self) -> &primitives::PropertyValue {
        &self.inner
    }

    pub fn from_rust(value: primitives::PropertyValue) -> Self {
        Self { inner: value }
    }
}

#[pymethods]
impl PyPropertyValue {
    // -- Typed constructors --------------------------------------------------

    #[staticmethod]
    fn null() -> Self {
        Self {
            inner: primitives::PropertyValue::Null,
        }
    }

    #[staticmethod]
    fn boolean(value: bool) -> Self {
        Self {
            inner: primitives::PropertyValue::Boolean(value),
        }
    }

    #[staticmethod]
    fn unsigned(value: u64) -> Self {
        Self {
            inner: primitives::PropertyValue::Unsigned(value),
        }
    }

    #[staticmethod]
    fn signed(value: i32) -> Self {
        Self {
            inner: primitives::PropertyValue::Signed(value),
        }
    }

    #[staticmethod]
    fn real(value: f32) -> Self {
        Self {
            inner: primitives::PropertyValue::Real(value),
        }
    }

    #[staticmethod]
    fn double(value: f64) -> Self {
        Self {
            inner: primitives::PropertyValue::Double(value),
        }
    }

    #[staticmethod]
    fn character_string(value: String) -> Self {
        Self {
            inner: primitives::PropertyValue::CharacterString(value),
        }
    }

    #[staticmethod]
    fn octet_string(value: Vec<u8>) -> Self {
        Self {
            inner: primitives::PropertyValue::OctetString(value),
        }
    }

    #[staticmethod]
    fn enumerated(value: u32) -> Self {
        Self {
            inner: primitives::PropertyValue::Enumerated(value),
        }
    }

    #[staticmethod]
    fn object_identifier(oid: &PyObjectIdentifier) -> Self {
        Self {
            inner: primitives::PropertyValue::ObjectIdentifier(oid.to_rust()),
        }
    }

    // -- Accessors -----------------------------------------------------------

    /// The BACnet type tag (e.g. "real", "unsigned", "boolean").
    #[getter]
    fn tag(&self) -> &str {
        match &self.inner {
            primitives::PropertyValue::Null => "null",
            primitives::PropertyValue::Boolean(_) => "boolean",
            primitives::PropertyValue::Unsigned(_) => "unsigned",
            primitives::PropertyValue::Signed(_) => "signed",
            primitives::PropertyValue::Real(_) => "real",
            primitives::PropertyValue::Double(_) => "double",
            primitives::PropertyValue::OctetString(_) => "octet_string",
            primitives::PropertyValue::CharacterString(_) => "character_string",
            primitives::PropertyValue::BitString { .. } => "bit_string",
            primitives::PropertyValue::Enumerated(_) => "enumerated",
            primitives::PropertyValue::Date(_) => "date",
            primitives::PropertyValue::Time(_) => "time",
            primitives::PropertyValue::ObjectIdentifier(_) => "object_identifier",
            primitives::PropertyValue::List(_) => "list",
        }
    }

    /// The value as a native Python type (float, int, str, bool, bytes, etc.).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        property_value_to_py(py, &self.inner)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            primitives::PropertyValue::Null => "PropertyValue.null()".to_string(),
            primitives::PropertyValue::Boolean(b) => format!("PropertyValue.boolean({b})"),
            primitives::PropertyValue::Unsigned(u) => format!("PropertyValue.unsigned({u})"),
            primitives::PropertyValue::Signed(i) => format!("PropertyValue.signed({i})"),
            primitives::PropertyValue::Real(f) => format!("PropertyValue.real({f})"),
            primitives::PropertyValue::Double(f) => format!("PropertyValue.double({f})"),
            primitives::PropertyValue::CharacterString(s) => {
                format!("PropertyValue.character_string({s:?})")
            }
            primitives::PropertyValue::OctetString(b) => {
                format!("PropertyValue.octet_string(<{} bytes>)", b.len())
            }
            primitives::PropertyValue::BitString { data, .. } => {
                format!("PropertyValue.bit_string(<{} bytes>)", data.len())
            }
            primitives::PropertyValue::Enumerated(e) => format!("PropertyValue.enumerated({e})"),
            primitives::PropertyValue::Date(d) => {
                format!("PropertyValue.date({}/{}/{})", d.year, d.month, d.day)
            }
            primitives::PropertyValue::Time(t) => {
                format!("PropertyValue.time({}:{}:{})", t.hour, t.minute, t.second)
            }
            primitives::PropertyValue::ObjectIdentifier(oid) => {
                format!(
                    "PropertyValue.object_identifier({}, {})",
                    oid.object_type(),
                    oid.instance_number()
                )
            }
            primitives::PropertyValue::List(elements) => {
                format!("PropertyValue.list(<{} elements>)", elements.len())
            }
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.inner).hash(&mut h);
        format!("{:?}", self.inner).hash(&mut h);
        h.finish()
    }
}

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// Parse an address string to a MAC byte vector.
///
/// Supported formats:
/// - IPv4: `"192.168.1.100:47808"` → 6-byte MAC (4-byte IP + 2-byte port BE)
/// - IPv6: `"[::1]:47808"` → 18-byte MAC (16-byte IPv6 + 2-byte port BE)
/// - Hex:  `"01:02:03:04:05:06"` → raw bytes (for SC VMAC or Ethernet MAC)
pub fn parse_address(address: &str) -> PyResult<Vec<u8>> {
    // IPv6 bracket notation: [addr]:port
    if address.starts_with('[') {
        let close = address
            .find(']')
            .ok_or_else(|| PyValueError::new_err("IPv6 address missing closing bracket"))?;
        let ip_str = &address[1..close];
        let ip: std::net::Ipv6Addr = ip_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid IPv6 address: {e}")))?;
        let rest = &address[close + 1..];
        let port_str = rest
            .strip_prefix(':')
            .ok_or_else(|| PyValueError::new_err("expected ':port' after IPv6 address"))?;
        let port: u16 = port_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
        let mut mac = Vec::with_capacity(18);
        mac.extend_from_slice(&ip.octets());
        mac.extend_from_slice(&port.to_be_bytes());
        return Ok(mac);
    }

    // Hex colon notation: aa:bb:cc:dd:ee:ff (6 or more hex pairs)
    if address.contains(':')
        && address
            .split(':')
            .all(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let bytes: Result<Vec<u8>, _> = address
            .split(':')
            .map(|s| u8::from_str_radix(s, 16))
            .collect();
        return bytes.map_err(|e| PyValueError::new_err(format!("invalid hex address: {e}")));
    }

    // IPv4: ip:port
    let (ip_str, port_str) = address.rsplit_once(':').ok_or_else(|| {
        PyValueError::new_err("address must be 'ip:port', '[ipv6]:port', or 'aa:bb:...' hex")
    })?;
    let ip: Ipv4Addr = ip_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid IP address: {e}")))?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    Ok(mac)
}

// ---------------------------------------------------------------------------
// PropertyValue → native Python conversion (used by PropertyValue.value getter)
// ---------------------------------------------------------------------------

fn property_value_to_py(py: Python<'_>, value: &primitives::PropertyValue) -> PyResult<Py<PyAny>> {
    Ok(match value {
        primitives::PropertyValue::Null => py.None(),
        primitives::PropertyValue::Boolean(b) => {
            b.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Unsigned(u) => u.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Signed(i) => i.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Real(f) => (*f as f64).into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::Double(f) => f.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::CharacterString(s) => {
            s.into_pyobject(py)?.to_owned().into_any().unbind()
        }
        primitives::PropertyValue::Enumerated(e) => e.into_pyobject(py)?.into_any().unbind(),
        primitives::PropertyValue::OctetString(b) => PyBytes::new(py, b).into_any().unbind(),
        primitives::PropertyValue::ObjectIdentifier(oid) => {
            Py::new(py, PyObjectIdentifier::from_rust(*oid))?.into_any()
        }
        primitives::PropertyValue::BitString { unused_bits, data } => {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("unused_bits", unused_bits)?;
            dict.set_item("data", PyBytes::new(py, data))?;
            dict.into_any().unbind()
        }
        primitives::PropertyValue::Date(d) => (d.year, d.month, d.day, d.day_of_week)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        primitives::PropertyValue::Time(t) => (t.hour, t.minute, t.second, t.hundredths)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        primitives::PropertyValue::List(elements) => {
            let list = pyo3::types::PyList::empty(py);
            for elem in elements {
                list.append(property_value_to_py(py, elem)?)?;
            }
            list.into_any().unbind()
        }
    })
}

// ---------------------------------------------------------------------------
// DiscoveredDevice — read-only wrapper for discovered BACnet devices
// ---------------------------------------------------------------------------

/// A discovered BACnet device from WhoIs/IAm.
#[pyclass(name = "DiscoveredDevice", frozen)]
pub struct PyDiscoveredDevice {
    inner: DiscoveredDevice,
    created: Instant,
}

#[pymethods]
impl PyDiscoveredDevice {
    #[getter]
    fn object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.object_identifier)
    }

    #[getter]
    fn mac_address(&self) -> Vec<u8> {
        self.inner.mac_address.to_vec()
    }

    #[getter]
    fn max_apdu_length(&self) -> u32 {
        self.inner.max_apdu_length
    }

    #[getter]
    fn segmentation_supported(&self) -> PySegmentation {
        PySegmentation {
            inner: self.inner.segmentation_supported,
        }
    }

    #[getter]
    fn vendor_id(&self) -> u16 {
        self.inner.vendor_id
    }

    #[getter]
    fn seconds_since_seen(&self) -> f64 {
        self.created.elapsed().as_secs_f64()
    }

    fn __repr__(&self) -> String {
        format!(
            "DiscoveredDevice({}, instance={}, vendor={})",
            self.inner.object_identifier.object_type(),
            self.inner.object_identifier.instance_number(),
            self.inner.vendor_id
        )
    }
}

impl PyDiscoveredDevice {
    pub fn from_rust(dev: DiscoveredDevice) -> Self {
        Self {
            created: dev.last_seen,
            inner: dev,
        }
    }
}

// ---------------------------------------------------------------------------
// COV Notification — read-only wrapper
// ---------------------------------------------------------------------------

/// An incoming COV notification from a server.
#[pyclass(name = "CovNotification", frozen)]
pub struct PyCovNotification {
    inner: COVNotificationRequest,
}

#[pymethods]
impl PyCovNotification {
    #[getter]
    fn subscriber_process_identifier(&self) -> u32 {
        self.inner.subscriber_process_identifier
    }

    #[getter]
    fn initiating_device_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.initiating_device_identifier)
    }

    #[getter]
    fn monitored_object_identifier(&self) -> PyObjectIdentifier {
        PyObjectIdentifier::from_rust(self.inner.monitored_object_identifier)
    }

    #[getter]
    fn time_remaining(&self) -> u32 {
        self.inner.time_remaining
    }

    /// List of property values as dicts with `property_id`, `array_index`, `value`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for pv in &self.inner.list_of_values {
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
            "CovNotification(device={}, object={}, remaining={})",
            self.inner.initiating_device_identifier.instance_number(),
            self.inner.monitored_object_identifier,
            self.inner.time_remaining
        )
    }
}

// ---------------------------------------------------------------------------
// COV Notification async iterator
// ---------------------------------------------------------------------------

/// Async iterator yielding COV notifications from a broadcast channel.
#[pyclass(name = "CovNotificationIterator")]
pub struct PyCovNotificationIterator {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<COVNotificationRequest>>>,
}

impl PyCovNotificationIterator {
    pub fn new(rx: broadcast::Receiver<COVNotificationRequest>) -> Self {
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
// Module registration
// ---------------------------------------------------------------------------

/// Register all type classes with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Enum types — add class then populate constants from ALL_NAMED.
    m.add_class::<PyObjectType>()?;
    PyObjectType::register_constants(&m.getattr("ObjectType")?)?;

    m.add_class::<PyPropertyIdentifier>()?;
    PyPropertyIdentifier::register_constants(&m.getattr("PropertyIdentifier")?)?;

    m.add_class::<PyErrorClass>()?;
    PyErrorClass::register_constants(&m.getattr("ErrorClass")?)?;

    m.add_class::<PyErrorCode>()?;
    PyErrorCode::register_constants(&m.getattr("ErrorCode")?)?;

    m.add_class::<PyEnableDisable>()?;
    PyEnableDisable::register_constants(&m.getattr("EnableDisable")?)?;

    m.add_class::<PyReinitializedState>()?;
    PyReinitializedState::register_constants(&m.getattr("ReinitializedState")?)?;

    m.add_class::<PySegmentation>()?;
    PySegmentation::register_constants(&m.getattr("Segmentation")?)?;

    m.add_class::<PyLifeSafetyOperation>()?;
    PyLifeSafetyOperation::register_constants(&m.getattr("LifeSafetyOperation")?)?;

    m.add_class::<PyEventState>()?;
    PyEventState::register_constants(&m.getattr("EventState")?)?;

    m.add_class::<PyEventType>()?;
    PyEventType::register_constants(&m.getattr("EventType")?)?;

    m.add_class::<PyMessagePriority>()?;
    PyMessagePriority::register_constants(&m.getattr("MessagePriority")?)?;

    // Composite types
    m.add_class::<PyObjectIdentifier>()?;
    m.add_class::<PyPropertyValue>()?;
    m.add_class::<PyDiscoveredDevice>()?;
    m.add_class::<PyCovNotification>()?;
    m.add_class::<PyCovNotificationIterator>()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_address_ipv4() {
        let mac = parse_address("192.168.1.100:47808").unwrap();
        assert_eq!(mac.len(), 6);
        assert_eq!(&mac[..4], &[192, 168, 1, 100]);
        assert_eq!(u16::from_be_bytes([mac[4], mac[5]]), 47808);
    }

    #[test]
    fn parse_address_ipv6() {
        let mac = parse_address("[::1]:47808").unwrap();
        assert_eq!(mac.len(), 18);
        // ::1 → 15 zero bytes + 0x01
        assert_eq!(mac[15], 1);
        assert_eq!(u16::from_be_bytes([mac[16], mac[17]]), 47808);
    }

    #[test]
    fn parse_address_ipv6_full() {
        let mac = parse_address("[fe80::1]:47808").unwrap();
        assert_eq!(mac.len(), 18);
        assert_eq!(mac[0], 0xfe);
        assert_eq!(mac[1], 0x80);
    }

    #[test]
    fn parse_address_hex_mac() {
        let mac = parse_address("01:02:03:04:05:06").unwrap();
        assert_eq!(mac, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn parse_address_rejects_garbage() {
        assert!(parse_address("not_an_address").is_err());
    }

    #[test]
    fn parse_address_ipv6_missing_bracket() {
        assert!(parse_address("[::1").is_err());
    }
}
