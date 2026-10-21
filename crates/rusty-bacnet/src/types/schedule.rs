//! The typed write forms of a Schedule's Weekly_Schedule, Exception_Schedule
//! and Effective_Period and a Calendar's Date_List (#1499): the forms their
//! typed reads give as `.value` (see `constructed_py`), so a read can be
//! handed back to `add_schedule` or `add_calendar`.
//!
//! - A day is a list of `(time, value)` pairs, `time` an `(hour, minute,
//!   second, hundredths)` tuple and `value` a PropertyValue; a week is seven
//!   days, Monday first.
//! - A calendar entry is a mapping whose `kind` names the alternative:
//!   `date` with `date`, `date_range` with `start_date` and `end_date`, or
//!   `week_n_day` with `month`, `week_of_month` and `day_of_week`.
//! - An exception is a mapping of `period` (a calendar entry mapping, or a
//!   Calendar's ObjectIdentifier), `time_values` (a day) and `priority`.
//! - A date range is a `(start_date, end_date)` pair.
//!
//! Dates are `(year, month, day, day_of_week)` with the full year (see
//! `date`). This layer checks shapes and Python types; the object refuses
//! what BACnet doesn't allow, such as a time given twice in a day or a
//! priority outside 1 to 16, as a protocol error.

use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::primitives::{Date, Time};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use super::date::date_from_value;
use super::mapping::{
    discriminator, fixed_integer, mapping, object_identifier, required_item, validate_keys,
};
use super::{PyObjectIdentifier, PyPropertyValue};

/// The seven days of a Weekly_Schedule, Monday first.
pub(crate) fn weekly_schedule_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<Vec<Vec<BACnetTimeValue>>> {
    let days = list(value, "weekly_schedule")?;
    if days.len() != 7 {
        return Err(PyValueError::new_err(format!(
            "weekly_schedule must have 7 days, Monday first, got {}",
            days.len()
        )));
    }
    days.iter()
        .enumerate()
        .map(|(day, entries)| time_values(entries, &format!("weekly_schedule[{day}]")))
        .collect()
}

/// An Exception_Schedule, in order.
pub(crate) fn special_events_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<Vec<BACnetSpecialEvent>> {
    list(value, "exception_schedule")?
        .iter()
        .enumerate()
        .map(|(index, event)| special_event(event, &format!("exception_schedule[{index}]")))
        .collect()
}

/// A Date_List, in order.
pub(crate) fn calendar_entries_from_py(
    value: &Bound<'_, PyAny>,
) -> PyResult<Vec<BACnetCalendarEntry>> {
    list(value, "date_list")?
        .iter()
        .enumerate()
        .map(|(index, entry)| calendar_entry(entry, &format!("date_list[{index}]")))
        .collect()
}

/// An Effective_Period, `(start_date, end_date)`.
pub(crate) fn date_range_from_py(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetDateRange> {
    let shape = || PyTypeError::new_err(format!("{name} must be a (start_date, end_date) pair"));
    let pair = value.cast::<PyTuple>().map_err(|_| shape())?;
    if pair.len() != 2 {
        return Err(shape());
    }
    Ok(BACnetDateRange {
        start_date: date(&pair.get_item(0)?, &format!("{name} start_date"))?,
        end_date: date(&pair.get_item(1)?, &format!("{name} end_date"))?,
    })
}

fn list<'py>(value: &Bound<'py, PyAny>, name: &str) -> PyResult<Vec<Bound<'py, PyAny>>> {
    value
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("{name} must be a list")))
}

/// One day's `(time, value)` pairs.
fn time_values(value: &Bound<'_, PyAny>, name: &str) -> PyResult<Vec<BACnetTimeValue>> {
    list(value, name)?
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            let name = format!("{name}[{index}]");
            let shape = || PyTypeError::new_err(format!("{name} must be a (time, value) pair"));
            let pair = pair.cast::<PyTuple>().map_err(|_| shape())?;
            if pair.len() != 2 {
                return Err(shape());
            }
            let value = pair
                .get_item(1)?
                .extract::<PyPropertyValue>()
                .map_err(|_| PyTypeError::new_err(format!("{name} value must be a PropertyValue")))?
                .inner;
            Ok(BACnetTimeValue {
                time: time(&pair.get_item(0)?, &format!("{name} time"))?,
                value,
            })
        })
        .collect()
}

fn special_event(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetSpecialEvent> {
    let value = mapping(value, name)?;
    validate_keys(value, name, &["period", "time_values", "priority"], &[])?;
    let period = required_item(value, name, "period")?;
    let period = if period.extract::<PyObjectIdentifier>().is_ok() {
        SpecialEventPeriod::CalendarReference(object_identifier(&period, name)?)
    } else {
        SpecialEventPeriod::CalendarEntry(calendar_entry(&period, &format!("{name}.period"))?)
    };
    Ok(BACnetSpecialEvent {
        period,
        list_of_time_values: time_values(
            &required_item(value, name, "time_values")?,
            &format!("{name}.time_values"),
        )?,
        event_priority: fixed_integer::<u64>(
            &required_item(value, name, "priority")?,
            &format!("{name}.priority"),
        )?,
    })
}

fn calendar_entry(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetCalendarEntry> {
    let value = mapping(value, name)?;
    let field = |key: &str| format!("{name}.{key}");
    let octet = |key: &str| fixed_integer::<u8>(&required_item(value, name, key)?, &field(key));
    match discriminator(value, name)?.as_str() {
        "date" => {
            validate_keys(value, name, &["kind", "date"], &[])?;
            let entry = date(&required_item(value, name, "date")?, &field("date"))?;
            Ok(BACnetCalendarEntry::Date(entry))
        }
        "date_range" => {
            validate_keys(value, name, &["kind", "start_date", "end_date"], &[])?;
            Ok(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: date(
                    &required_item(value, name, "start_date")?,
                    &field("start_date"),
                )?,
                end_date: date(&required_item(value, name, "end_date")?, &field("end_date"))?,
            }))
        }
        "week_n_day" => {
            validate_keys(
                value,
                name,
                &["kind", "month", "week_of_month", "day_of_week"],
                &[],
            )?;
            Ok(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: octet("month")?,
                week_of_month: octet("week_of_month")?,
                day_of_week: octet("day_of_week")?,
            }))
        }
        kind => Err(PyValueError::new_err(format!(
            "{name}.kind must be 'date', 'date_range' or 'week_n_day', got {kind:?}"
        ))),
    }
}

/// A `(year, month, day, day_of_week)` tuple.
fn date(value: &Bound<'_, PyAny>, name: &str) -> PyResult<Date> {
    let parts = value.extract::<(u16, u8, u8, u8)>().map_err(|_| {
        PyTypeError::new_err(format!(
            "{name} must be a (year, month, day, day_of_week) tuple of ints"
        ))
    })?;
    date_from_value(parts)
}

/// An `(hour, minute, second, hundredths)` tuple.
fn time(value: &Bound<'_, PyAny>, name: &str) -> PyResult<Time> {
    let (hour, minute, second, hundredths) = value.extract::<(u8, u8, u8, u8)>().map_err(|_| {
        PyTypeError::new_err(format!(
            "{name} must be an (hour, minute, second, hundredths) tuple of ints"
        ))
    })?;
    Ok(Time {
        hour,
        minute,
        second,
        hundredths,
    })
}
