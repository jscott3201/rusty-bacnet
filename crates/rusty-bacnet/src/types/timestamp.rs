use super::*;

use pyo3::types::{PyBool, PyInt, PyTuple};

/// Python wrapper for the protocol's lossless `BACnetTimeStamp` CHOICE.
///
/// Construct one explicitly with `sequence_number`, `time`, or `date_time`.
/// Date fields accept the complete BACnet pattern domains: month 1..=14,
/// day 1..=34, day-of-week 1..=7, and 255 for an unspecified field. Time
/// fields accept their normal ranges or 255 for unspecified. A full year is
/// 1900..=2154, or 255 for unspecified.
#[pyclass(name = "BACnetTimeStamp", frozen, from_py_object)]
#[derive(Clone)]
pub struct PyBACnetTimeStamp {
    inner: primitives::BACnetTimeStamp,
}

impl PyBACnetTimeStamp {
    pub fn to_rust(&self) -> &primitives::BACnetTimeStamp {
        &self.inner
    }
}

fn integer(value: &Bound<'_, PyAny>, name: &str) -> PyResult<i128> {
    if value.is_instance_of::<PyBool>() || value.cast::<PyInt>().is_err() {
        return Err(PyValueError::new_err(format!("{name} must be an integer")));
    }
    value
        .extract::<i128>()
        .map_err(|_| PyValueError::new_err(format!("{name} must be an integer")))
}

fn ranged_or_unspecified(
    value: &Bound<'_, PyAny>,
    name: &str,
    minimum: u8,
    maximum: u8,
) -> PyResult<u8> {
    let value = integer(value, name)?;
    if value == i128::from(primitives::Time::UNSPECIFIED)
        || (i128::from(minimum)..=i128::from(maximum)).contains(&value)
    {
        return Ok(value as u8);
    }
    Err(PyValueError::new_err(format!(
        "{name} must be {minimum}..={maximum} or 255 (unspecified), got {value}"
    )))
}

fn full_year(value: &Bound<'_, PyAny>) -> PyResult<u8> {
    let value = integer(value, "full_year")?;
    match value {
        255 => Ok(primitives::Date::UNSPECIFIED),
        1900..=2154 => Ok((value - 1900) as u8),
        _ => Err(PyValueError::new_err(format!(
            "full_year must be 1900..=2154 or 255 (unspecified), got {value}"
        ))),
    }
}

fn time_parts(
    hour: &Bound<'_, PyAny>,
    minute: &Bound<'_, PyAny>,
    second: &Bound<'_, PyAny>,
    hundredths: &Bound<'_, PyAny>,
) -> PyResult<primitives::Time> {
    Ok(primitives::Time {
        hour: ranged_or_unspecified(hour, "hour", 0, 23)?,
        minute: ranged_or_unspecified(minute, "minute", 0, 59)?,
        second: ranged_or_unspecified(second, "second", 0, 59)?,
        hundredths: ranged_or_unspecified(hundredths, "hundredths", 0, 99)?,
    })
}

fn tuple4<'py>(
    value: &'py Bound<'py, PyAny>,
    name: &str,
    shape: &str,
) -> PyResult<&'py Bound<'py, PyTuple>> {
    let tuple = value.cast::<PyTuple>().map_err(|_| {
        PyValueError::new_err(format!(
            "{name} must be a tuple of exactly 4 integers: {shape}"
        ))
    })?;
    if tuple.len() != 4 {
        return Err(PyValueError::new_err(format!(
            "{name} must be a tuple of exactly 4 integers: {shape}"
        )));
    }
    Ok(tuple)
}

fn actual_year(date: &primitives::Date) -> u16 {
    date.actual_year()
        .unwrap_or(u16::from(primitives::Date::UNSPECIFIED))
}

fn date_value(date: &primitives::Date) -> (u16, u8, u8, u8) {
    (actual_year(date), date.month, date.day, date.day_of_week)
}

fn time_value(time: &primitives::Time) -> (u8, u8, u8, u8) {
    (time.hour, time.minute, time.second, time.hundredths)
}

#[pymethods]
impl PyBACnetTimeStamp {
    /// Construct the Sequence Number CHOICE (0..=65535).
    #[staticmethod]
    fn sequence_number(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = integer(value, "sequence number")?;
        let value = u16::try_from(value).map_err(|_| {
            PyValueError::new_err(format!("sequence number must be 0..=65535, got {value}"))
        })?;
        Ok(Self {
            inner: primitives::BACnetTimeStamp::SequenceNumber(value),
        })
    }

    /// Construct the Time CHOICE without normalizing any component.
    #[staticmethod]
    fn time(
        hour: &Bound<'_, PyAny>,
        minute: &Bound<'_, PyAny>,
        second: &Bound<'_, PyAny>,
        hundredths: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: primitives::BACnetTimeStamp::Time(time_parts(hour, minute, second, hundredths)?),
        })
    }

    /// Construct the DateTime CHOICE from `(full_year, month, day, day_of_week)`
    /// and `(hour, minute, second, hundredths)` tuples.
    #[staticmethod]
    fn date_time(date: &Bound<'_, PyAny>, time: &Bound<'_, PyAny>) -> PyResult<Self> {
        let date = tuple4(date, "date", "(full_year, month, day, day_of_week)")?;
        let time = tuple4(time, "time", "(hour, minute, second, hundredths)")?;
        let date = primitives::Date {
            year: full_year(&date.get_item(0)?)?,
            month: ranged_or_unspecified(&date.get_item(1)?, "month", 1, 14)?,
            day: ranged_or_unspecified(&date.get_item(2)?, "day", 1, 34)?,
            day_of_week: ranged_or_unspecified(&date.get_item(3)?, "day_of_week", 1, 7)?,
        };
        let time = time_parts(
            &time.get_item(0)?,
            &time.get_item(1)?,
            &time.get_item(2)?,
            &time.get_item(3)?,
        )?;
        Ok(Self {
            inner: primitives::BACnetTimeStamp::DateTime { date, time },
        })
    }

    /// Selected CHOICE: `sequence_number`, `time`, or `date_time`.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            primitives::BACnetTimeStamp::Time(_) => "time",
            primitives::BACnetTimeStamp::SequenceNumber(_) => "sequence_number",
            primitives::BACnetTimeStamp::DateTime { .. } => "date_time",
        }
    }

    /// Exact selected value: an integer, a Time tuple, or `(Date, Time)` tuples.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match &self.inner {
            primitives::BACnetTimeStamp::SequenceNumber(value) => {
                value.into_pyobject(py)?.into_any().unbind()
            }
            primitives::BACnetTimeStamp::Time(time) => {
                time_value(time).into_pyobject(py)?.into_any().unbind()
            }
            primitives::BACnetTimeStamp::DateTime { date, time } => {
                (date_value(date), time_value(time))
                    .into_pyobject(py)?
                    .into_any()
                    .unbind()
            }
        })
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            primitives::BACnetTimeStamp::SequenceNumber(value) => {
                format!("BACnetTimeStamp.sequence_number({value})")
            }
            primitives::BACnetTimeStamp::Time(time) => {
                let (hour, minute, second, hundredths) = time_value(time);
                format!("BACnetTimeStamp.time({hour}, {minute}, {second}, {hundredths})")
            }
            primitives::BACnetTimeStamp::DateTime { date, time } => {
                format!(
                    "BACnetTimeStamp.date_time({:?}, {:?})",
                    date_value(date),
                    time_value(time)
                )
            }
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
