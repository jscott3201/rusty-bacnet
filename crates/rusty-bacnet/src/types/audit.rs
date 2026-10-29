//! Python mapping boundary for typed Audit client services.

use bacnet_services::audit::{
    AuditLogQueryRequest, AuditNotificationRequest, AuditPropertyReference,
    BACnetAuditLogQueryParameters, BACnetAuditNotification,
};
use bacnet_services::common::MAX_DECODED_ITEMS;
use bacnet_types::MacAddr;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{AuditOperation, BACnetSuccessFilter};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyList, PyMapping, PyTuple};

use super::mapping::{
    bytes, discriminator, fixed_integer, mapping, object_identifier, optional_item, ranged_integer,
    required_item, string, validate_keys,
};
use super::{PyAuditOperation, PyBACnetTimeStamp, PyPropertyIdentifier, int_enums};

const NOTIFICATION_REQUIRED: &[&str] = &["source_device", "operation", "target_device"];
const NOTIFICATION_OPTIONAL: &[&str] = &[
    "source_timestamp",
    "target_timestamp",
    "source_object",
    "source_comment",
    "target_comment",
    "invoke_id",
    "source_user_id",
    "source_user_role",
    "target_object",
    "target_property",
    "target_priority",
    "target_value",
    "current_value",
    "result",
];

pub(crate) fn recipient(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetRecipient> {
    let value = mapping(value, name)?;
    match discriminator(value, name)?.as_str() {
        "device" => {
            validate_keys(value, name, &["kind", "object_identifier"], &[])?;
            Ok(BACnetRecipient::Device(object_identifier(
                &required_item(value, name, "object_identifier")?,
                &format!("{name}.object_identifier"),
            )?))
        }
        "address" => Ok(BACnetRecipient::Address(address_mapping(value, name)?)),
        kind => Err(PyValueError::new_err(format!(
            "{name}.kind must be 'device' or 'address', got '{kind}'"
        ))),
    }
}

fn address_mapping(value: &Bound<'_, PyMapping>, name: &str) -> PyResult<BACnetAddress> {
    validate_keys(value, name, &["kind", "network_number", "mac_address"], &[])?;
    let kind = discriminator(value, name)?;
    if kind != "address" {
        return Err(PyValueError::new_err(format!(
            "{name}.kind must be 'address', got '{kind}'"
        )));
    }
    let network_number = fixed_integer::<u16>(
        &required_item(value, name, "network_number")?,
        &format!("{name}.network_number"),
    )?;
    let mac_address = bytes(
        &required_item(value, name, "mac_address")?,
        &format!("{name}.mac_address"),
    )?;
    Ok(BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(&mac_address),
    })
}

fn optional_address(
    value: Option<Bound<'_, PyAny>>,
    name: &str,
) -> PyResult<Option<BACnetAddress>> {
    value
        .map(|value| {
            let value = mapping(&value, name)?;
            address_mapping(value, name)
        })
        .transpose()
}

fn property_reference(value: &Bound<'_, PyAny>, name: &str) -> PyResult<AuditPropertyReference> {
    let value = mapping(value, name)?;
    validate_keys(
        value,
        name,
        &["property_identifier"],
        &["property_array_index"],
    )?;
    let property_identifier = required_item(value, name, "property_identifier")?
        .extract::<PyPropertyIdentifier>()
        .map(|value| value.to_rust())
        .map_err(|_| {
            PyTypeError::new_err(format!(
                "{name}.property_identifier must be a PropertyIdentifier"
            ))
        })?;
    let property_array_index = optional_item(value, "property_array_index")?
        .map(|value| fixed_integer(&value, &format!("{name}.property_array_index")))
        .transpose()?;
    Ok(AuditPropertyReference {
        property_identifier,
        property_array_index,
    })
}

fn operation(value: &Bound<'_, PyAny>, name: &str) -> PyResult<AuditOperation> {
    let operation = value
        .extract::<PyAuditOperation>()
        .map(|value| value.to_rust())
        .map_err(|_| PyTypeError::new_err(format!("{name} must be an AuditOperation")))?;
    let raw = operation.to_raw();
    if raw <= 15 || (32..=63).contains(&raw) {
        Ok(operation)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must select a standard operation 0..=15 or proprietary operation 32..=63, got {raw}"
        )))
    }
}

fn result(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<(
    bacnet_types::enums::ErrorClass,
    bacnet_types::enums::ErrorCode,
)> {
    let value = value.cast::<PyTuple>().map_err(|_| {
        PyTypeError::new_err(format!("{name} must be an (ErrorClass, ErrorCode) tuple"))
    })?;
    if value.len() != 2 {
        return Err(PyTypeError::new_err(format!(
            "{name} must be an (ErrorClass, ErrorCode) tuple"
        )));
    }
    // An int out of range keeps from_py's ValueError; any other refusal is
    // a TypeError naming the item.
    let py = value.py();
    let item = |index: usize, what: &str| {
        let what = what.to_owned();
        move |error: PyErr| {
            if error.is_instance_of::<PyValueError>(py) {
                error
            } else {
                PyTypeError::new_err(format!("{name}[{index}] must be an {what} or int"))
            }
        }
    };
    let error_class = value
        .get_item(0)
        .and_then(|value| int_enums::from_py(&value))
        .map_err(item(0, "ErrorClass"))?;
    let error_code = value
        .get_item(1)
        .and_then(|value| int_enums::from_py(&value))
        .map_err(item(1, "ErrorCode"))?;
    Ok((error_class, error_code))
}

fn notification(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetAuditNotification> {
    let value = mapping(value, name)?;
    validate_keys(value, name, NOTIFICATION_REQUIRED, NOTIFICATION_OPTIONAL)?;

    let timestamp = |key: &str| -> PyResult<Option<bacnet_types::primitives::BACnetTimeStamp>> {
        optional_item(value, key)?
            .map(|item| {
                item.extract::<PyBACnetTimeStamp>()
                    .map(|value| value.to_rust().clone())
                    .map_err(|_| {
                        PyTypeError::new_err(format!("{name}.{key} must be a BACnetTimeStamp"))
                    })
            })
            .transpose()
    };
    let optional_oid = |key: &str| -> PyResult<Option<ObjectIdentifier>> {
        optional_item(value, key)?
            .map(|item| object_identifier(&item, &format!("{name}.{key}")))
            .transpose()
    };
    let optional_string = |key: &str| -> PyResult<Option<String>> {
        optional_item(value, key)?
            .map(|item| string(&item, &format!("{name}.{key}")))
            .transpose()
    };
    let optional_u8 = |key: &str| -> PyResult<Option<u8>> {
        optional_item(value, key)?
            .map(|item| fixed_integer(&item, &format!("{name}.{key}")))
            .transpose()
    };

    let target_priority = optional_item(value, "target_priority")?
        .map(|item| ranged_integer::<u8>(&item, &format!("{name}.target_priority"), 1..=16))
        .transpose()?;

    Ok(BACnetAuditNotification {
        source_timestamp: timestamp("source_timestamp")?,
        target_timestamp: timestamp("target_timestamp")?,
        source_device: recipient(
            &required_item(value, name, "source_device")?,
            &format!("{name}.source_device"),
        )?,
        source_object: optional_oid("source_object")?,
        operation: operation(
            &required_item(value, name, "operation")?,
            &format!("{name}.operation"),
        )?,
        source_comment: optional_string("source_comment")?,
        target_comment: optional_string("target_comment")?,
        invoke_id: optional_u8("invoke_id")?,
        source_user_id: optional_item(value, "source_user_id")?
            .map(|item| fixed_integer::<u16>(&item, &format!("{name}.source_user_id")))
            .transpose()?,
        source_user_role: optional_u8("source_user_role")?,
        target_device: recipient(
            &required_item(value, name, "target_device")?,
            &format!("{name}.target_device"),
        )?,
        target_object: optional_oid("target_object")?,
        target_property: optional_item(value, "target_property")?
            .map(|item| property_reference(&item, &format!("{name}.target_property")))
            .transpose()?,
        target_priority,
        target_value: optional_item(value, "target_value")?
            .map(|item| bytes(&item, &format!("{name}.target_value")))
            .transpose()?,
        current_value: optional_item(value, "current_value")?
            .map(|item| bytes(&item, &format!("{name}.current_value")))
            .transpose()?,
        result: optional_item(value, "result")?
            .map(|item| result(&item, &format!("{name}.result")))
            .transpose()?,
    })
}

/// Convert the public Audit notification mapping without retaining Python objects.
pub(crate) fn audit_notification_request_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<AuditNotificationRequest> {
    let value = mapping(value, "request")?;
    validate_keys(value, "request", &["notifications"], &[])?;
    let notifications = required_item(value, "request", "notifications")?;
    let notifications = notifications
        .cast::<PyList>()
        .map_err(|_| PyTypeError::new_err("request.notifications must be a list"))?;
    if notifications.is_empty() || notifications.len() > MAX_DECODED_ITEMS {
        return Err(PyValueError::new_err(format!(
            "request.notifications count must be 1..={MAX_DECODED_ITEMS}, got {}",
            notifications.len()
        )));
    }
    let notifications = notifications
        .iter()
        .enumerate()
        .map(|(index, value)| notification(&value, &format!("request.notifications[{index}]")))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(AuditNotificationRequest { notifications })
}

fn operation_flags(value: &Bound<'_, PyAny>, name: &str) -> PyResult<AuditOperationFlags> {
    let bits = fixed_integer::<u64>(value, name)?;
    AuditOperationFlags::from_bits(bits)
        .map_err(|error| PyValueError::new_err(format!("{name}: {error}")))
}

/// Parse the corrected-baseline three-state success filter (RB-02, RB-20).
///
/// The mapping accepts the raw `BACnetSuccessFilter` values 0 (all), 1
/// (successes-only), and 2 (failures-only) as an integer. The pre-RB-02
/// Boolean `successful-actions-only` meaning is not accepted here: `True`
/// used to mean successes-only and `False` meant all (see
/// `BACnetSuccessFilter::from_legacy_bool`). Pass 1 for the old `True` and 0
/// for the old `False`. In particular a `bool` is never accepted as an
/// arbitrary enum integer: it raises `TypeError`, while an out-of-range
/// integer raises `ValueError`.
fn success_filter(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetSuccessFilter> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} must be 0 (all), 1 (successes-only), or 2 (failures-only) as an integer; the deprecated Boolean successful-actions-only field is not accepted (use 1 for True, 0 for False)"
        )));
    }
    let raw = ranged_integer::<u8>(value, name, 0..=2).map_err(|error| {
        if error.is_instance_of::<PyValueError>(value.py()) {
            PyValueError::new_err(format!(
                "{name} must be 0 (all), 1 (successes-only), or 2 (failures-only), got {}",
                value
                    .extract::<i128>()
                    .map(|raw| raw.to_string())
                    .unwrap_or_else(|_| "<non-integer>".into())
            ))
        } else {
            error
        }
    })?;
    Ok(BACnetSuccessFilter::from_raw(raw as u32))
}

fn query_parameters(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetAuditLogQueryParameters> {
    let value = mapping(value, name)?;
    match discriminator(value, name)?.as_str() {
        "by_target" => {
            validate_keys(
                value,
                name,
                &[
                    "kind",
                    "target_device_identifier",
                    "successful_actions_only",
                ],
                &[
                    "target_device_address",
                    "target_object_identifier",
                    "target_property_identifier",
                    "target_array_index",
                    "target_priority",
                    "operations",
                ],
            )?;
            let target_property_identifier = optional_item(value, "target_property_identifier")?
                .map(|item| {
                    item.extract::<PyPropertyIdentifier>()
                        .map(|value| value.to_rust())
                        .map_err(|_| {
                            PyTypeError::new_err(format!(
                                "{name}.target_property_identifier must be a PropertyIdentifier"
                            ))
                        })
                })
                .transpose()?;
            Ok(BACnetAuditLogQueryParameters::ByTarget {
                target_device_identifier: object_identifier(
                    &required_item(value, name, "target_device_identifier")?,
                    &format!("{name}.target_device_identifier"),
                )?,
                target_device_address: optional_address(
                    optional_item(value, "target_device_address")?,
                    &format!("{name}.target_device_address"),
                )?,
                target_object_identifier: optional_item(value, "target_object_identifier")?
                    .map(|item| {
                        object_identifier(&item, &format!("{name}.target_object_identifier"))
                    })
                    .transpose()?,
                target_property_identifier,
                target_array_index: optional_item(value, "target_array_index")?
                    .map(|item| fixed_integer(&item, &format!("{name}.target_array_index")))
                    .transpose()?,
                target_priority: optional_item(value, "target_priority")?
                    .map(|item| {
                        ranged_integer::<u8>(&item, &format!("{name}.target_priority"), 1..=16)
                    })
                    .transpose()?,
                operations: optional_item(value, "operations")?
                    .map(|item| operation_flags(&item, &format!("{name}.operations")))
                    .transpose()?,
                successful_actions_only: success_filter(
                    &required_item(value, name, "successful_actions_only")?,
                    &format!("{name}.successful_actions_only"),
                )?,
            })
        }
        "by_source" => {
            validate_keys(
                value,
                name,
                &[
                    "kind",
                    "source_device_identifier",
                    "successful_actions_only",
                ],
                &[
                    "source_device_address",
                    "source_object_identifier",
                    "operations",
                ],
            )?;
            Ok(BACnetAuditLogQueryParameters::BySource {
                source_device_identifier: object_identifier(
                    &required_item(value, name, "source_device_identifier")?,
                    &format!("{name}.source_device_identifier"),
                )?,
                source_device_address: optional_address(
                    optional_item(value, "source_device_address")?,
                    &format!("{name}.source_device_address"),
                )?,
                source_object_identifier: optional_item(value, "source_object_identifier")?
                    .map(|item| {
                        object_identifier(&item, &format!("{name}.source_object_identifier"))
                    })
                    .transpose()?,
                operations: optional_item(value, "operations")?
                    .map(|item| operation_flags(&item, &format!("{name}.operations")))
                    .transpose()?,
                successful_actions_only: success_filter(
                    &required_item(value, name, "successful_actions_only")?,
                    &format!("{name}.successful_actions_only"),
                )?,
            })
        }
        kind => Err(PyValueError::new_err(format!(
            "{name}.kind must be 'by_target' or 'by_source', got '{kind}'"
        ))),
    }
}

/// Convert the public Audit Log query mapping without retaining Python objects.
pub(crate) fn audit_log_query_request_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<AuditLogQueryRequest> {
    let value = mapping(value, "request")?;
    validate_keys(
        value,
        "request",
        &["audit_log", "query_parameters", "requested_count"],
        &["start_at_sequence_number"],
    )?;
    Ok(AuditLogQueryRequest {
        audit_log: object_identifier(
            &required_item(value, "request", "audit_log")?,
            "request.audit_log",
        )?,
        query_parameters: query_parameters(
            &required_item(value, "request", "query_parameters")?,
            "request.query_parameters",
        )?,
        start_at_sequence_number: optional_item(value, "start_at_sequence_number")?
            .map(|item| fixed_integer(&item, "request.start_at_sequence_number"))
            .transpose()?,
        requested_count: fixed_integer::<u16>(
            &required_item(value, "request", "requested_count")?,
            "request.requested_count",
        )?,
    })
}

#[cfg(test)]
#[path = "audit/tests.rs"]
mod tests;
