//! Batched polls of one device's watched points (from the TUI's watch
//! list, #2115).
//!
//! The rows due on a device are read together: by ReadPropertyMultiple, up
//! to [`MAX_BATCH`] rows a request, each row its own access specification,
//! or by ReadProperty once the device has refused RPM (Clause 15.5). A
//! batch whose answer would be too large is asked again a row at a time;
//! any other batch the device fails as a whole is read by ReadProperty for
//! this poll only, so one bad object can't put errors on its neighbours. A device
//! that stops answering ends the poll, and the rows left get its error at
//! once. Every request takes its own turn at the device's gate under the
//! shared [`RequestLimits`].

use std::collections::VecDeque;

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use crate::engine::{RpmFailure, network_of, rpm_failure, unreachable};
use crate::format::{RawRow, RowError};
use crate::limits::{RequestLimits, Turn};

/// Rows read by one ReadPropertyMultiple when polling.
pub const MAX_BATCH: usize = 16;

/// Properties one ReadPropertyMultiple of a poll asks for at most, all its
/// rows together: a batch ends at [`MAX_BATCH`] rows or here, whichever
/// comes first, so rows that read many properties make smaller batches.
pub const MAX_BATCH_PROPERTIES: usize = 32;

/// Why a row of an answer has no reading.
pub const MISSING: &str = "missing from the device's answer";

/// One row of a poll: what it reads of which object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollRow<K> {
    /// The caller's name for the row.
    pub key: K,
    /// The object read.
    pub object: ObjectIdentifier,
    /// The properties a ReadPropertyMultiple asks for, each with its array
    /// element if one; the first is the watched one.
    pub wanted: Vec<(PropertyIdentifier, Option<u32>)>,
    /// How many of `wanted`, from the first, are read one by one by
    /// ReadProperty when RPM isn't used; at least one.
    pub one_by_one: usize,
}

/// One poll of a device's rows.
#[derive(Debug)]
pub struct Polled<K> {
    /// Every row asked for: its properties as read, or why it has none.
    pub rows: Vec<(K, Result<Vec<RawRow>, String>)>,
    /// The device refused RPM during this poll.
    pub refused_rpm: bool,
    /// The device stopped answering: the poll gave up on the rows left.
    pub unreachable: bool,
}

/// `device`'s turn, on its network when the limits pace networks.
async fn turn<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: u32,
) -> Turn {
    let network = network_of(client, limits, device).await;
    limits.turn_on(device, network).await
}

/// Read every row in `rows` of `device`, by RPM while `rpm` holds.
pub async fn poll<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: u32,
    rows: &[PollRow<K>],
    mut rpm: bool,
) -> Polled<K> {
    let started_rpm = rpm;
    let mut read = Vec::with_capacity(rows.len());
    let mut silent: Option<Error> = None;
    let mut batches: VecDeque<&[PollRow<K>]> = batches(rows).collect();
    while let Some(batch) = batches.pop_front() {
        if let Some(error) = &silent {
            read.extend(batch.iter().map(|row| (row.key, Err(error.to_string()))));
            continue;
        }
        if rpm {
            match rpm_batch(client, limits, device, batch).await {
                Ok(rows) => {
                    read.extend(rows);
                    continue;
                }
                Err(error) if unreachable(&error) => {
                    read.extend(batch.iter().map(|row| (row.key, Err(error.to_string()))));
                    silent = Some(error);
                    continue;
                }
                Err(error) => match rpm_failure(&error) {
                    RpmFailure::Refused => {
                        tracing::info!(
                            "device {device} refused ReadPropertyMultiple ({error}); polling one property at a time"
                        );
                        rpm = false;
                    }
                    RpmFailure::TooLarge if batch.len() > 1 => {
                        tracing::debug!(
                            "device {device}: batch answer too large ({error}); reading it a row at a time"
                        );
                        for at in (0..batch.len()).rev() {
                            batches.push_front(&batch[at..=at]);
                        }
                        continue;
                    }
                    RpmFailure::TooLarge | RpmFailure::Other => tracing::debug!(
                        "device {device}: batch read failed ({error}); reading it one property at a time"
                    ),
                },
            }
        }
        for row in batch {
            if let Some(error) = &silent {
                read.push((row.key, Err(error.to_string())));
                continue;
            }
            match one_by_one(client, limits, device, row).await {
                Ok(rows) => read.push((row.key, Ok(rows))),
                Err(error) => {
                    read.push((row.key, Err(error.to_string())));
                    silent = Some(error);
                }
            }
        }
    }
    Polled {
        rows: read,
        refused_rpm: started_rpm && !rpm,
        unreachable: silent.is_some(),
    }
}

/// `rows` in batches of at most [`MAX_BATCH`] rows and
/// [`MAX_BATCH_PROPERTIES`] properties; a row asking for more is a batch of
/// its own.
fn batches<K>(rows: &[PollRow<K>]) -> impl Iterator<Item = &[PollRow<K>]> {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start == rows.len() {
            return None;
        }
        let mut end = start;
        let mut properties = 0;
        while end < rows.len() && end - start < MAX_BATCH {
            let more = rows[end].wanted.len();
            if end > start && properties + more > MAX_BATCH_PROPERTIES {
                break;
            }
            properties += more;
            end += 1;
        }
        let batch = &rows[start..end];
        start = end;
        Some(batch)
    })
}

/// `row`'s first [`PollRow::one_by_one`] properties by ReadProperty each;
/// the error of a device that stopped answering.
async fn one_by_one<K, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: u32,
    row: &PollRow<K>,
) -> Result<Vec<RawRow>, Error> {
    let mut rows = Vec::with_capacity(row.one_by_one);
    for &(property, index) in row.wanted.iter().take(row.one_by_one.max(1)) {
        let turn = turn(client, limits, device).await;
        let read = client
            .read_property_from_device(device, row.object, property, index)
            .await;
        drop(turn);
        let value = match read {
            Ok(ack) => Ok(ack.property_value),
            Err(error) if unreachable(&error) => return Err(error),
            Err(error) => Err(RowError::from(error)),
        };
        rows.push((property, value));
    }
    Ok(rows)
}

/// One ReadPropertyMultiple for `batch`, one access specification per row.
async fn rpm_batch<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: u32,
    batch: &[PollRow<K>],
) -> Result<Vec<(K, Result<Vec<RawRow>, String>)>, Error> {
    let specs = batch
        .iter()
        .map(|row| ReadAccessSpecification {
            object_identifier: row.object,
            list_of_property_references: row
                .wanted
                .iter()
                .map(
                    |&(property_identifier, property_array_index)| PropertyReference {
                        property_identifier,
                        property_array_index,
                    },
                )
                .collect(),
        })
        .collect();
    let turn = turn(client, limits, device).await;
    let ack = client
        .read_property_multiple_from_device(device, specs)
        .await;
    drop(turn);
    let ack = ack?;
    // The answer lists the objects in the order they were asked for; a row
    // it leaves out gets an error.
    let mut results = ack.list_of_read_access_results.into_iter();
    Ok(batch
        .iter()
        .map(|row| {
            let Some(result) = results.next() else {
                return (row.key, Err(MISSING.to_string()));
            };
            let rows: Vec<RawRow> = result
                .list_of_results
                .into_iter()
                .map(|element| {
                    let value = match (element.property_value, element.error) {
                        (Some(value), _) => Ok(value),
                        (None, Some((class, code))) => {
                            Err(RowError::Remote(RemoteError::Error { class, code }))
                        }
                        (None, None) => Err(RowError::NoValue),
                    };
                    (element.property_identifier, value)
                })
                .collect();
            (row.key, Ok(rows))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;

    fn row(key: usize, properties: usize) -> PollRow<usize> {
        PollRow {
            key,
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            wanted: vec![(PropertyIdentifier::PRESENT_VALUE, None); properties],
            one_by_one: 1,
        }
    }

    fn sizes(rows: &[PollRow<usize>]) -> Vec<usize> {
        batches(rows).map(<[_]>::len).collect()
    }

    /// Two-property rows go sixteen to a batch; ten-property rows three;
    /// a row past the property cap goes alone.
    #[test]
    fn batches_end_at_sixteen_rows_or_thirty_two_properties() {
        let small: Vec<_> = (0..40).map(|k| row(k, 2)).collect();
        assert_eq!(sizes(&small), [16, 16, 8]);
        let points: Vec<_> = (0..7).map(|k| row(k, 10)).collect();
        assert_eq!(sizes(&points), [3, 3, 1]);
        let wide = [row(0, 40), row(1, 2), row(2, 40)];
        assert_eq!(sizes(&wide), [1, 1, 1]);
        assert!(sizes(&[]).is_empty());
    }
}
