//! Python mapping boundary for Recipient_List destinations
//! (`BACnetDestination`, Clause 21), as `add_notification_forwarder` and
//! `add_notification_class` seed them (#1260, #1364).
//!
//! Each destination is a `Destination` mapping. Its recipient uses the
//! recipient mapping the Audit services take, and its time window uses the
//! tuple shape of `BACnetTimeStamp.time`. This layer checks shapes and Python
//! types only; the object's `add_destination` decides what the list accepts,
//! such as its length cap.

use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::BACnetDestination;
use bacnet_types::primitives::Time;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool};

use super::audit::recipient;
use super::mapping::{
    fixed_integer, mapping, optional_item, ranged_integer, required_item, validate_keys,
};
use super::timestamp::time_tuple;

const REQUIRED: &[&str] = &["recipient", "process_identifier"];
const OPTIONAL: &[&str] = &[
    "valid_days",
    "from_time",
    "to_time",
    "issue_confirmed_notifications",
    "transitions",
];

/// Read the `recipients=` seed of a Recipient_List, in order: nothing, or a
/// list of `Destination` mappings.
pub(crate) fn destinations(
    recipients: Option<Vec<Bound<'_, PyAny>>>,
) -> PyResult<Vec<BACnetDestination>> {
    recipients
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, value)| destination(value, &format!("recipients[{index}]")))
        .collect()
}

/// Read one `Destination` mapping. A key left out gives a destination that
/// is active every day, all day, for every transition, with unconfirmed
/// notifications.
pub(crate) fn destination(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetDestination> {
    let value = mapping(value, name)?;
    validate_keys(value, name, REQUIRED, OPTIONAL)?;
    let field = |key: &str| format!("{name}.{key}");
    let bits = |key: &str, maximum: u8| {
        optional_item(value, key)?
            .map(|item| ranged_integer::<u8>(&item, &field(key), 0..=maximum))
            .transpose()
    };
    let time = |key: &str| {
        optional_item(value, key)?
            .map(|item| time_tuple(&item, &field(key)))
            .transpose()
    };
    let issue_confirmed_notifications = match optional_item(value, "issue_confirmed_notifications")?
    {
        None => false,
        Some(item) if item.is_instance_of::<PyBool>() => item.extract()?,
        Some(_) => {
            return Err(PyTypeError::new_err(format!(
                "{} must be a bool",
                field("issue_confirmed_notifications")
            )));
        }
    };
    Ok(BACnetDestination {
        valid_days: bits("valid_days", DaysOfWeek::all().bits())?
            .map_or(DaysOfWeek::all(), DaysOfWeek::from_bits_truncate),
        from_time: time("from_time")?.unwrap_or(Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        }),
        to_time: time("to_time")?.unwrap_or(Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        }),
        recipient: recipient(
            &required_item(value, name, "recipient")?,
            &field("recipient"),
        )?,
        process_identifier: fixed_integer::<u32>(
            &required_item(value, name, "process_identifier")?,
            &field("process_identifier"),
        )?,
        issue_confirmed_notifications,
        transitions: bits("transitions", EventTransitionBits::all().bits())?.map_or(
            EventTransitionBits::all(),
            EventTransitionBits::from_bits_truncate,
        ),
    })
}

#[cfg(test)]
#[path = "destination_tests.rs"]
mod tests;
