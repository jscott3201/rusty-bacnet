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
use bacnet_types::enums as bacnet_enums;
use bacnet_types::primitives;

mod address;
mod cov;
mod device;
mod enums;
mod object_identifier;
mod property_value;
mod rpm_wpm;

pub use address::parse_address;
pub use cov::{PyCovNotification, PyCovNotificationIterator};
pub use device::PyDiscoveredDevice;
pub use enums::*;
pub use object_identifier::PyObjectIdentifier;
pub use property_value::PyPropertyValue;
pub(crate) use rpm_wpm::{py_to_rpm_specs, py_to_wpm_specs, rpm_ack_to_py};

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
mod tests;
