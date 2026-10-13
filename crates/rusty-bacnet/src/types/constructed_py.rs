//! The Python form of each element a typed constructed read decodes (#1310,
//! #1344, #1345). Where the binding takes the production as a typed value,
//! the form is that value's, so a read can be handed back to the write.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::rpm::ReadAccessResult;
use bacnet_types::constructed::{
    AccessResult, BACnetAccessRule, BACnetActionCommand, BACnetActionList,
    BACnetAuthenticationFactorFormat, BACnetAuthenticationPolicy, BACnetCOVSubscription,
    BACnetCalendarEntry, BACnetDateRange, BACnetDestination, BACnetDeviceObjectPropertyReference,
    BACnetDeviceObjectReference, BACnetNameValue, BACnetPortPermission, BACnetPrescale,
    BACnetPropertyAccessResult, BACnetRecipient, BACnetScale, BACnetSpecialEvent,
    BACnetStageLimitValue, BACnetTimeValue, BACnetValueSource, ReadAccessSpecification,
    SpecialEventPeriod,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};
use bytes::BytesMut;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::IntoPyObjectExt;

use super::audit_projection::recipient_to_py;
use super::date::date_value;
use super::read_value::decode_read_value;
use super::rpm_wpm::read_access_result_to_py;
use super::timestamp::time_value;
use super::{
    PyBACnetTimeStamp, PyErrorClass, PyErrorCode, PyObjectIdentifier, PyPropertyIdentifier,
    PyPropertyValue,
};

/// One element, decoded.
pub(super) enum Decoded {
    NameValue(BACnetNameValue),
    Destination(BACnetDestination),
    PortPermission(BACnetPortPermission),
    ReadAccessSpecification(ReadAccessSpecification),
    ReadAccessResult(ReadAccessResult),
    ActionList(BACnetActionList),
    DeviceObjectReference(BACnetDeviceObjectReference),
    AuthenticationFactorFormat(BACnetAuthenticationFactorFormat),
    AuthenticationPolicy(BACnetAuthenticationPolicy),
    StageLimitValue(BACnetStageLimitValue),
    AccessRule(BACnetAccessRule),
    DeviceObjectPropertyReference(BACnetDeviceObjectPropertyReference),
    PropertyAccessResult(BACnetPropertyAccessResult),
    Recipient(BACnetRecipient),
    DailySchedule(Vec<BACnetTimeValue>),
    SpecialEvent(BACnetSpecialEvent),
    CalendarEntry(BACnetCalendarEntry),
    DateRange(BACnetDateRange),
    TimeStamp(BACnetTimeStamp),
    CovSubscription(BACnetCOVSubscription),
    ValueSource(BACnetValueSource),
    Scale(BACnetScale),
    Prescale(BACnetPrescale),
    /// The Device, then its network number and MAC.
    AddressBinding(super::constructed_read::AddressBinding),
}

impl Decoded {
    pub(super) fn into_python(self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match self {
            Self::NameValue(tag) => {
                let dict = PyDict::new(py);
                dict.set_item("name", tag.name)?;
                dict.set_item("value", tag.value.map(PyPropertyValue::from_rust))?;
                dict.into_any().unbind()
            }

            Self::Destination(destination) => {
                destination_to_py(py, &destination)?.into_any().unbind()
            }
            Self::PortPermission(permission) => any(py, (permission.port_id, permission.enabled))?,
            Self::ReadAccessSpecification(specification) => specification_to_py(py, specification)?,
            Self::ReadAccessResult(result) => {
                read_access_result_to_py(py, result)?.into_any().unbind()
            }
            Self::ActionList(list) => {
                let commands = PyList::empty(py);
                for command in &list.commands {
                    commands.append(action_command_to_py(py, command)?)?;
                }
                commands.into_any().unbind()
            }
            Self::DeviceObjectReference(reference) => object_reference_to_py(py, &reference)?,
            Self::AuthenticationFactorFormat(format) => {
                let format_type = format.format_type.to_raw();
                match (format.vendor_id, format.vendor_format) {
                    (None, None) => any(py, format_type)?,
                    (vendor_id, vendor_format) => any(py, (format_type, vendor_id, vendor_format))?,
                }
            }
            Self::AuthenticationPolicy(policy) => {
                let entries = PyList::empty(py);
                for entry in &policy.policy {
                    entries.append((
                        object_reference_to_py(py, &entry.credential_data_input)?,
                        entry.index,
                    ))?;
                }
                any(py, (entries, policy.order_enforced, policy.timeout))?
            }
            Self::StageLimitValue(stage) => any(
                py,
                (
                    f64::from(stage.limit),
                    stage.values,
                    f64::from(stage.deadband),
                ),
            )?,
            Self::AccessRule(rule) => access_rule_to_py(py, &rule)?.into_any().unbind(),
            Self::DeviceObjectPropertyReference(reference) => {
                property_reference_to_py(py, &reference)?
                    .into_any()
                    .unbind()
            }
            Self::PropertyAccessResult(result) => property_access_result_to_py(py, result)?
                .into_any()
                .unbind(),
            Self::Recipient(recipient) => recipient_to_py(py, &recipient)?.into_any().unbind(),
            Self::DailySchedule(time_values) => time_values_to_py(py, time_values)?,
            Self::SpecialEvent(event) => special_event_to_py(py, event)?.into_any().unbind(),
            Self::CalendarEntry(entry) => calendar_entry_to_py(py, &entry)?.into_any().unbind(),
            Self::DateRange(range) => any(
                py,
                (date_value(&range.start_date), date_value(&range.end_date)),
            )?,
            Self::TimeStamp(timestamp) => {
                Py::new(py, PyBACnetTimeStamp::from_rust(timestamp))?.into_any()
            }
            Self::CovSubscription(subscription) => cov_subscription_to_py(py, &subscription)?
                .into_any()
                .unbind(),
            Self::ValueSource(source) => match source {
                BACnetValueSource::None => py.None(),
                BACnetValueSource::Object(reference) => object_reference_to_py(py, &reference)?,
                // The address form the recipient mappings use, `kind` included.
                BACnetValueSource::Address(address) => {
                    let dict = PyDict::new(py);
                    dict.set_item("kind", "address")?;
                    dict.set_item("network_number", address.network_number)?;
                    dict.set_item("mac_address", PyBytes::new(py, &address.mac_address))?;
                    dict.into_any().unbind()
                }
            },
            Self::Scale(BACnetScale::FloatScale(factor)) => any(py, f64::from(factor))?,
            Self::Scale(BACnetScale::IntegerScale(power)) => any(py, power)?,
            Self::Prescale(prescale) => any(py, (prescale.multiplier, prescale.modulo_divide))?,
            Self::AddressBinding((device, network_number, mac_address)) => {
                let dict = PyDict::new(py);
                dict.set_item("device_identifier", PyObjectIdentifier::from_rust(device))?;
                dict.set_item("network_number", network_number)?;
                dict.set_item("mac_address", PyBytes::new(py, &mac_address))?;
                dict.into_any().unbind()
            }
        })
    }
}

/// `value` as a Python object.
fn any<'py, T: IntoPyObject<'py>>(py: Python<'py>, value: T) -> PyResult<Py<PyAny>> {
    value.into_py_any(py)
}

/// `value`, read from `property` of `object`, shaped as a read of that
/// property would be, so a typed value stays typed.
fn read_shaped(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> PyPropertyValue {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value)
        .and_then(|()| decode_read_value(object_type, property, array_index, &encoded))
        .unwrap_or_else(|_| PyPropertyValue::from_rust(value.clone()))
}

fn property_identifier(raw: u32) -> PyPropertyIdentifier {
    PyPropertyIdentifier {
        inner: PropertyIdentifier::from_raw(raw),
    }
}

/// One destination, in the `Destination` mapping `add_notification_forwarder`
/// takes, with every key present.
fn destination_to_py<'py>(
    py: Python<'py>,
    destination: &BACnetDestination,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("recipient", recipient_to_py(py, &destination.recipient)?)?;
    dict.set_item("process_identifier", destination.process_identifier)?;
    dict.set_item("valid_days", destination.valid_days.bits())?;
    dict.set_item("from_time", time_value(&destination.from_time))?;
    dict.set_item("to_time", time_value(&destination.to_time))?;
    dict.set_item(
        "issue_confirmed_notifications",
        destination.issue_confirmed_notifications,
    )?;
    dict.set_item("transitions", destination.transitions.bits())?;
    Ok(dict)
}

/// One member, as `(object, [(property, array_index), ...])`.
fn specification_to_py(
    py: Python<'_>,
    specification: ReadAccessSpecification,
) -> PyResult<Py<PyAny>> {
    let references: Vec<_> = specification
        .list_of_property_references
        .into_iter()
        .map(|reference| {
            (
                PyPropertyIdentifier {
                    inner: reference.property_identifier,
                },
                reference.property_array_index,
            )
        })
        .collect();
    any(
        py,
        (
            PyObjectIdentifier::from_rust(specification.object_identifier),
            references,
        ),
    )
}

/// One command, in the `ActionCommand` mapping `add_command` takes, with
/// every key present. Its value reads as a read of the property it writes
/// would, so a typed value stays typed.
fn action_command_to_py<'py>(
    py: Python<'py>,
    command: &BACnetActionCommand,
) -> PyResult<Bound<'py, PyDict>> {
    let value = read_shaped(
        command.object_identifier.object_type(),
        command.property_identifier,
        command.property_array_index,
        &command.property_value,
    );
    let dict = PyDict::new(py);
    dict.set_item(
        "device_identifier",
        command.device_identifier.map(PyObjectIdentifier::from_rust),
    )?;
    dict.set_item(
        "object_identifier",
        PyObjectIdentifier::from_rust(command.object_identifier),
    )?;
    dict.set_item(
        "property_identifier",
        PyPropertyIdentifier {
            inner: command.property_identifier,
        },
    )?;
    dict.set_item("property_array_index", command.property_array_index)?;
    dict.set_item("property_value", value)?;
    dict.set_item("priority", command.priority)?;
    dict.set_item("post_delay", command.post_delay)?;
    dict.set_item("quit_on_failure", command.quit_on_failure)?;
    dict.set_item("write_successful", command.write_successful)?;
    Ok(dict)
}

/// An object reference: an `ObjectIdentifier`, or a `(device, object)` pair
/// when it names a device, the forms `door_members` takes.
fn object_reference_to_py(
    py: Python<'_>,
    reference: &BACnetDeviceObjectReference,
) -> PyResult<Py<PyAny>> {
    let object = PyObjectIdentifier::from_rust(reference.object_identifier);
    match reference.device_identifier {
        None => any(py, object),
        Some(device) => any(py, (PyObjectIdentifier::from_rust(device), object)),
    }
}

/// A property reference, in the `DeviceObjectPropertyReference` mapping, with
/// every key present.
fn property_reference_to_py<'py>(
    py: Python<'py>,
    reference: &BACnetDeviceObjectPropertyReference,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item(
        "object_identifier",
        PyObjectIdentifier::from_rust(reference.object_identifier),
    )?;
    dict.set_item(
        "property_identifier",
        property_identifier(reference.property_identifier),
    )?;
    dict.set_item("property_array_index", reference.property_array_index)?;
    dict.set_item(
        "device_identifier",
        reference
            .device_identifier
            .map(PyObjectIdentifier::from_rust),
    )?;
    Ok(dict)
}

/// One rule, in the `AccessRule` mapping `add_access_rights` takes, with every
/// key present: `None` stands for ALWAYS and ALL, as it does in the write.
fn access_rule_to_py<'py>(
    py: Python<'py>,
    rule: &BACnetAccessRule,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("enable", rule.enable)?;
    let time_range = rule
        .time_range
        .as_ref()
        .map(|reference| property_reference_to_py(py, reference))
        .transpose()?;
    dict.set_item("time_range", time_range)?;
    let location = rule
        .location
        .as_ref()
        .map(|reference| object_reference_to_py(py, reference))
        .transpose()?;
    dict.set_item("location", location)?;
    Ok(dict)
}

/// A Global Group member's result: its reference's keys, then `value` (shaped
/// as a read of the member) or `error`, an `(ErrorClass, ErrorCode)` pair.
fn property_access_result_to_py(
    py: Python<'_>,
    result: BACnetPropertyAccessResult,
) -> PyResult<Bound<'_, PyDict>> {
    let reference = &result.reference;
    let dict = property_reference_to_py(py, reference)?;
    match &result.access_result {
        AccessResult::Value(value) => {
            let value = read_shaped(
                reference.object_identifier.object_type(),
                PropertyIdentifier::from_raw(reference.property_identifier),
                reference.property_array_index,
                value,
            );
            dict.set_item("value", value)?;
            dict.set_item("error", py.None())?;
        }
        AccessResult::Error { class, code } => {
            dict.set_item("value", py.None())?;
            dict.set_item(
                "error",
                (PyErrorClass { inner: *class }, PyErrorCode { inner: *code }),
            )?;
        }
    }
    Ok(dict)
}

/// A day's or an event's time-values, as `[(time, value), ...]`.
fn time_values_to_py(py: Python<'_>, time_values: Vec<BACnetTimeValue>) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for time_value_entry in time_values {
        list.append((
            time_value(&time_value_entry.time),
            PyPropertyValue::from_rust(time_value_entry.value),
        ))?;
    }
    Ok(list.into_any().unbind())
}

/// One Exception_Schedule event: `period` (a calendar entry mapping, or the
/// Calendar's `ObjectIdentifier`), `time_values` and `priority`.
fn special_event_to_py(py: Python<'_>, event: BACnetSpecialEvent) -> PyResult<Bound<'_, PyDict>> {
    let dict = PyDict::new(py);
    match &event.period {
        SpecialEventPeriod::CalendarEntry(entry) => {
            dict.set_item("period", calendar_entry_to_py(py, entry)?)?
        }
        SpecialEventPeriod::CalendarReference(calendar) => {
            dict.set_item("period", PyObjectIdentifier::from_rust(*calendar))?
        }
    }
    dict.set_item(
        "time_values",
        time_values_to_py(py, event.list_of_time_values)?,
    )?;
    dict.set_item("priority", event.event_priority)?;
    Ok(dict)
}

/// One calendar entry, a mapping whose `kind` names the alternative: `date`
/// with `date`, `date_range` with `start_date` and `end_date`, or
/// `week_n_day` with `month`, `week_of_month` and `day_of_week`.
fn calendar_entry_to_py<'py>(
    py: Python<'py>,
    entry: &BACnetCalendarEntry,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match entry {
        BACnetCalendarEntry::Date(date) => {
            dict.set_item("kind", "date")?;
            dict.set_item("date", date_value(date))?;
        }
        BACnetCalendarEntry::DateRange(range) => {
            dict.set_item("kind", "date_range")?;
            dict.set_item("start_date", date_value(&range.start_date))?;
            dict.set_item("end_date", date_value(&range.end_date))?;
        }
        BACnetCalendarEntry::WeekNDay(week_n_day) => {
            dict.set_item("kind", "week_n_day")?;
            dict.set_item("month", week_n_day.month)?;
            dict.set_item("week_of_month", week_n_day.week_of_month)?;
            dict.set_item("day_of_week", week_n_day.day_of_week)?;
        }
    }
    Ok(dict)
}

/// One COV subscription: the recipient and process, the monitored property's
/// keys, and the subscription's flags.
fn cov_subscription_to_py<'py>(
    py: Python<'py>,
    subscription: &BACnetCOVSubscription,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item(
        "recipient",
        recipient_to_py(py, &subscription.recipient.recipient)?,
    )?;
    dict.set_item(
        "process_identifier",
        subscription.recipient.process_identifier,
    )?;
    let monitored = &subscription.monitored_property_reference;
    dict.set_item(
        "object_identifier",
        PyObjectIdentifier::from_rust(monitored.object_identifier),
    )?;
    dict.set_item(
        "property_identifier",
        property_identifier(monitored.property_identifier),
    )?;
    dict.set_item("property_array_index", monitored.property_array_index)?;
    dict.set_item(
        "issue_confirmed_notifications",
        subscription.issue_confirmed_notifications,
    )?;
    dict.set_item("time_remaining", subscription.time_remaining)?;
    dict.set_item("cov_increment", subscription.cov_increment.map(f64::from))?;
    Ok(dict)
}
