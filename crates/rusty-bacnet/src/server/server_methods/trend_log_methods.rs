//! Trend Log (#1480) and Trend Log Multiple (#1235) registration with the
//! configuration the server's poller acts on.
use super::super::*;
use bacnet_objects::log_persistence::FileLogPersistence;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::LoggingType;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use pyo3::exceptions::PyValueError;

use crate::types::date_time_tuple;

#[pymethods]
impl BACnetServer {
    /// Add a Trend Log object to the server (before starting).
    ///
    /// The keyword arguments configure what the server's poller does with
    /// it (#1480), as `add_trend_log_multiple`'s do. `reference` sets
    /// Log_DeviceObjectProperty, one element as `members` takes them.
    /// `logging_type` is `"polled"`, `"triggered"` or `"cov"`: POLLED with
    /// no `log_interval` takes a one-minute interval; TRIGGERED zeroes it and
    /// makes it read-only, so a `log_interval` with it raises
    /// WRITE_ACCESS_DENIED; COV zeroes it too, and a nonzero `log_interval`
    /// then turns the log back to POLLED. `client_cov_increment` is the
    /// increment a COV log holds a numeric value to, `None` (NULL) for the
    /// property's own criteria; `cov_resubscription_interval` is in seconds.
    /// The window, alignment and `total_record_count` are as on
    /// `add_trend_log_multiple`. `"polled"` with `log_interval=0` raises
    /// ValueError: the Clause 12.25.9 switch would make that a COV log.
    /// `log_interval=0` alone leaves the new log polled and idle.
    /// `storage_path` (a `str`) keeps the log in `<storage_path>.slot0` and
    /// `.slot1`, as on `add_trend_log_multiple`; `client_cov_increment` and
    /// `cov_resubscription_interval` aren't saved and apply on every start.
    #[pyo3(signature = (
        instance,
        name,
        buffer_size=100,
        *,
        reference=None,
        log_interval=None,
        logging_type=None,
        client_cov_increment=None,
        cov_resubscription_interval=None,
        start_time=None,
        stop_time=None,
        align_intervals=None,
        interval_offset=None,
        storage_path=None,
        total_record_count=0
    ))]
    fn add_trend_log(
        &self,
        instance: u32,
        name: &str,
        buffer_size: u32,
        reference: Option<&Bound<'_, PyAny>>,
        log_interval: Option<u32>,
        logging_type: Option<&str>,
        client_cov_increment: Option<f32>,
        cov_resubscription_interval: Option<u32>,
        start_time: Option<&Bound<'_, PyAny>>,
        stop_time: Option<&Bound<'_, PyAny>>,
        align_intervals: Option<bool>,
        interval_offset: Option<u32>,
        storage_path: Option<&str>,
        total_record_count: u32,
    ) -> PyResult<()> {
        if logging_type == Some("polled") && log_interval == Some(0) {
            return Err(PyValueError::new_err(
                "logging_type 'polled' with log_interval 0 would make a COV log \
                 (Clause 12.25.9); pass logging_type='cov', or a nonzero log_interval",
            ));
        }
        let reference = reference
            .map(|value| {
                let mut reference = vec![crate::types::property_reference_from_py(
                    value,
                    "reference",
                )?];
                crate::types::localize(
                    &mut reference,
                    crate::types::local_device(self.device_instance),
                );
                PyResult::Ok(reference.remove(0))
            })
            .transpose()?;
        let settings = TrendLogSettings {
            reference,
            log_interval,
            logging_type: logging_type.map(logging_type_from_py).transpose()?,
            client_cov_increment,
            cov_resubscription_interval,
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
        let obj =
            trend_log(instance, name, buffer_size, storage_path, settings).map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }

    /// Add a Trend Log Multiple object to the server (before starting).
    ///
    /// The keyword arguments configure what the server's poller does with
    /// it. `members` fills Log_DeviceObjectProperty in order, at most 64; a
    /// `device_identifier` that isn't a Device raises ValueError.
    /// `logging_type` is `"polled"` or `"triggered"`: POLLED with no
    /// `log_interval` takes a one-minute interval, and TRIGGERED zeroes it,
    /// so a `log_interval` with it raises WRITE_ACCESS_DENIED; `"cov"`
    /// raises VALUE_OUT_OF_RANGE, as a client's write of it does (a Trend
    /// Log Multiple never logs by COV), and any other string ValueError.
    /// `log_interval` is in hundredths of a second.
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
    /// `storage_path` (a `str`) keeps the log in `<storage_path>.slot0` and
    /// `.slot1` (#1541): it commits every 100 records or, while it changes,
    /// once a minute, and once more at `stop()`. A log reopened from those
    /// files keeps its saved records, Total_Record_Count, BUFFER_READY
    /// counts, Notification_Threshold and the settings a peer can write,
    /// ignoring the keyword arguments, and appends a LOG_INTERRUPTED record.
    /// `buffer_size` wins over the saved Buffer_Size; a smaller one keeps the
    /// newest records. A file that can't be read, or a `buffer_size` past
    /// what a slot holds, raises BacnetError.
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
        storage_path=None,
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
        storage_path: Option<&str>,
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
        let obj = trend_log_multiple(instance, name, buffer_size, storage_path, settings)
            .map_err(to_py_err)?;
        self.push_pending(Box::new(obj))
    }
}

fn logging_type_from_py(value: &str) -> PyResult<LoggingType> {
    match value {
        "polled" => Ok(LoggingType::POLLED),
        "triggered" => Ok(LoggingType::TRIGGERED),
        // A Trend Log takes it; a Trend Log Multiple refuses it as it
        // refuses a client's write.
        "cov" => Ok(LoggingType::COV),
        other => Err(PyValueError::new_err(format!(
            "logging_type must be 'polled', 'triggered' or 'cov', got {other:?}"
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
/// Logging_Type goes before Log_Interval, which it may set or lock. A log
/// reopened from `storage_path` keeps what it saved, so the settings, all of
/// which it saves, are left alone.
fn trend_log_multiple(
    instance: u32,
    name: &str,
    buffer_size: u32,
    storage_path: Option<&str>,
    settings: TrendLogMultipleSettings,
) -> Result<TrendLogMultipleObject, Error> {
    let mut log = match storage_path {
        Some(path) => TrendLogMultipleObject::with_persistence(
            instance,
            name,
            buffer_size,
            Arc::new(FileLogPersistence::new(path)?),
        )?,
        None => TrendLogMultipleObject::new(instance, name, buffer_size)?,
    };
    if log.reopened() {
        return Ok(log);
    }
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

/// The optional `add_trend_log` keyword arguments; `None` keeps the
/// object's default.
#[derive(Default)]
struct TrendLogSettings {
    reference: Option<BACnetDeviceObjectPropertyReference>,
    log_interval: Option<u32>,
    logging_type: Option<LoggingType>,
    client_cov_increment: Option<f32>,
    cov_resubscription_interval: Option<u32>,
    start_time: Option<(Date, Time)>,
    stop_time: Option<(Date, Time)>,
    align_intervals: Option<bool>,
    interval_offset: Option<u32>,
    /// Total_Record_Count to seed the empty log at.
    total_record_count: u32,
}

/// Build the log through the Rust setters, so Python gets their checks.
/// Logging_Type goes before Log_Interval, which it may set or lock, and
/// which may then switch POLLED and COV (Clause 12.25.9). A log reopened
/// from `storage_path` keeps what it saved: only the two COV settings, which
/// it doesn't save, apply then.
fn trend_log(
    instance: u32,
    name: &str,
    buffer_size: u32,
    storage_path: Option<&str>,
    settings: TrendLogSettings,
) -> Result<TrendLogObject, Error> {
    let mut log = match storage_path {
        Some(path) => TrendLogObject::with_persistence(
            instance,
            name,
            buffer_size,
            Arc::new(FileLogPersistence::new(path)?),
        )?,
        None => TrendLogObject::new(instance, name, buffer_size)?,
    };
    log.set_client_cov_increment(settings.client_cov_increment)?;
    if let Some(seconds) = settings.cov_resubscription_interval {
        log.set_cov_resubscription_interval(seconds)?;
    }
    if log.reopened() {
        return Ok(log);
    }
    log.restore_log_buffer(settings.total_record_count, [])?;
    log.set_log_device_object_property(settings.reference)?;
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
