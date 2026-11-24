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
use crate::client::read::ListStop;
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
/// refuses the list ([`InventoryHalt::Device`]); an element of a list
/// read element by element that fails, which names the element
/// ([`InventoryHalt::ObjectList`]); and [`InventoryHalt::Cancelled`] once
/// the reader's stop flag is set, checked before each batch and before
/// each ReadProperty of a batch read property by property. A stop after
/// the Object_List is in keeps what was read, as a device going silent
/// does: the inventory in the halt holds the records read whole and
/// lists the rest in [`Inventory::unread`].
pub async fn read_inventory<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    options: &InventoryOptions,
) -> Result<Inventory, InventoryHalt> {
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
        return Err(InventoryHalt::ObjectList(stop));
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
    let sizing = Sizing::scan();
    let read = match read_objects(reader, &wanted, options, Some(&own), sizing).await {
        Ok(read) => read,
        // No route to the device any more, or the link failed the first
        // batch: as a device gone silent.
        Err(error) => Objects::stopped(error),
    };
    let next = read.points.len();
    inventory.points = read.points;
    if read.cancelled {
        inventory.unread = wanted[next..].to_vec();
        return Err(InventoryHalt::Cancelled(Some(Box::new(inventory))));
    }
    if let Some(error) = read.stopped {
        inventory.unread = wanted[next..].to_vec();
        inventory.stopped = Some(OpsError::at(&reader.device, error));
    }
    Ok(inventory)
}

/// What [`read_objects`] read of a run of objects.
pub(super) struct Objects {
    /// A record for each object read, in the order asked: the first
    /// `points.len()` of them.
    pub(super) points: Vec<PointRecord>,
    /// The device stopped answering, or the way to it failed once a batch
    /// was read: the objects after `points` weren't read.
    pub(super) stopped: Option<Error>,
    /// The reader's stop flag went up: the objects after `points` weren't
    /// read.
    pub(super) cancelled: bool,
    /// The batches' size at the end: smaller than it began once the device
    /// couldn't send a batch's answer for its size.
    pub(super) sizing: Sizing,
}

impl Objects {
    /// Nothing read, for `error`.
    fn stopped(error: Error) -> Self {
        Self {
            points: Vec::new(),
            stopped: Some(error),
            cancelled: false,
            sizing: Sizing::scan(),
        }
    }
}

/// How [`read_objects`] fills its batches, and how it shrinks them after
/// an answer the device couldn't send for its size: a batch the device
/// couldn't send, of more than one object, is the only thing that shrinks
/// them, never a timeout or an error of another kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Sizing {
    /// The most objects a batch holds.
    pub(super) objects: usize,
    /// The most properties a batch asks for, all its objects together.
    pub(super) properties: usize,
    /// Whether a batch too large to send halves `properties` (from the
    /// refused batch's count), so the limit carries over to other options;
    /// otherwise it halves `objects`, as the inventory always has.
    pub(super) by_properties: bool,
}

impl Sizing {
    /// The inventory's: [`MAX_BATCH`] objects and [`MAX_BATCH_PROPERTIES`]
    /// properties, the objects halved after a refusal.
    pub(super) fn scan() -> Self {
        Self {
            objects: MAX_BATCH,
            properties: MAX_BATCH_PROPERTIES,
            by_properties: false,
        }
    }

    /// Point records': [`MAX_BATCH`] objects and at most `properties`
    /// properties, the properties halved after a refusal.
    pub(super) fn pager(properties: usize) -> Self {
        Self {
            objects: MAX_BATCH,
            properties: properties.clamp(1, MAX_BATCH_PROPERTIES),
            by_properties: true,
        }
    }

    /// The sizing after the batch of `count` objects and `properties`
    /// properties came back too large to send.
    fn shrunk(self, count: usize, properties: usize) -> Self {
        if self.by_properties {
            Self {
                properties: (properties / 2).max(1),
                ..self
            }
        } else {
            Self {
                objects: (count / 2).max(1),
                ..self
            }
        }
    }
}

/// The records of `objects`, in the order asked, read in batches of at
/// most `sizing`'s objects and properties, one batch a call of
/// [`records`], so each batch's answers become records before the next is
/// sent. A batch the device couldn't send for its size shrinks the
/// batches after it ([`Sizing`]), down to one object. What `own` holds
/// of the Device object is taken from it instead of asked for. The loop
/// under [`read_inventory`] and [`Device::point_records`]: the same
/// records, problems and halts for the same objects.
///
/// [`Device::point_records`]: crate::Device::point_records
///
/// # Errors
///
/// The way to the device failing the first batch before anything was read
/// (no route, or a link that refuses the send): later, the same ends the
/// reading with what was read, in [`Objects::stopped`], as a device going
/// silent does.
pub(super) async fn read_objects<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    objects: &[ObjectIdentifier],
    options: &InventoryOptions,
    own: Option<&Own>,
    sizing: Sizing,
) -> Result<Objects, Error> {
    let mut read = Objects {
        points: Vec::with_capacity(objects.len()),
        stopped: None,
        cancelled: false,
        sizing,
    };
    let mut next = 0;
    while next < objects.len() {
        if reader.cancel.load(Ordering::Relaxed) {
            read.cancelled = true;
            break;
        }
        let (count, properties) = one_batch(&objects[next..], read.sizing, options, own);
        let batch = match records(reader, &objects[next..next + count], options, own).await {
            Ok(batch) => batch,
            Err(Halt::Device(error)) if read.points.is_empty() => return Err(error),
            // No route to the device any more: as a device gone silent.
            Err(Halt::Device(error)) => Records::stopped(error),
            // Before the batch's first request.
            Err(Halt::Cancelled) => {
                read.cancelled = true;
                break;
            }
        };
        next += batch.records.len();
        read.points.extend(batch.records);
        if batch.cancelled {
            read.cancelled = true;
            break;
        }
        if let Some(error) = batch.stopped {
            read.stopped = Some(error);
            break;
        }
        if batch.too_large {
            read.sizing = read.sizing.shrunk(count, properties);
        }
    }
    Ok(read)
}

/// Why [`read_inventory`] failed.
#[derive(Debug)]
pub enum InventoryHalt {
    /// The reader's stop flag went up. Once the Object_List is in, the
    /// inventory read until then: the records read whole, the objects not
    /// read in [`Inventory::unread`], and [`Inventory::stopped`] empty, as
    /// the device didn't end it. `None` when the flag went up while the
    /// summary or the list was read, before any record.
    Cancelled(Option<Box<Inventory>>),
    /// The device, or the way to it, failed the summary's read or the
    /// Object_List's.
    Device(Error),
    /// An Object_List read element by element failed at one element:
    /// written `Object_List element 3 of 40: ` and the error, so it
    /// doesn't read as the whole list failing.
    ObjectList(ListStop),
}

impl From<Halt> for InventoryHalt {
    fn from(halt: Halt) -> Self {
        match halt {
            Halt::Cancelled => Self::Cancelled(None),
            Halt::Device(error) => Self::Device(error),
        }
    }
}

impl std::fmt::Display for InventoryHalt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled(_) => f.write_str("the inventory was stopped"),
            Self::Device(error) => write!(f, "{error}"),
            Self::ObjectList(stop) => write!(f, "Object_List {stop}"),
        }
    }
}

impl std::error::Error for InventoryHalt {}

/// What one batch read.
pub(super) struct Records {
    /// The objects read whole, in the order asked.
    pub(super) records: Vec<PointRecord>,
    /// The device stopped answering, or the route to it is gone: the
    /// objects after `records` weren't read.
    stopped: Option<Error>,
    /// The answer was too large for the device to send.
    too_large: bool,
    /// The reader's stop flag went up while the batch was read property
    /// by property: the objects after `records` weren't read.
    cancelled: bool,
}

impl Records {
    /// Nothing read, for `error`.
    fn stopped(error: Error) -> Self {
        Self {
            records: Vec::new(),
            stopped: Some(error),
            too_large: false,
            cancelled: false,
        }
    }
}

/// The Device object, and what the summary already read of it.
pub(super) struct Own {
    pub(super) object: ObjectIdentifier,
    pub(super) rows: Vec<RawRow>,
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

/// How many of `objects`, from the first, make one ReadPropertyMultiple,
/// and the properties they ask for: at most `sizing`'s objects and
/// properties, and at least one object (none of none). The batch reader
/// splits a run only past [`MAX_BATCH`] objects or [`MAX_BATCH_PROPERTIES`]
/// properties, which `sizing` never exceeds, so a call is exactly one
/// request while the device answers it.
pub(super) fn one_batch(
    objects: &[ObjectIdentifier],
    sizing: Sizing,
    options: &InventoryOptions,
    own: Option<&Own>,
) -> (usize, usize) {
    let mut count = 0;
    let mut total = 0;
    for &object in objects.iter().take(sizing.objects.max(1)) {
        let more = asked(object, options, own).len();
        if count > 0 && total + more > sizing.properties {
            break;
        }
        total += more;
        count += 1;
    }
    (count, total)
}

/// The records of `objects`, read together ([`batch::read_until`]). What
/// `own` holds of the Device object is taken from it instead of asked for.
pub(super) async fn records<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    objects: &[ObjectIdentifier],
    options: &InventoryOptions,
    own: Option<&Own>,
) -> Result<Records, Halt> {
    let instance = reader.instance().await?;
    // Only the objects with something left to ask go to the device, each
    // row keyed by its object's place among `objects`.
    let rows: Vec<BatchRow<usize>> = objects
        .iter()
        .enumerate()
        .filter_map(|(key, &object)| {
            let wanted: Vec<_> = asked(object, options, own)
                .into_iter()
                .map(|property| (property, None))
                .collect();
            let row = BatchRow {
                key,
                object,
                one_by_one: wanted.len(),
                wanted,
            };
            (!row.wanted.is_empty()).then_some(row)
        })
        .collect();
    let (mut answers, stopped, too_large, cancelled) = if rows.is_empty() {
        (Vec::new(), None, false, false)
    } else {
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
        // An object read only in part before the device stopped, or the
        // stop flag went up, is not read.
        let whole = read.rows.len() - usize::from(read.partial);
        let answers: Vec<_> = read.rows.into_iter().take(whole).collect();
        (answers, read.stopped, read.too_large, read.cancelled)
    };
    // One record an object, in the order asked, each answer found by its
    // key whatever order the answers came back in. An object with nothing
    // to ask is built from what is known. While the device answers, an
    // object without an answer has no value for any property; once it has
    // stopped, or the stop flag went up, the first such object ends the
    // records.
    let mut records = Vec::with_capacity(objects.len());
    for (key, &object) in objects.iter().enumerate() {
        let sent = rows.iter().any(|row| row.key == key);
        let answer = answers
            .iter()
            .position(|(answered, _)| *answered == key)
            .map(|at| answers.swap_remove(at).1);
        if sent && answer.is_none() && (stopped.is_some() || cancelled) {
            break;
        }
        // In the order asked for, the summary's rows in their places. A
        // property the answer left out, or every property of an object it
        // left out, has no value.
        let mut answer: Vec<Option<RawRow>> = answer
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .map(Some)
            .collect();
        let all = properties(object.object_type(), options)
            .into_iter()
            .map(
                |property| match own.and_then(|own| own.row(object, property)) {
                    Some(row) => row.clone(),
                    None => answer
                        .iter_mut()
                        .find(|slot| slot.as_ref().is_some_and(|(read, _)| *read == property))
                        .and_then(Option::take)
                        .unwrap_or((property, Err(PropertyError::NoValue))),
                },
            )
            .collect();
        records.push(record(&reader.device, instance, object, all));
    }
    Ok(Records {
        records,
        stopped,
        too_large,
        cancelled,
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
