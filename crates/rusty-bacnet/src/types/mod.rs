//! Python-facing type wrappers for BACnet enums, ObjectIdentifier, and PropertyValue.

#![allow(non_snake_case)]

use std::net::Ipv4Addr;
use std::time::Instant;

use bytes::BytesMut;
use pyo3::Py;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use tokio::sync::broadcast;

use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums as bacnet_enums;
use bacnet_types::primitives;

mod access_credential;
mod access_rule;
mod action_list;
mod address;
mod audit;
mod audit_projection;
mod authentication_policy;
mod bit_string;
mod constructed_py;
mod constructed_read;
mod cov;
mod date;
mod destination;
mod device;
mod enums;
mod event_parameter_py;
pub(crate) mod int_enums;
mod mapping;
mod notification_parameters_py;
mod object_identifier;
mod object_profile;
mod property_reference;
mod property_value;
mod read_value;
mod rpm_wpm;
mod schedule;
mod timestamp;

pub(crate) use access_credential::{
    AccessRightsValue, assigned_access_rights_from_py, authorization_exemptions_from_py,
};
pub(crate) use access_rule::{access_rules_from_py, device_object_reference};
pub(crate) use action_list::action_lists_from_py;
pub use address::parse_address;
pub(crate) use address::{PeerAddresses, parse_bip6_address};
pub(crate) use audit::recipient as audit_recipient_from_py;
pub(crate) use audit::{audit_log_query_request_from_py, audit_notification_request_from_py};
pub(crate) use audit_projection::{audit_log_query_ack_to_py, audit_log_record_to_py};
pub(crate) use authentication_policy::authentication_policies_from_py;
pub use bit_string::PyBitString;
pub use cov::{PyCovNotification, PyCovNotificationIterator};
pub(crate) use date::{date_from_value, date_value, specific_date_time};
pub(crate) use destination::destinations;
pub use device::PyDiscoveredDevice;
pub use enums::*;
pub(crate) use int_enums::IntEnumValue;
pub(crate) use mapping::scale as scale_from_py;
pub(crate) use notification_parameters_py::notification_parameters_to_py;
pub use object_identifier::PyObjectIdentifier;
pub(crate) use object_profile::object_profile_from_py;
pub(crate) use property_reference::{
    check_device, local_device, localize, property_reference_from_py, property_references_from_py,
};
pub use property_value::PyPropertyValue;
pub(crate) use read_value::{decode_read_ack, decode_read_value};
pub(crate) use rpm_wpm::{
    PyDeviceWrite, PyPropertyWrite, PyReadAccessSpec, PyWriteAccessSpec, py_to_rpm_specs,
    py_to_wpm_specs, rpm_ack_to_py,
};
pub(crate) use schedule::{
    calendar_entries_from_py, date_range_from_py, special_events_from_py, weekly_schedule_from_py,
};
pub use timestamp::PyBACnetTimeStamp;
pub(crate) use timestamp::{date_time_tuple, time_value};

/// What `__reduce__` raises for a class with no constructor that doesn't
/// copy or pickle, because it holds live or receive-time state (#1500).
/// Without it, pickle protocols 0 and 1 would dump such an object, naming
/// its `rusty_bacnet` class, and fail only when the pickle is loaded.
pub(crate) fn not_picklable(object: &Bound<'_, PyAny>) -> PyErr {
    let name = object
        .get_type()
        .qualname()
        .map_or_else(|_| "object".to_owned(), |name| name.to_string());
    pyo3::exceptions::PyTypeError::new_err(format!("cannot pickle 'rusty_bacnet.{name}' object"))
}

// Module registration
// ---------------------------------------------------------------------------

/// Register all type classes with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Enum types — add class then populate constants from ALL_NAMED.
    m.add_class::<PyObjectType>()?;
    PyObjectType::register_constants(&m.getattr("ObjectType")?)?;

    m.add_class::<PyPropertyIdentifier>()?;
    PyPropertyIdentifier::register_constants(&m.getattr("PropertyIdentifier")?)?;

    // EngineeringUnits, ErrorClass, ErrorCode, RejectReason and AbortReason
    // are IntEnum classes (#1816).
    int_enums::register(m)?;

    m.add_class::<PyAuditOperation>()?;
    PyAuditOperation::register_constants(&m.getattr("AuditOperation")?)?;

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

    m.add_class::<PyAcknowledgmentFilter>()?;
    PyAcknowledgmentFilter::register_constants(&m.getattr("AcknowledgmentFilter")?)?;

    m.add_class::<PyEnrollmentSummaryEventStateFilter>()?;
    PyEnrollmentSummaryEventStateFilter::register_constants(
        &m.getattr("EnrollmentSummaryEventStateFilter")?,
    )?;

    m.add_class::<PyEventType>()?;
    PyEventType::register_constants(&m.getattr("EventType")?)?;

    m.add_class::<PyNotifyType>()?;
    PyNotifyType::register_constants(&m.getattr("NotifyType")?)?;

    m.add_class::<PyMessagePriority>()?;
    PyMessagePriority::register_constants(&m.getattr("MessagePriority")?)?;

    m.add_class::<PyVTClass>()?;
    PyVTClass::register_constants(&m.getattr("VTClass")?)?;

    // Composite types
    m.add_class::<PyObjectIdentifier>()?;
    m.add_class::<PyPropertyValue>()?;
    m.add_class::<PyBitString>()?;
    m.add_function(wrap_pyfunction!(bit_string::enum_name, m)?)?;
    m.add_class::<PyBACnetTimeStamp>()?;
    m.add_class::<PyDiscoveredDevice>()?;
    m.add_class::<PyCovNotification>()?;
    m.add_class::<PyCovNotificationIterator>()?;

    // 255, what an unspecified date or time field holds (#1501).
    m.add("UNSPECIFIED", date::UNSPECIFIED)?;

    Ok(())
}

#[cfg(test)]
mod tests;
