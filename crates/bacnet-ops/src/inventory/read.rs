//! Reading an inventory over a [`Reader`]: the summary, the Object_List,
//! then the objects, one batch a call, so each batch's answers become
//! records before the next is sent.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::record::record;
use super::{Inventory, InventoryOptions, PointRecord, properties};
use crate::client::batch::{self, BatchRow, MAX_BATCH, MAX_BATCH_PROPERTIES};
use crate::client::{
    DEVICE_PROPERTIES, DeviceSummary, Halt, Reader, RpmFailure, Wanted, rpm_failure, unreachable,
};
use crate::error::OpsError;
use crate::format::{PropertyError, RawRow};
use crate::reading::readings;
use crate::watch::CovSupport;

/// The inventory of `reader`'s device under `options`: the layer under
/// [`Device::inventory`](crate::Device::inventory), for a caller that owns
/// a low-level client and a [`Reader`] and no [`Client`](crate::Client),
/// such as the `bacnet inventory` command. It reads what
/// [`Device::inventory`](crate::Device::inventory) reads, at the same cost
/// ([`inventory`](crate::inventory)), each request at the reader's gate
/// and rate cap. A reader that meets a refusal of RPM leaves `rpm` false,
/// for its owner to remember; one that doesn't know its device's instance
/// reads it first, by the wildcard instance. [`Inventory::cov`] says only
/// what the summary's Protocol_Services_Supported lists.
///
/// # Errors
///
/// A device that doesn't answer the summary or the Object_List, or
/// refuses the list, an element of a list read element by element
/// included. And [`Halt::Cancelled`] once the reader's stop flag is set,
/// checked before each batch and before each ReadProperty of a batch read
/// property by property: the records read before it are dropped with the
/// call, since `Halt` has nowhere to carry them.
pub async fn read_inventory<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    options: &InventoryOptions,
) -> Result<Inventory, Halt> {
    let device = summary(reader).await?;
    let cov = declared(&device);
    // What the summary read of the Device object is not asked for again
    // when that object's own record is read.
    let own = Own {
        object: reader.device_object()?,
        rows: device
            .readings
            .iter()
            .map(|reading| {
                let value = reading.value.clone().map(|value| value.raw);
                (reading.property, value)
            })
            .collect(),
    };
    let (list, list_stop) = reader.object_list_until(|_, _| {}).await?;
    if let Some(stop) = list_stop {
        // Part of a list is not the device: no object is asked for.
        return Err(Halt::Device(stop.error));
    }
    // A list that names an object twice gets one record for it.
    let mut seen = HashSet::new();
    let wanted: Vec<ObjectIdentifier> = list
        .objects
        .into_iter()
        .filter(|object| options.reads(object.object_type()) && seen.insert(*object))
        .collect();
    let mut inventory = Inventory::new(reader.device.clone(), device);
    inventory.cov = cov;
    inventory.points.reserve(wanted.len());
    let mut next = 0;
    let mut most = MAX_BATCH;
    while next < wanted.len() {
        if reader.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        let count = one_batch(&wanted[next..], most, options, &own);
        let objects = &wanted[next..next + count];
        let read = match records(reader, objects, options, Some(&own)).await {
            Ok(read) => read,
            // No route to the device any more: as a device gone silent.
            Err(Halt::Device(error)) => Records::stopped(error),
            Err(Halt::Cancelled) => return Err(Halt::Cancelled),
        };
        next += read.records.len();
        inventory.points.extend(read.records);
        if let Some(error) = read.stopped {
            inventory.unread = wanted[next..].to_vec();
            inventory.stopped = Some(OpsError::at(&reader.device, error));
            break;
        }
        if read.too_large {
            most = (count / 2).max(1);
        }
    }
    Ok(inventory)
}

/// One object's record as [`read_inventory`] reads it, for a caller that
/// pages a device's objects itself over a [`Reader`]: one
/// ReadPropertyMultiple, or a ReadProperty each property.
///
/// # Errors
///
/// A device that doesn't answer or has no route, and [`Halt::Cancelled`]
/// once the reader's stop flag is set.
pub async fn read_point_record<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    object: ObjectIdentifier,
    options: &InventoryOptions,
) -> Result<PointRecord, Halt> {
    let read = records(reader, &[object], options, None).await?;
    match (read.records.into_iter().next(), read.stopped) {
        (Some(record), None) => Ok(record),
        (_, Some(error)) => Err(Halt::Device(error)),
        (None, None) => unreachable!("a batch of one object reads it or stops"),
    }
}

/// What one batch read.
struct Records {
    /// The objects read whole, in the order asked.
    records: Vec<PointRecord>,
    /// The device stopped answering, or the route to it is gone: the
    /// objects after `records` weren't read.
    stopped: Option<Error>,
    /// The answer was too large for the device to send.
    too_large: bool,
}

impl Records {
    /// Nothing read, for `error`.
    fn stopped(error: Error) -> Self {
        Self {
            records: Vec::new(),
            stopped: Some(error),
            too_large: false,
        }
    }
}

/// The Device object, and what the summary already read of it.
struct Own {
    object: ObjectIdentifier,
    rows: Vec<RawRow>,
}

impl Own {
    /// The summary's row for `property` of `object`, when `object` is the
    /// Device object and the summary read it.
    fn row(&self, object: ObjectIdentifier, property: PropertyIdentifier) -> Option<&RawRow> {
        (object == self.object)
            .then(|| self.rows.iter().find(|(read, _)| *read == property))
            .flatten()
    }
}

/// The properties to ask of `object`: its list ([`properties`]), less
/// what `own` holds of the Device object.
fn asked(
    object: ObjectIdentifier,
    options: &InventoryOptions,
    own: Option<&Own>,
) -> Vec<PropertyIdentifier> {
    properties(object.object_type(), options)
        .into_iter()
        .filter(|&property| own.and_then(|own| own.row(object, property)).is_none())
        .collect()
}

/// How many of `objects`, from the first, make one ReadPropertyMultiple:
/// at most `most` objects and [`MAX_BATCH_PROPERTIES`] properties, and at
/// least one object. The batch reader would split a longer run itself, so
/// a call is exactly one request while the device answers it.
fn one_batch(
    objects: &[ObjectIdentifier],
    most: usize,
    options: &InventoryOptions,
    own: &Own,
) -> usize {
    let mut count = 0;
    let mut total = 0;
    for &object in objects.iter().take(most) {
        let more = asked(object, options, Some(own)).len();
        if count > 0 && total + more > MAX_BATCH_PROPERTIES {
            break;
        }
        total += more;
        count += 1;
    }
    count.max(1)
}

/// The records of `objects`, read together ([`batch::read_until`]). What
/// `own` holds of the Device object is taken from it instead of asked for.
async fn records<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    objects: &[ObjectIdentifier],
    options: &InventoryOptions,
    own: Option<&Own>,
) -> Result<Records, Halt> {
    let instance = reader.instance().await?;
    let rows: Vec<BatchRow<usize>> = objects
        .iter()
        .enumerate()
        .map(|(key, &object)| {
            let wanted: Vec<_> = asked(object, options, own)
                .into_iter()
                .map(|property| (property, None))
                .collect();
            BatchRow {
                key,
                object,
                one_by_one: wanted.len(),
                wanted,
            }
        })
        .collect();
    let read = batch::read_until(
        reader.client,
        reader.rate,
        &reader.gate(),
        &rows,
        reader.rpm,
        reader.cancel,
    )
    .await
    .map_err(Halt::Device)?;
    if read.refused_rpm {
        reader.rpm = false;
    }
    if read.cancelled {
        return Err(Halt::Cancelled);
    }
    // An object read only in part before the device stopped is not read.
    let whole = read.rows.len() - usize::from(read.partial);
    let records = read
        .rows
        .into_iter()
        .take(whole)
        .filter_map(|(key, answer)| {
            // The answer's row is the one asked under its key, whatever
            // order the answers came back in.
            let row = rows.get(key)?;
            // In the order asked for, the summary's rows in their places.
            // A property the answer left out, or every property of an
            // object it left out, has no value.
            let mut answer: Vec<Option<RawRow>> =
                answer.unwrap_or_default().into_iter().map(Some).collect();
            let all = properties(row.object.object_type(), options)
                .into_iter()
                .map(
                    |property| match own.and_then(|own| own.row(row.object, property)) {
                        Some(row) => row.clone(),
                        None => answer
                            .iter_mut()
                            .find(|slot| slot.as_ref().is_some_and(|(read, _)| *read == property))
                            .and_then(Option::take)
                            .unwrap_or((property, Err(PropertyError::NoValue))),
                    },
                )
                .collect();
            Some(record(&reader.device, instance, row.object, all))
        })
        .collect();
    Ok(Records {
        records,
        stopped: read.stopped,
        too_large: read.too_large,
    })
}

/// The device summary, as [`Device::describe_device`](crate::Device::describe_device)
/// reads it: one RPM, or a ReadProperty each once the device has refused
/// RPM or failed this one as a whole.
async fn summary<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
) -> Result<DeviceSummary, Halt> {
    let device = reader.instance().await?;
    let object = reader.device_object()?;
    let wanted = Wanted::List(&DEVICE_PROPERTIES);
    let (rows, via) = match reader.properties(object, wanted).await {
        Err(Halt::Device(error))
            if reader.rpm && !unreachable(&error) && rpm_failure(&error) == RpmFailure::Other =>
        {
            tracing::debug!(
                "{object}: the RPM failed as a whole ({error}); reading one property at a time"
            );
            reader.rpm = false;
            let read = reader.properties(object, wanted).await;
            // This RPM failed; the device didn't refuse the service.
            reader.rpm = true;
            read?
        }
        read => read?,
    };
    let readings = readings(rows.into_iter().map(|row| (object, None, row)).collect());
    Ok(DeviceSummary {
        device,
        readings,
        via,
    })
}

/// What `summary`'s Protocol_Services_Supported says of SubscribeCOV.
fn declared(summary: &DeviceSummary) -> CovSupport {
    use bacnet_types::enums::{PropertyIdentifier, ServiceSupported};
    let services = summary
        .value(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .and_then(|value| crate::watch::services(&value.raw));
    match services {
        Some(services) if services.contains(ServiceSupported::SUBSCRIBE_COV) => CovSupport::Offered,
        Some(_) => CovSupport::NotOffered,
        None => CovSupport::Unknown,
    }
}
