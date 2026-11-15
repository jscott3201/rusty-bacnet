//! Reading an inventory over a [`Reader`]: the summary, the Object_List,
//! then the objects, a few batches a call so each call's answers become
//! records before the next is sent.

use std::sync::atomic::Ordering;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::record::record;
use super::{Inventory, InventoryOptions, PointRecord, properties};
use crate::client::batch::{self, BatchRow, MAX_BATCH};
use crate::client::{
    DEVICE_PROPERTIES, DeviceSummary, Halt, Reader, RpmFailure, Wanted, rpm_failure, unreachable,
};
use crate::error::OpsError;
use crate::format::{PropertyError, RawRow};
use crate::reading::readings;
use crate::watch::CovSupport;

/// The inventory of `reader`'s device under `options`, for a front end
/// that holds a [`Reader`] and no [`Client`](crate::Client), such as the
/// `bacnet inventory` command: what [`Device::inventory`](crate::Device::inventory)
/// reads, at the same cost, each request at the reader's gate and rate
/// cap. A reader that meets a refusal of RPM leaves `rpm` false, for its
/// owner to remember; one that doesn't know its device's instance reads
/// it first, by the wildcard instance. [`Inventory::cov`] says only what
/// the summary's Protocol_Services_Supported lists.
///
/// # Errors
///
/// A device that doesn't answer the summary or the whole Object_List, or
/// refuses the list; and [`Halt::Cancelled`] once the reader's stop flag
/// is set, whatever was read.
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
    let mut inventory = Inventory {
        address: reader.device.clone(),
        device,
        cov,
        points: Vec::new(),
        unread: Vec::new(),
        stopped: None,
    };
    if let Some(stop) = list_stop {
        // A list that stopped part way means the device stopped answering,
        // or won't give the rest: asking for each object read so far would
        // list a device that isn't the whole one.
        inventory.stopped = Some(OpsError::at(&reader.device, stop.error));
        return Ok(inventory);
    }
    let wanted: Vec<ObjectIdentifier> = list
        .objects
        .into_iter()
        .filter(|object| options.reads(object.object_type()))
        .collect();
    inventory.points.reserve(wanted.len());
    let mut next = 0;
    let mut a_call = MAX_BATCH;
    while next < wanted.len() {
        if reader.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        let objects = &wanted[next..wanted.len().min(next + a_call)];
        let read = records(reader, objects, options, Some(&own)).await?;
        next += read.records.len();
        inventory.points.extend(read.records);
        if let Some(error) = read.stopped {
            inventory.unread = wanted[next..].to_vec();
            inventory.stopped = Some(OpsError::at(&reader.device, error));
            break;
        }
        if read.too_large {
            a_call = (a_call / 2).max(1);
        }
    }
    Ok(inventory)
}

/// One object's record as [`read_inventory`] reads it, for a front end
/// that pages a device's objects itself over a [`Reader`]: one
/// ReadPropertyMultiple, or a ReadProperty each property.
///
/// # Errors
///
/// A device that doesn't answer.
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

/// What one call read.
struct Records {
    /// The objects read whole, in the order asked.
    records: Vec<PointRecord>,
    /// The device stopped answering: the objects after `records` weren't
    /// read.
    stopped: Option<bacnet_types::error::Error>,
    /// An answer was too large for the device to send.
    too_large: bool,
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

/// The records of `objects`, read together ([`batch::read`]). What `own`
/// holds of the Device object is taken from it instead of asked for.
async fn records<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    objects: &[ObjectIdentifier],
    options: &InventoryOptions,
    own: Option<&Own>,
) -> Result<Records, Halt> {
    let instance = reader.instance().await?;
    let known = |object, property| own.and_then(|own| own.row(object, property));
    let rows: Vec<BatchRow<usize>> = objects
        .iter()
        .enumerate()
        .map(|(key, &object)| {
            let wanted: Vec<_> = properties(object.object_type(), options)
                .into_iter()
                .filter(|&property| known(object, property).is_none())
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
    let read = batch::read(
        reader.client,
        reader.rate,
        &reader.gate(),
        &rows,
        reader.rpm,
    )
    .await
    .map_err(Halt::Device)?;
    if read.refused_rpm {
        reader.rpm = false;
    }
    // An object read only in part before the device stopped is not read.
    let whole = read.rows.len() - usize::from(read.partial);
    let records = read
        .rows
        .into_iter()
        .take(whole)
        .zip(&rows)
        .map(|((_, answer), row)| {
            // In the order asked for, the summary's rows in their places.
            // A property the answer left out, or every property of an
            // object it left out, has no value.
            let mut answer: Vec<Option<RawRow>> =
                answer.unwrap_or_default().into_iter().map(Some).collect();
            let all = properties(row.object.object_type(), options)
                .into_iter()
                .map(|property| match known(row.object, property) {
                    Some(row) => row.clone(),
                    None => answer
                        .iter_mut()
                        .find(|slot| slot.as_ref().is_some_and(|(read, _)| *read == property))
                        .and_then(Option::take)
                        .unwrap_or((property, Err(PropertyError::NoValue))),
                })
                .collect();
            record(&reader.device, instance, row.object, all)
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
