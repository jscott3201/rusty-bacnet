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
//! it, and of a row read property by property the properties read before
//! it. Every request takes its own turn at the device's gate under the
//! shared [`RequestLimits`].
//!
//! [`Device::read_many`]: crate::Device::read_many

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::{BACnetClient, Station};
use bacnet_services::rpm::ReadAccessResult;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use super::{ReadVia, RpmFailure, failed_locally, rpm_failure, unreachable};
use crate::device::{DeviceAddress, path_to};
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

/// A row the device's ReadPropertyMultiple answer left out: no result for
/// its object where the answer should have had one.
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
    /// The device stopped answering, or the client's own failure ended the
    /// read after some rows: the rows after `rows` weren't read.
    pub(crate) stopped: Option<Error>,
    /// The last of `rows` was read property by property when the device
    /// stopped answering, so it holds only the properties read before.
    pub(crate) partial: bool,
    /// The device couldn't send a batch's answer for its size, so that
    /// batch was asked again a row at a time: a caller with more to read
    /// asks for fewer rows a call.
    pub(crate) too_large: bool,
    /// The caller's stop flag went up while a row was read property by
    /// property: the rows after `rows` weren't read, and the last of `rows`
    /// may be `partial`.
    pub(crate) cancelled: bool,
    /// Some rows were read by ReadPropertyMultiple.
    by_rpm: bool,
    /// Some rows were read by ReadProperty.
    by_rp: bool,
}

impl<K> Batched<K> {
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
/// holds.
///
/// # Errors
///
/// A routed address the discovery table holds no router for has no path
/// ([`Error::UnknownDevice`]); nothing is sent. A request that fails by
/// the client's or the link's own doing ([`failed_locally`], #2610) before
/// any row was read fails the read with that error; once rows were read
/// it ends the read as a device going quiet does, with those rows and the
/// error in [`Batched::stopped`]. It is never a row's own error.
pub(crate) async fn read<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: &DeviceAddress,
    rows: &[BatchRow<K>],
    rpm: bool,
) -> Result<Batched<K>, Error> {
    read_until(client, limits, device, rows, rpm, &super::NEVER_STOPPED).await
}

/// [`read`], ending before the next ReadProperty of a row read property
/// by property once `stop` is set ([`Batched::cancelled`]): a batch that
/// falls back to ReadProperty can be a hundred requests.
///
/// # Errors
///
/// As [`read`].
pub(crate) async fn read_until<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: &DeviceAddress,
    rows: &[BatchRow<K>],
    mut rpm: bool,
    stop: &AtomicBool,
) -> Result<Batched<K>, Error> {
    let started_rpm = rpm;
    let path = path_to(client, device).await?;
    let path = &path;
    let mut read = Batched {
        rows: Vec::with_capacity(rows.len()),
        refused_rpm: false,
        stopped: None,
        partial: false,
        too_large: false,
        cancelled: false,
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
                // The client's own failure: the next would fail the same
                // way. With nothing read the read fails; after that it ends
                // with what it has, as for a device gone quiet.
                Err(error) if failed_locally(&error) => {
                    if read.rows.is_empty() {
                        return Err(error);
                    }
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
                        read.too_large = true;
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
            let (rows, end) = one_by_one(client, limits, (device, path), row, stop).await;
            let some = !rows.is_empty();
            if some {
                read.by_rp = true;
                read.rows.push((row.key, Ok(rows)));
            }
            match end {
                None => {}
                // Some of this row's properties, never all: it stopped on one.
                Some(End::Silent(error)) => {
                    read.partial = some;
                    read.stopped = Some(error);
                    break 'batches;
                }
                Some(End::Failed(error)) => {
                    if read.rows.is_empty() {
                        return Err(error);
                    }
                    read.partial = some;
                    read.stopped = Some(error);
                    break 'batches;
                }
                Some(End::Cancelled) => {
                    read.partial = some;
                    read.cancelled = true;
                    break 'batches;
                }
            }
        }
    }
    read.refused_rpm = started_rpm && !rpm;
    Ok(read)
}

/// Whether `result` answers a read of `object`: it names that object, or
/// a Device object for one asked by the wildcard instance, which the
/// device answers as its own.
fn answers(object: ObjectIdentifier, result: &ReadAccessResult) -> bool {
    let named = result.object_identifier;
    named == object
        || (object.object_type() == ObjectType::DEVICE
            && object.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
            && named.object_type() == ObjectType::DEVICE)
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

/// Why a row read property by property ended before its last property.
enum End {
    /// The device stopped answering.
    Silent(Error),
    /// The client's own failure of the request
    /// ([`failed_locally`]): the next would fail the same way.
    Failed(Error),
    /// The caller's stop flag went up.
    Cancelled,
}

/// `row`'s first [`BatchRow::one_by_one`] properties by ReadProperty each:
/// those read before the device stopped answering or `stop` was set, and
/// which of the two ended it.
async fn one_by_one<K, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    (device, path): (&DeviceAddress, &Station),
    row: &BatchRow<K>,
    stop: &AtomicBool,
) -> (Vec<RawRow>, Option<End>) {
    let mut rows = Vec::with_capacity(row.one_by_one);
    for &(property, index) in row.wanted.iter().take(row.one_by_one.max(1)) {
        if stop.load(Ordering::Relaxed) {
            return (rows, Some(End::Cancelled));
        }
        let turn = limits.turn(device).await;
        let read = client
            .read_property(path, row.object, property, index)
            .await;
        drop(turn);
        let value = match read {
            Ok(ack) => Ok(ack.property_value),
            Err(error) if unreachable(&error) => return (rows, Some(End::Silent(error))),
            Err(error) if failed_locally(&error) => return (rows, Some(End::Failed(error))),
            Err(error) => Err(PropertyError::from(error)),
        };
        rows.push((property, value));
    }
    (rows, None)
}

/// One ReadPropertyMultiple for `batch`, one access specification per row.
async fn rpm_batch<K: Copy, T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    (device, path): (&DeviceAddress, &Station),
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
    let ack = client.read_property_multiple(path, specs).await;
    drop(turn);
    let ack = ack?;
    // Each row takes a result not yet taken that names its object,
    // wherever the answer put it: a device may answer out of order. Of two
    // such results (two rows for one object), the first whose first
    // property is the row's first wanted one, else the first of them. A
    // row with none is missing, and the rows after it still find theirs; a
    // result nobody asked for is left untaken.
    let mut results: Vec<Option<ReadAccessResult>> = ack
        .list_of_read_access_results
        .into_iter()
        .map(Some)
        .collect();
    Ok(batch
        .iter()
        .map(|row| {
            let fits = |slot: &Option<ReadAccessResult>, exactly: bool| {
                slot.as_ref().is_some_and(|result| {
                    let first = result.list_of_results.first();
                    let wanted = row.wanted.first().map(|&(property, _)| property);
                    answers(row.object, result)
                        && (!exactly || first.map(|e| e.property_identifier) == wanted)
                })
            };
            let at = results
                .iter()
                .position(|slot| fits(slot, true))
                .or_else(|| results.iter().position(|slot| fits(slot, false)));
            let found = at.and_then(|at| results[at].take());
            let Some(result) = found else {
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
