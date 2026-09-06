//! Projection of decoded Audit Log query ACKs into Python-owned mappings.

use bacnet_services::audit::{
    AuditLogQueryAck, AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditNotification,
};
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::primitives::Date;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyList};

use super::{
    PyAuditOperation, PyBACnetTimeStamp, PyErrorClass, PyErrorCode, PyObjectIdentifier,
    PyPropertyIdentifier,
};

fn recipient_to_py<'py>(
    py: Python<'py>,
    recipient: &BACnetRecipient,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match recipient {
        BACnetRecipient::Device(object_identifier) => {
            result.set_item("kind", "device")?;
            result.set_item(
                "object_identifier",
                PyObjectIdentifier::from_rust(*object_identifier),
            )?;
        }
        BACnetRecipient::Address(address) => {
            result.set_item("kind", "address")?;
            result.set_item("network_number", address.network_number)?;
            result.set_item("mac_address", PyBytes::new(py, &address.mac_address))?;
        }
    }
    Ok(result)
}

fn property_reference_to_py<'py>(
    py: Python<'py>,
    reference: &AuditPropertyReference,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item(
        "property_identifier",
        PyPropertyIdentifier {
            inner: reference.property_identifier,
        },
    )?;
    result.set_item("property_array_index", reference.property_array_index)?;
    Ok(result)
}

fn notification_to_py<'py>(
    py: Python<'py>,
    notification: &BACnetAuditNotification,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item(
        "source_timestamp",
        notification
            .source_timestamp
            .clone()
            .map(PyBACnetTimeStamp::from_rust),
    )?;
    result.set_item(
        "target_timestamp",
        notification
            .target_timestamp
            .clone()
            .map(PyBACnetTimeStamp::from_rust),
    )?;
    result.set_item(
        "source_device",
        recipient_to_py(py, &notification.source_device)?,
    )?;
    result.set_item(
        "source_object",
        notification
            .source_object
            .map(PyObjectIdentifier::from_rust),
    )?;
    result.set_item(
        "operation",
        PyAuditOperation {
            inner: notification.operation,
        },
    )?;
    result.set_item("source_comment", notification.source_comment.as_deref())?;
    result.set_item("target_comment", notification.target_comment.as_deref())?;
    result.set_item("invoke_id", notification.invoke_id)?;
    result.set_item("source_user_id", notification.source_user_id)?;
    result.set_item("source_user_role", notification.source_user_role)?;
    result.set_item(
        "target_device",
        recipient_to_py(py, &notification.target_device)?,
    )?;
    result.set_item(
        "target_object",
        notification
            .target_object
            .map(PyObjectIdentifier::from_rust),
    )?;
    match &notification.target_property {
        Some(reference) => {
            result.set_item("target_property", property_reference_to_py(py, reference)?)?
        }
        None => result.set_item("target_property", py.None())?,
    }
    result.set_item("target_priority", notification.target_priority)?;
    match &notification.target_value {
        Some(value) => result.set_item("target_value", PyBytes::new(py, value))?,
        None => result.set_item("target_value", py.None())?,
    }
    match &notification.current_value {
        Some(value) => result.set_item("current_value", PyBytes::new(py, value))?,
        None => result.set_item("current_value", py.None())?,
    }
    result.set_item(
        "result",
        notification.result.map(|(error_class, error_code)| {
            (
                PyErrorClass { inner: error_class },
                PyErrorCode { inner: error_code },
            )
        }),
    )?;
    Ok(result)
}

fn actual_year(date: &Date) -> u16 {
    date.actual_year().unwrap_or(u16::from(Date::UNSPECIFIED))
}

fn datum_to_py<'py>(py: Python<'py>, datum: &BACnetAuditLogDatum) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match datum {
        BACnetAuditLogDatum::LogStatus(status) => {
            result.set_item("kind", "log_status")?;
            result.set_item("log_status", status)?;
        }
        BACnetAuditLogDatum::AuditNotification(notification) => {
            result.set_item("kind", "audit_notification")?;
            result.set_item("audit_notification", notification_to_py(py, notification)?)?;
        }
        BACnetAuditLogDatum::TimeChange(change) => {
            result.set_item("kind", "time_change")?;
            result.set_item("time_change", change)?;
        }
    }
    Ok(result)
}

/// Project a completely decoded ACK into canonical Python-owned mappings.
pub(crate) fn audit_log_query_ack_to_py(
    py: Python<'_>,
    ack: &AuditLogQueryAck,
) -> PyResult<Py<PyAny>> {
    let result = PyDict::new(py);
    result.set_item("audit_log", PyObjectIdentifier::from_rust(ack.audit_log))?;
    let records = PyList::empty(py);
    for item in &ack.records {
        let record_result = PyDict::new(py);
        record_result.set_item("sequence_number", item.sequence_number)?;
        let record = PyDict::new(py);
        let (date, time) = &item.record.timestamp;
        record.set_item(
            "timestamp",
            (
                (actual_year(date), date.month, date.day, date.day_of_week),
                (time.hour, time.minute, time.second, time.hundredths),
            ),
        )?;
        record.set_item("datum", datum_to_py(py, &item.record.datum)?)?;
        record_result.set_item("record", record)?;
        records.append(record_result)?;
    }
    result.set_item("records", records)?;
    result.set_item("no_more_items", ack.no_more_items)?;
    Ok(result.into_any().unbind())
}
