//! Trend Log Multiple registration with the configuration the server's
//! poller acts on (#1235).
use super::super::*;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::LoggingType;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use pyo3::exceptions::PyValueError;

use crate::types::date_time_tuple;

#[pymethods]
impl BACnetServer {
    /// Add a Trend Log Multiple object to the server (before starting).
    ///
    /// The keyword arguments configure what the server's poller does with
    /// it. `members` fills Log_DeviceObjectProperty in order, at most 64; a
    /// `device_identifier` that isn't a Device raises ValueError.
    /// `logging_type` is `"polled"` or `"triggered"`: POLLED with no
    /// `log_interval` takes a one-minute interval, and TRIGGERED zeroes it,
    /// so a `log_interval` with it raises WRITE_ACCESS_DENIED; `"cov"`
    /// raises VALUE_OUT_OF_RANGE, as a client's write of it does, and any
    /// other string ValueError. `log_interval` is in hundredths of a second.
    /// `start_time` and `stop_time` bound when records are kept, each a
    /// `(date, time)` pair of `(full_year, month, day, day_of_week)` and
    /// `(hour, minute, second, hundredths)` tuples, every field 255 to leave
    /// that side open; 255 seconds or hundredths count as zero, and anything
    /// else that isn't an actual date and time raises VALUE_OUT_OF_RANGE. `align_intervals` and `interval_offset` (in
    /// hundredths) align a POLLED log's acquisitions to the clock. Peers can
    /// write each of these, and Trigger, which asks a TRIGGERED log for one
    /// record; `write_property_local` writes it from the application.
    /// `total_record_count` seeds Total_Record_Count: the first record is
    /// numbered one past it, going from 2^32 - 1 on to 1 (#1537).
    #[pyo3(signature = (
        instance,
        name,
        buffer_size=100,
        *,
        members=None,
        log_interval=None,
        logging_type=None,
        start_time=None,
        stop_time=None,
        align_intervals=None,
        interval_offset=None,
        total_record_count=0
    ))]
    fn add_trend_log_multiple(
        &self,
        instance: u32,
        name: &str,
        buffer_size: u32,
        members: Option<&Bound<'_, PyAny>>,
        log_interval: Option<u32>,
        logging_type: Option<&str>,
        start_time: Option<&Bound<'_, PyAny>>,
        stop_time: Option<&Bound<'_, PyAny>>,
        align_intervals: Option<bool>,
        interval_offset: Option<u32>,
        total_record_count: u32,
    ) -> PyResult<()> {
        let settings = TrendLogMultipleSettings {
            members: members
                .map(|members| self.members_from_py(members, "members"))
                .transpose()?
                .unwrap_or_default(),
            log_interval,
            logging_type: logging_type.map(logging_type_from_py).transpose()?,
            start_time: start_time
                .map(|value| date_time_tuple(value, "start_time"))
                .transpose()?,
            stop_time: stop_time
                .map(|value| date_time_tuple(value, "stop_time"))
                .transpose()?,
            align_intervals,
            interval_offset,
            total_record_count,
        };
        let obj = trend_log_multiple(instance, name, buffer_size, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}

fn logging_type_from_py(value: &str) -> PyResult<LoggingType> {
    match value {
        "polled" => Ok(LoggingType::POLLED),
        "triggered" => Ok(LoggingType::TRIGGERED),
        // Named so the object refuses it as it refuses a client's write.
        "cov" => Ok(LoggingType::COV),
        other => Err(PyValueError::new_err(format!(
            "logging_type must be 'polled' or 'triggered', got {other:?}"
        ))),
    }
}

/// The optional `add_trend_log_multiple` keyword arguments; `None` keeps the
/// object's default.
#[derive(Default)]
struct TrendLogMultipleSettings {
    members: Vec<BACnetDeviceObjectPropertyReference>,
    log_interval: Option<u32>,
    logging_type: Option<LoggingType>,
    start_time: Option<(Date, Time)>,
    stop_time: Option<(Date, Time)>,
    align_intervals: Option<bool>,
    interval_offset: Option<u32>,
    /// Total_Record_Count to seed the empty log at.
    total_record_count: u32,
}

/// Build the log through the Rust setters, so Python gets their checks.
/// Logging_Type goes before Log_Interval, which it may set or lock.
fn trend_log_multiple(
    instance: u32,
    name: &str,
    buffer_size: u32,
    settings: TrendLogMultipleSettings,
) -> Result<TrendLogMultipleObject, Error> {
    let mut log = TrendLogMultipleObject::new(instance, name, buffer_size)?;
    log.restore_log_buffer(settings.total_record_count, [])?;
    for member in settings.members {
        log.add_property_reference(member)?;
    }
    if let Some(logging_type) = settings.logging_type {
        log.set_logging_type(logging_type)?;
    }
    if let Some(hundredths) = settings.log_interval {
        log.set_log_interval(hundredths)?;
    }
    if let Some((date, time)) = settings.start_time {
        log.set_start_time(date, time)?;
    }
    if let Some((date, time)) = settings.stop_time {
        log.set_stop_time(date, time)?;
    }
    if let Some(align) = settings.align_intervals {
        log.set_align_intervals(align);
    }
    if let Some(hundredths) = settings.interval_offset {
        log.set_interval_offset(hundredths);
    }
    Ok(log)
}

#[cfg(test)]
#[path = "trend_log_methods_tests.rs"]
mod tests;
