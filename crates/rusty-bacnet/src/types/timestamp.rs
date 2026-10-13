use super::*;

use bacnet_encoding::primitives::{decode_timestamp_choice, encode_timestamp_choice};
use pyo3::exceptions::PyOverflowError;
use pyo3::types::{PyBool, PyInt, PyTuple};

use super::date::{date_value, year_octet};

/// Python wrapper for the protocol's lossless `BACnetTimeStamp` CHOICE.
///
/// Construct one explicitly with `sequence_number`, `time`, or `date_time`.
/// Date fields accept the complete BACnet pattern domains: month 1..=14,
/// day 1..=34, day-of-week 1..=7, and 255 for an unspecified field. Time
/// fields accept their normal ranges or 255 for unspecified. A full year is
/// 1900..=2154, or 255 for unspecified (see [`super::date`]).
///
/// `copy` and `pickle` rebuild a timestamp from its CHOICE's octets
/// (#1500), so one read from a peer with a field outside those ranges
/// copies as it is.
#[pyclass(
    name = "BACnetTimeStamp",
    module = "rusty_bacnet",
    frozen,
    from_py_object
)]
#[derive(Clone)]
pub struct PyBACnetTimeStamp {
    inner: primitives::BACnetTimeStamp,
}

impl PyBACnetTimeStamp {
    pub fn to_rust(&self) -> &primitives::BACnetTimeStamp {
        &self.inner
    }

    pub(crate) fn from_rust(timestamp: primitives::BACnetTimeStamp) -> Self {
        Self { inner: timestamp }
    }
}

fn integer(value: &Bound<'_, PyAny>, name: &str) -> PyResult<i128> {
    if value.is_instance_of::<PyBool>() || value.cast::<PyInt>().is_err() {
        return Err(PyValueError::new_err(format!("{name} must be an integer")));
    }
    value
        .extract::<i128>()
        .map_err(|_| PyOverflowError::new_err(format!("{name} is out of range, got {value}")))
}

/// `value` as `T`, the width of the field it fills; outside `T` raises
/// OverflowError, as a parameter of that type does (#1360).
fn fixed<T: super::mapping::FixedWidth>(value: i128, name: &str) -> PyResult<T> {
    super::mapping::fit(value, name)
}

/// An octet field: outside unsigned8 raises OverflowError, and an octet
/// outside `minimum..=maximum` that isn't 255 (unspecified) ValueError.
fn ranged_or_unspecified(
    value: &Bound<'_, PyAny>,
    name: &str,
    minimum: u8,
    maximum: u8,
) -> PyResult<u8> {
    let value = fixed::<u8>(integer(value, name)?, name)?;
    if value == primitives::Time::UNSPECIFIED || (minimum..=maximum).contains(&value) {
        return Ok(value);
    }
    Err(PyValueError::new_err(format!(
        "{name} must be {minimum}..={maximum} or 255 (unspecified), got {value}"
    )))
}

fn full_year(value: &Bound<'_, PyAny>) -> PyResult<u8> {
    let value = fixed::<u16>(integer(value, "full_year")?, "full_year")?;
    year_octet(value, "full_year")
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

/// Read a `(hour, minute, second, hundredths)` tuple into a `Time`, with the
/// ranges `BACnetTimeStamp.time` takes.
pub(super) fn time_tuple(value: &Bound<'_, PyAny>, name: &str) -> PyResult<primitives::Time> {
    let time = tuple4(value, name, "(hour, minute, second, hundredths)")?;
    time_parts(
        &time.get_item(0)?,
        &time.get_item(1)?,
        &time.get_item(2)?,
        &time.get_item(3)?,
    )
}

/// Read a `(full_year, month, day, day_of_week)` tuple into a `Date`, with
/// the ranges `BACnetTimeStamp.date_time` takes.
fn date_tuple(value: &Bound<'_, PyAny>, name: &str) -> PyResult<primitives::Date> {
    let date = tuple4(value, name, "(full_year, month, day, day_of_week)")?;
    Ok(primitives::Date {
        year: full_year(&date.get_item(0)?)?,
        month: ranged_or_unspecified(&date.get_item(1)?, "month", 1, 14)?,
        day: ranged_or_unspecified(&date.get_item(2)?, "day", 1, 34)?,
        day_of_week: ranged_or_unspecified(&date.get_item(3)?, "day_of_week", 1, 7)?,
    })
}

/// Read a BACnetDateTime given as a `(date, time)` pair of those tuples.
pub(crate) fn date_time_tuple(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<(primitives::Date, primitives::Time)> {
    let shape = || {
        PyValueError::new_err(format!(
            "{name} must be a (date, time) pair: ((full_year, month, day, day_of_week), \
             (hour, minute, second, hundredths))"
        ))
    };
    let pair = value.cast::<PyTuple>().map_err(|_| shape())?;
    if pair.len() != 2 {
        return Err(shape());
    }
    Ok((
        date_tuple(&pair.get_item(0)?, &format!("{name} date"))?,
        time_tuple(&pair.get_item(1)?, &format!("{name} time"))?,
    ))
}

pub(crate) fn time_value(time: &primitives::Time) -> (u8, u8, u8, u8) {
    (time.hour, time.minute, time.second, time.hundredths)
}

#[pymethods]
impl PyBACnetTimeStamp {
    /// Construct the Sequence Number CHOICE (0..=65535).
    #[staticmethod]
    fn sequence_number(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = fixed::<u16>(integer(value, "sequence number")?, "sequence number")?;
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
        // Both shapes are checked before any field.
        tuple4(date, "date", "(full_year, month, day, day_of_week)")?;
        tuple4(time, "time", "(hour, minute, second, hundredths)")?;
        let date = date_tuple(date, "date")?;
        let time = time_tuple(time, "time")?;
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

    /// What `copy` and `pickle` call: `_from_octets` of the CHOICE's
    /// encoding, which holds every field exactly.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, (Bound<'py, PyBytes>,))> {
        let mut octets = BytesMut::new();
        encode_timestamp_choice(&mut octets, &slf.get().inner)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok((
            slf.get_type().getattr("_from_octets")?,
            (PyBytes::new(slf.py(), &octets),),
        ))
    }

    /// The timestamp `octets`, one encoded CHOICE, hold: what the pickles
    /// `__reduce__` makes call. Octets that aren't exactly one timestamp
    /// raise ValueError.
    #[staticmethod]
    fn _from_octets(octets: &[u8]) -> PyResult<Self> {
        match decode_timestamp_choice(octets, 0) {
            Ok((inner, end)) if end == octets.len() => Ok(Self { inner }),
            _ => Err(PyValueError::new_err(
                "octets are not exactly one encoded BACnetTimeStamp",
            )),
        }
    }
}
