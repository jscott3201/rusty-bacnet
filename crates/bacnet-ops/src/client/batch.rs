//! Batched reads of one device's properties: the watch hub's polls (from
//! the TUI's watch list, #2115) and [`Device::read_many`] (#2560).
//!
//! The rows are read together: by ReadPropertyMultiple, up to
//! [`MAX_BATCH`] rows a request, each row its own access specification, or
//! by ReadProperty once the device has refused RPM (Clause 15.5). A batch
//! whose answer would be too large is asked again a row at a time; any
//! other batch the device fails as a whole is read by ReadProperty for
//! this read only, so one bad object can't put errors on its neighbours. A
//! device that stops answering ends the read, keeping the rows read before
//! it. Every request takes its own turn at the device's gate under the
//! shared [`RequestLimits`].
//!
//! [`Device::read_many`]: crate::Device::read_many

use std::collections::VecDeque;

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use super::{ReadVia, RpmFailure, rpm_failure, unreachable};
use crate::device::{DeviceAddress, Path};
use crate::format::{PropertyError, RawRow};
use crate::limits::RequestLimits;

/// Rows read by one ReadPropertyMultiple.
pub(crate) const MAX_BATCH: usize = 16;

/// Properties one ReadPropertyMultiple asks for at most, all its rows
/// together: a batch ends at [`MAX_BATCH`] rows or here, whichever comes
/// first, so rows that read many properties make smaller batches.
pub(crate) const MAX_BATCH_PROPERTIES: usize = 32;

/// One row of a batch: what it reads of which object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchRow<K> {
    /// The caller's name for the row.
    pub(crate) key: K,
    /// The object read.
    pub(crate) object: ObjectIdentifier,
    /// The properties a ReadPropertyMultiple asks for, each with its array
    /// element if one; the first is the watched one.
    pub(crate) wanted: Vec<(PropertyIdentifier, Option<u32>)>,
    /// How many of `wanted`, from the first, are read one by one by
    /// ReadProperty when RPM isn't used; at least one.
    pub(crate) one_by_one: usize,
}

/// A row the device's ReadPropertyMultiple answer left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Missing;

/// What a batch read.
#[derive(Debug)]
pub(crate) struct Batched<K> {
    /// The rows read, in the order asked for, each its properties as read
    /// or [`Missing`]. The rows after them weren't read: see `stopped`.
    pub(crate) rows: Vec<(K, Result<Vec<RawRow>, Missing>)>,
    /// The device refused RPM during this read.
    pub(crate) refused_rpm: bool,
    /// Why the rows after `rows` weren't read: the device stopped
    /// answering, or a routed address has no path, and then nothing was
    /// sent.
    pub(crate) stopped: Option<Error>,
    /// Some rows were read by ReadPropertyMultiple.
    by_rpm: bool,
    /// Some rows were read by ReadProperty.
    by_rp: bool,
}

impl<K> Batched<K> {
    /// Nothing read, for `error`.
    pub(crate) fn stopped(error: Error) -> Self {
        Self {
            rows: Vec::new(),
            refused_rpm: false,
            stopped: Some(error),
            by_rpm: false,
            by_rp: false,
        }
    }

    /// How the rows were read; `None` when none was.
    pub(crate) fn via(&self) -> Option<ReadVia> {
        match (self.by_rpm, self.by_rp) {
            (true, true) => Some(ReadVia::Mixed),
            (true, false) => Some(ReadVia::Rpm),
            (false, true) => Some(ReadVia::Rp),
            (false, false) => None,
        }
    }
}

/// Read every row in `rows` of the device at `device`, by RPM while `rpm`
/// holds. A routed address the discovery table holds no router for reads
/// nothing, with nothing sent.
pub(crate) async fn read<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: &DeviceAddress,
    rows: &[BatchRow<K>],
    mut rpm: bool,
) -> Batched<K> {
    let started_rpm = rpm;
    let path = match Path::to(client, device).await {
        Ok(path) => path,
        Err(error) => return Batched::stopped(error),
    };
    let path = &path;
    let mut read = Batched {
        rows: Vec::with_capacity(rows.len()),
        refused_rpm: false,
        stopped: None,
        by_rpm: false,
        by_rp: false,
    };
    let mut batches: VecDeque<&[BatchRow<K>]> = batches(rows).collect();
    'batches: while let Some(batch) = batches.pop_front() {
        if rpm {
            match rpm_batch(client, limits, (device, path), batch).await {
                Ok(rows) => {
                    read.by_rpm = true;
                    read.rows.extend(rows);
                    continue;
                }
                Err(error) if unreachable(&error) => {
                    read.stopped = Some(error);
                    break;
                }
                Err(error) => match rpm_failure(&error) {
                    RpmFailure::Refused => {
                        tracing::info!(
                            "device {device} refused ReadPropertyMultiple ({error}); reading one property at a time"
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
            match one_by_one(client, limits, (device, path), row).await {
                Ok(rows) => {
                    read.by_rp = true;
                    read.rows.push((row.key, Ok(rows)));
                }
                Err(error) => {
                    read.stopped = Some(error);
                    break 'batches;
                }
            }
        }
    }
    read.refused_rpm = started_rpm && !rpm;
    read
}

/// `rows` in batches of at most [`MAX_BATCH`] rows and
/// [`MAX_BATCH_PROPERTIES`] properties; a row asking for more is a batch of
/// its own.
fn batches<K>(rows: &[BatchRow<K>]) -> impl Iterator<Item = &[BatchRow<K>]> {
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

/// `row`'s first [`BatchRow::one_by_one`] properties by ReadProperty each;
/// the error of a device that stopped answering.
async fn one_by_one<K, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    (device, path): (&DeviceAddress, &Path),
    row: &BatchRow<K>,
) -> Result<Vec<RawRow>, Error> {
    let mut rows = Vec::with_capacity(row.one_by_one);
    for &(property, index) in row.wanted.iter().take(row.one_by_one.max(1)) {
        let turn = limits.turn(device).await;
        let read = path
            .read_property(client, row.object, property, index)
            .await;
        drop(turn);
        let value = match read {
            Ok(ack) => Ok(ack.property_value),
            Err(error) if unreachable(&error) => return Err(error),
            Err(error) => Err(PropertyError::from(error)),
        };
        rows.push((property, value));
    }
    Ok(rows)
}

/// One ReadPropertyMultiple for `batch`, one access specification per row.
async fn rpm_batch<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    (device, path): (&DeviceAddress, &Path),
    batch: &[BatchRow<K>],
) -> Result<Vec<(K, Result<Vec<RawRow>, Missing>)>, Error> {
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
    let turn = limits.turn(device).await;
    let ack = path.read_property_multiple(client, specs).await;
    drop(turn);
    let ack = ack?;
    // The answer lists the objects in the order they were asked for; a row
    // it leaves out gets an error.
    let mut results = ack.list_of_read_access_results.into_iter();
    Ok(batch
        .iter()
        .map(|row| {
            let Some(result) = results.next() else {
                return (row.key, Err(Missing));
            };
            let rows: Vec<RawRow> = result
                .list_of_results
                .into_iter()
                .map(|element| {
                    let value = match (element.property_value, element.error) {
                        (Some(value), _) => Ok(value),
                        (None, Some((class, code))) => {
                            Err(PropertyError::Remote(RemoteError::Error { class, code }))
                        }
                        (None, None) => Err(PropertyError::NoValue),
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

    fn row(key: usize, properties: usize) -> BatchRow<usize> {
        BatchRow {
            key,
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            wanted: vec![(PropertyIdentifier::PRESENT_VALUE, None); properties],
            one_by_one: 1,
        }
    }

    fn sizes(rows: &[BatchRow<usize>]) -> Vec<usize> {
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
