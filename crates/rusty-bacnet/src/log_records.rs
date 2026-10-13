//! Log records, paged-read cursors and pages as Python values (#1530,
//! #1534).
//!
//! A record is a dict: `timestamp` as a `(date, time)` pair and `datum` as a
//! dict whose `kind` names the choice and whose same-named key, when there
//! is one, holds its value; a Trend Log record adds `status_flags`. A cursor
//! is `None` or `"oldest"`, or a `(kind, value)` pair: `("sequence", n)`,
//! `("position", n)` or `("time", reference_time)`.

use bacnet_client::log_reader::{LogCursor, LogGap, LogPage};
use bacnet_encoding::constructed::encode_notification_parameters;
use bacnet_services::read_range::{LogRecords, ReadRangeAck, ReadRangeViolation};
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum,
};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyInt, PyList, PySequence, PyString, PyTuple};

use crate::types::{
    audit_log_record_to_py, date_value, time_value, PyBACnetTimeStamp, PyEventState, PyEventType,
    PyObjectIdentifier, PyPropertyIdentifier,
};

/// `(date, time)` as Python reads them.
type DateTimeValue = ((u16, u8, u8, u8), (u8, u8, u8, u8));

fn date_time(date: &Date, time: &Time) -> DateTimeValue {
    (date_value(date), time_value(time))
}

fn datum<'py>(py: Python<'py>, kind: &str) -> PyResult<Bound<'py, PyDict>> {
    let datum = PyDict::new(py);
    datum.set_item("kind", kind)?;
    Ok(datum)
}

fn datum_with<'py>(
    py: Python<'py>,
    kind: &str,
    value: impl IntoPyObject<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let datum = datum(py, kind)?;
    datum.set_item(kind, value)?;
    Ok(datum)
}

/// A Trend Log datum, or a Trend Log Multiple value through its
/// [`LogDatum`] form.
fn log_datum<'py>(py: Python<'py>, value: &LogDatum) -> PyResult<Bound<'py, PyDict>> {
    match value {
        // Bit 0 log-disabled, bit 1 buffer-purged, bit 2 log-interrupted.
        LogDatum::LogStatus(status) => datum_with(py, "log_status", status.bits()),
        LogDatum::BooleanValue(value) => datum_with(py, "boolean", value),
        LogDatum::RealValue(value) => datum_with(py, "real", value),
        LogDatum::EnumValue(value) => datum_with(py, "enumerated", value),
        LogDatum::UnsignedValue(value) => datum_with(py, "unsigned", value),
        LogDatum::SignedValue(value) => datum_with(py, "signed", value),
        LogDatum::BitstringValue { unused_bits, data } => {
            datum_with(py, "bitstring", (*unused_bits, PyBytes::new(py, data)))
        }
        LogDatum::NullValue => datum(py, "null"),
        LogDatum::Failure {
            error_class,
            error_code,
        } => datum_with(py, "failure", (*error_class, *error_code)),
        LogDatum::TimeChange(seconds) => datum_with(py, "time_change", seconds),
        LogDatum::AnyValue(bytes) => datum_with(py, "any", PyBytes::new(py, bytes)),
    }
}

fn notification<'py>(
    py: Python<'py>,
    notification: &EventNotificationRequest,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("process_identifier", notification.process_identifier)?;
    result.set_item(
        "initiating_device_identifier",
        PyObjectIdentifier::from_rust(notification.initiating_device_identifier),
    )?;
    result.set_item(
        "event_object_identifier",
        PyObjectIdentifier::from_rust(notification.event_object_identifier),
    )?;
    result.set_item(
        "timestamp",
        PyBACnetTimeStamp::from_rust(notification.timestamp.clone()),
    )?;
    result.set_item("notification_class", notification.notification_class)?;
    result.set_item("priority", notification.priority)?;
    result.set_item(
        "event_type",
        PyEventType {
            inner: notification.event_type,
        },
    )?;
    result.set_item("message_text", notification.message_text.as_deref())?;
    result.set_item("notify_type", notification.notify_type.to_raw())?;
    result.set_item("ack_required", notification.ack_required)?;
    result.set_item(
        "from_state",
        PyEventState {
            inner: notification.from_state,
        },
    )?;
    result.set_item(
        "to_state",
        PyEventState {
            inner: notification.to_state,
        },
    )?;
    let event_values = match &notification.event_values {
        Some(parameters) => {
            let mut encoded = BytesMut::new();
            encode_notification_parameters(parameters, &mut encoded)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
            Some(PyBytes::new(py, &encoded))
        }
        None => None,
    };
    result.set_item("event_values", event_values)?;
    Ok(result)
}

fn trend_log_record<'py>(
    py: Python<'py>,
    record: &BACnetLogRecord,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("timestamp", date_time(&record.date, &record.time))?;
    result.set_item("datum", log_datum(py, &record.log_datum)?)?;
    // Bit 0 in-alarm, bit 1 fault, bit 2 overridden, bit 3 out-of-service.
    result.set_item(
        "status_flags",
        record.status_flags.map(|flags| flags.bits()),
    )?;
    Ok(result)
}

fn event_log_record<'py>(
    py: Python<'py>,
    record: &BACnetEventLogRecord,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("timestamp", date_time(&record.date, &record.time))?;
    let value = match &record.log_datum {
        EventLogDatum::LogStatus(status) => datum_with(py, "log_status", status.bits())?,
        EventLogDatum::Notification(request) => {
            datum_with(py, "notification", notification(py, request)?)?
        }
        EventLogDatum::TimeChange(seconds) => datum_with(py, "time_change", seconds)?,
    };
    result.set_item("datum", value)?;
    Ok(result)
}

fn trend_log_multiple_record<'py>(
    py: Python<'py>,
    record: &BACnetLogMultipleRecord,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("timestamp", date_time(&record.date, &record.time))?;
    let value = match &record.log_data {
        LogData::LogStatus(status) => datum_with(py, "log_status", status.bits())?,
        LogData::Values(values) => {
            let list = PyList::empty(py);
            for value in values {
                list.append(log_datum(py, &LogDatum::from(value.clone()))?)?;
            }
            datum_with(py, "values", list)?
        }
        LogData::TimeChange(seconds) => datum_with(py, "time_change", seconds)?,
    };
    result.set_item("datum", value)?;
    Ok(result)
}

/// The records as a list of dicts, oldest first.
pub(crate) fn records_to_py<'py>(
    py: Python<'py>,
    records: &LogRecords,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    match records {
        LogRecords::TrendLog(records) => {
            for record in records {
                list.append(trend_log_record(py, record)?)?;
            }
        }
        LogRecords::EventLog(records) => {
            for record in records {
                list.append(event_log_record(py, record)?)?;
            }
        }
        LogRecords::TrendLogMultiple(records) => {
            for record in records {
                list.append(trend_log_multiple_record(py, record)?)?;
            }
        }
        LogRecords::AuditLog(records) => {
            for record in records {
                list.append(audit_log_record_to_py(py, record)?)?;
            }
        }
    }
    Ok(list)
}

/// Decode the records in a `read_range` result of a log's Log_Buffer.
///
/// Raises ValueError when the result isn't for the Log_Buffer of a Trend
/// Log, Event Log, Trend Log Multiple or Audit Log, or when a record doesn't
/// decode, naming that record's index and offset in `item_data`.
#[pyfunction]
pub(crate) fn decode_log_records<'py>(
    py: Python<'py>,
    result: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyList>> {
    let field = |name: &str| {
        result
            .get_item(name)?
            .ok_or_else(|| PyValueError::new_err(format!("read_range result has no '{name}'")))
    };
    let object: PyObjectIdentifier = field("object_id")?.extract()?;
    let property: PyPropertyIdentifier = field("property_id")?.extract()?;
    let ack = ReadRangeAck {
        object_identifier: object.to_rust(),
        property_identifier: property.to_rust(),
        property_array_index: None,
        result_flags: (false, false, false),
        item_count: field("item_count")?.extract()?,
        item_data: field("item_data")?.extract()?,
        first_sequence_number: None,
    };
    match ack.log_records() {
        Some(Ok(records)) => records_to_py(py, &records),
        Some(Err(error)) => Err(PyValueError::new_err(error.to_string())),
        None => Err(PyValueError::new_err(
            "decode_log_records reads the Log_Buffer of a Trend Log, Event Log, \
             Trend Log Multiple or Audit Log",
        )),
    }
}

/// `value` with its outer sequence and the sequences in it as tuples, so a
/// `(date, time)` pair that went through JSON as nested lists reads like
/// the tuples it was. Anything else comes back as it was, for the tuple
/// parser to refuse.
fn as_tuples<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = value.py();
    let Ok(list) = value.cast::<PyList>() else {
        return Ok(value.clone());
    };
    let items = list
        .iter()
        .map(|item| match item.cast::<PyList>() {
            Ok(inner) => Ok(PyTuple::new(py, inner.iter())?.into_any()),
            Err(_) => Ok(item),
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyTuple::new(py, items)?.into_any())
}

/// A `reference_time`: a naive `datetime.datetime`, taken as the device's
/// local time, or a `(date, time)` pair of tuples.
pub(crate) fn reference_time(value: &Bound<'_, PyAny>) -> PyResult<(Date, Time)> {
    let py = value.py();
    let datetime = py.import("datetime")?.getattr("datetime")?;
    if !value.is_instance(&datetime)? {
        return crate::types::date_time_tuple(&as_tuples(value)?, "reference_time");
    }
    if !value.getattr("tzinfo")?.is_none() {
        return Err(PyValueError::new_err(
            "reference_time must be a naive datetime in the device's local time; \
             convert an aware one with .astimezone(device_zone).replace(tzinfo=None)",
        ));
    }
    let part = |name: &str| -> PyResult<u32> { value.getattr(name)?.extract() };
    let narrow = |value: u32| u8::try_from(value).map_err(|e| PyValueError::new_err(e.to_string()));
    let weekday: u8 = value.call_method0("isoweekday")?.extract()?;
    let date = crate::types::date_from_value((
        u16::try_from(part("year")?).map_err(|e| PyValueError::new_err(e.to_string()))?,
        narrow(part("month")?)?,
        narrow(part("day")?)?,
        weekday,
    ))?;
    let time = Time {
        hour: narrow(part("hour")?)?,
        minute: narrow(part("minute")?)?,
        second: narrow(part("second")?)?,
        hundredths: narrow(part("microsecond")? / 10_000)?,
    };
    Ok((date, time))
}

/// A cursor from Python: `None` or `"oldest"`, or a `(kind, value)` pair.
pub(crate) fn cursor_from_py(cursor: Option<&Bound<'_, PyAny>>) -> PyResult<LogCursor> {
    let Some(cursor) = cursor.filter(|cursor| !cursor.is_none()) else {
        return Ok(LogCursor::Oldest);
    };
    if let Ok(kind) = cursor.cast::<PyString>() {
        return match kind.to_str()? {
            "oldest" => Ok(LogCursor::Oldest),
            other => Err(PyValueError::new_err(format!(
                "cursor must be None, 'oldest' or a (kind, value) pair, got '{other}'"
            ))),
        };
    }
    let shape = || {
        PyValueError::new_err(
            "cursor must be None, 'oldest', ('sequence', n), ('position', n) or \
             ('time', reference_time)",
        )
    };
    let pair = cursor.cast::<PySequence>().map_err(|_| shape())?;
    if pair.len()? != 2 {
        return Err(shape());
    }
    let kind: String = pair.get_item(0)?.extract().map_err(|_| shape())?;
    let value = pair.get_item(1)?;
    match kind.as_str() {
        "sequence" => Ok(LogCursor::Sequence(value.extract()?)),
        "position" => Ok(LogCursor::Position(value.extract()?)),
        "time" => {
            let (date, time) = reference_time(&value)?;
            Ok(LogCursor::Time(date, time))
        }
        _ => Err(shape()),
    }
}

fn cursor_to_py(py: Python<'_>, cursor: LogCursor) -> PyResult<Py<PyAny>> {
    Ok(match cursor {
        LogCursor::Oldest => "oldest".into_pyobject(py)?.into_any().unbind(),
        LogCursor::Sequence(sequence) => ("sequence", sequence)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        LogCursor::Position(position) => ("position", position)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        LogCursor::Time(date, time) => ("time", date_time(&date, &time))
            .into_pyobject(py)?
            .into_any()
            .unbind(),
    })
}

fn gap_to_py(py: Python<'_>, gap: Option<LogGap>) -> PyResult<Py<PyAny>> {
    let Some(gap) = gap else {
        return Ok(py.None());
    };
    let result = PyDict::new(py);
    result.set_item("expected", gap.expected)?;
    result.set_item("first", gap.first)?;
    result.set_item("skipped", gap.skipped)?;
    Ok(result.into_any().unbind())
}

/// The rule names a list of violations carries.
pub(crate) fn violation_names(violations: &[ReadRangeViolation]) -> Vec<&'static str> {
    violations.iter().map(|rule| rule.name()).collect()
}

/// A page as the dict `read_log_page` returns.
pub(crate) fn page_to_py(py: Python<'_>, page: LogPage) -> PyResult<Py<PyAny>> {
    let result = PyDict::new(py);
    result.set_item("records", records_to_py(py, &page.records)?)?;
    result.set_item("first_sequence_number", page.first_sequence_number)?;
    result.set_item("result_flags", page.result_flags)?;
    result.set_item("gap", gap_to_py(py, page.gap)?)?;
    result.set_item("violations", violation_names(&page.violations))?;
    result.set_item("next", cursor_to_py(py, page.next)?)?;
    result.set_item("done", page.done)?;
    result.set_item("wrapped", page.wrapped)?;
    Ok(result.into_any().unbind())
}

/// A `read_log_page` call's log and cursor, checked before any I/O:
/// ValueError for an object that keeps no log or a bad cursor.
pub(crate) fn page_request(
    object: &PyObjectIdentifier,
    cursor: Option<&Bound<'_, PyAny>>,
) -> PyResult<(ObjectIdentifier, LogCursor)> {
    let log = object.to_rust();
    if LogRecords::empty_for(log.object_type()).is_none() {
        return Err(PyValueError::new_err(
            "read_log_page reads a Trend Log, Event Log, Trend Log Multiple or Audit Log",
        ));
    }
    Ok((log, cursor_from_py(cursor)?))
}

/// A `page_size` argument: an int in 1..=32767, or ValueError for any other
/// int, however large; TypeError for anything that isn't an int.
pub(crate) fn page_size_arg(value: &Bound<'_, PyAny>) -> PyResult<u16> {
    if !value.is_instance_of::<PyInt>() {
        return Err(PyTypeError::new_err("page_size must be an int"));
    }
    value
        .extract::<i64>()
        .ok()
        .and_then(|size| u16::try_from(size).ok())
        .filter(|size| (1..=32_767).contains(size))
        .ok_or_else(|| PyValueError::new_err(format!("page_size must be 1..=32767, got {value}")))
}
