//! The requests a device's scheduler runs as jobs, one at a time: starting
//! a point (a first read, then a managed COV subscription), cancelling
//! one, polling a batch, and asking where a device that stopped answering
//! went. Every one goes to the point's address: a local station at its
//! MAC, a routed one through the router its row in the discovery table
//! names; none looks the device up by its instance.
//!
//! Every request takes its turn at the device's gate in the client's
//! limits, so the hub, the browser and the writes never have more than one
//! confirmed request outstanding to a device between them. The managed
//! subscription's renewals, sent by its own task once per lifetime, take
//! the same gate, though no rate slot. No lock is held across an await.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::Instant;

use bacnet_client::client::{
    CovPropertySubscription, ManagedCOVSubscription, ManagedCOVSubscriptionOptions, Station,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;

use super::hub::Inner;
use super::hub::first::first_read;
use super::options::{Fallback, Reads};
use super::shared::Shared;
use super::support::{CovSupport, fills_table, refuses_service};
use super::{Point, Watched};
use crate::client::batch::{self, BatchRow, Missing};
use crate::client::{device_error, failed_locally, unreachable};
use crate::device::{DeviceAddress, path_to};
use crate::format::{PropertyError, RawRow};

/// How long one cancel may take before it is given up.
const CANCEL_WAIT: Duration = Duration::from_secs(30);

/// How a point's start went.
pub(super) struct Started {
    /// Where the device is; `None` when no route to its address is known,
    /// and then nothing was sent.
    pub(super) target: Option<Station>,
    /// The instance the discovery table, or a read by the wildcard
    /// instance, names at the address once the start is answered, if
    /// either does: not what it named when the start began, which a
    /// device taking the address meanwhile can change.
    pub(super) instance: Option<u32>,
    /// The first read, unless no route is known.
    pub(super) first: Option<Result<Vec<RawRow>, Error>>,
    /// When the first read ended, for its sample.
    pub(super) read_at: Option<(Instant, SystemTime)>,
    /// The subscription, or why there is none.
    pub(super) outcome: Outcome,
    /// What is known of the point's COV now.
    pub(super) cov: CovSupport,
}

/// Whether a start asked for its subscription, and how that went.
pub(super) enum Outcome {
    /// Not asked: the first read had no answer, or no route is known.
    NotAsked,
    /// Not asked: the device refused the service itself before.
    Refused,
    /// Asked: the subscription, or the device's refusal or silence.
    Asked(Result<ManagedCOVSubscription, Error>),
}

/// The properties `point` reads, each with its element: for a poll, and
/// with the first read's extras.
pub(super) fn wanted(reads: &Reads, point: &Point) -> Vec<(PropertyIdentifier, Option<u32>)> {
    match point.watched {
        Watched::Object => reads.object.iter().map(|p| (*p, None)).collect(),
        Watched::Property { property, index } => {
            let mut wanted = vec![(property, index)];
            if reads.property_flags && property != PropertyIdentifier::STATUS_FLAGS {
                wanted.push((PropertyIdentifier::STATUS_FLAGS, None));
            }
            wanted
        }
    }
}

/// `point` as a row of a poll.
pub(super) fn poll_row(reads: &Reads, process: u32, point: &Point) -> BatchRow<u32> {
    let wanted = wanted(reads, point);
    let one_by_one = match reads.fallback {
        Fallback::All => wanted.len(),
        Fallback::Watched => 1,
    };
    BatchRow {
        key: process,
        object: point.object,
        wanted,
        one_by_one,
    }
}

/// Note where `shared`'s device is, read the point once, and subscribe
/// unless the read had no answer or the device refused the service before.
pub(super) async fn start<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    shared: Arc<Shared>,
) -> Started {
    let (process, point) = (shared.process, &shared.point);
    let address = &point.device;
    let client = &inner.client;
    let instance = client.instance_at(address).await;
    let known = client.cov_known(address, &point.watched);
    let target = path_to(client.low_level(), address).await;
    // No route: nothing is sent, and the point waits for one.
    let Ok(target) = target else {
        return Started {
            target: None,
            instance,
            first: None,
            read_at: None,
            outcome: Outcome::NotAsked,
            cov: known,
        };
    };
    // Before anything is sent: the device may notify as soon as it takes
    // the subscription, before this job is back with the scheduler, and
    // the receiver only takes notifications from where the device is.
    shared.lock().target = Some(target.clone());
    let first = read(&inner, point, address, instance).await;
    let read_at = Some((Instant::now(), SystemTime::now()));
    let (outcome, cov) = if first.as_ref().is_err_and(unreachable) {
        // Subscribing would only time out too.
        (Outcome::NotAsked, known)
    } else if known == CovSupport::Refused {
        (Outcome::Refused, CovSupport::Refused)
    } else {
        let outcome = subscribe(&inner, (target.clone(), address), process, point).await;
        let cov = match &outcome {
            Ok(_) => {
                client.cov_observed(address, &point.watched, true);
                CovSupport::Accepted
            }
            // Not the device's last word: it is asked again. A request
            // the client itself failed (a link reconnecting, say) is no
            // word of the device's at all.
            Err(error) if unreachable(error) || failed_locally(error) || fills_table(error) => {
                known
            }
            Err(error) => {
                if refuses_service(error) {
                    client.cov_observed(address, &point.watched, false);
                }
                CovSupport::Refused
            }
        };
        (Outcome::Asked(outcome), cov)
    };
    // What the table names at the address now, once the start is answered.
    let instance = client.instance_at(address).await;
    Started {
        target: Some(target),
        instance,
        first: Some(first),
        read_at,
        outcome,
        cov,
    }
}

/// The first read. A watch of one property alone, or of an array element,
/// is one ReadProperty, a property the device can't give an error row;
/// anything else goes through [`first_read`] with the hub's [`Reads`].
async fn read<T: TransportPort + 'static>(
    inner: &Inner<T>,
    point: &Point,
    address: &DeviceAddress,
    instance: Option<u32>,
) -> Result<Vec<RawRow>, Error> {
    let client = &inner.client;
    let reads = &inner.options.reads;
    let mut wanted = wanted(reads, point);
    let mut reader = client.reader(address, instance);
    let alone = wanted.len() == 1 && reads.first.is_empty() && !reads.name;
    if let Watched::Property { property, index } = point.watched
        && (alone || index.is_some())
    {
        // The client's readers are never stopped by their flag.
        return match reader
            .rp(point.object, property, index)
            .await
            .map_err(device_error)
        {
            Ok(data) => Ok(vec![(property, Ok(data))]),
            // A request never sent fails the first read as it does for a
            // whole object's (`Reader::properties`, #2610), not as a row.
            Err(error) if unreachable(&error) || failed_locally(&error) => Err(error),
            Err(error) => Ok(vec![(property, Err(PropertyError::from(error)))]),
        };
    }
    wanted.extend(reads.first.iter().map(|p| (*p, None)));
    let properties: Vec<PropertyIdentifier> = wanted.iter().map(|(p, _)| *p).collect();
    let read = first_read(
        &mut reader,
        point.object,
        &properties,
        reads.name,
        reads.state_texts,
    )
    .await;
    client.settle(&reader);
    read.map_err(device_error)
}

/// Ask for the managed subscription: SubscribeCOV for an object,
/// SubscribeCOVProperty for one property, unconfirmed and finite.
async fn subscribe<T: TransportPort + 'static>(
    inner: &Inner<T>,
    (target, address): (Station, &DeviceAddress),
    process: u32,
    point: &Point,
) -> Result<ManagedCOVSubscription, Error> {
    let options = &inner.options;
    let client = inner.client.low_level();
    let limits = inner.client.limits();
    let managed = ManagedCOVSubscriptionOptions::default()
        .with_renewal_margin(options.renewal_margin)
        .with_renewal_gate(limits.gate(address));
    // The first request goes under this turn; renewals take the gate
    // themselves.
    let _turn = limits.turn(address).await;
    match point.watched {
        Watched::Object => {
            Arc::clone(client)
                .manage_cov_subscription(
                    &target,
                    process,
                    point.object,
                    false,
                    options.lifetime.secs(),
                    managed,
                )
                .await
        }
        Watched::Property { property, index } => {
            let subscription = CovPropertySubscription {
                subscriber_process_identifier: process,
                monitored_object_identifier: point.object,
                monitored_property_identifier: property,
                monitored_property_array_index: index,
                confirmed: false,
                lifetime: options.lifetime.0,
                cov_increment: None,
            };
            Arc::clone(client)
                .manage_cov_property_subscription(&target, subscription, managed)
                .await
        }
    }
}

/// One poll of a device's rows.
#[derive(Debug)]
pub(super) struct Polled {
    /// The rows read, in the order asked for: each its properties as read,
    /// or missing from the device's answer.
    pub(super) rows: Vec<(u32, Result<Vec<RawRow>, Missing>)>,
    /// Why the rows in `unread` weren't read: the device stopped answering,
    /// or it couldn't be reached at all (no row for it, or no route).
    pub(super) stopped: Option<Error>,
    /// The rows not read, one read only in part included; empty when every
    /// row was.
    pub(super) unread: Vec<u32>,
}

impl Polled {
    /// The device stopped answering: the poll gave up on the rows left.
    pub(super) fn unreachable(&self) -> bool {
        self.stopped.as_ref().is_some_and(unreachable)
    }
}

/// Poll `rows` of the device at `address` together, by RPM unless it
/// refused it.
pub(super) async fn poll<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    rows: Vec<BatchRow<u32>>,
) -> Polled {
    let client = &inner.client;
    let rpm = !client.refused_rpm(&address);
    let read = batch::read(client.low_level(), client.limits(), &address, &rows, rpm).await;
    if read.as_ref().is_ok_and(|read| read.refused_rpm) {
        client.refuse_rpm(&address);
    }
    let (read_rows, stopped) = match read {
        Ok(read) => {
            let mut read_rows = read.rows;
            // A row read only in part counts as not read: its point gets
            // the poll's error, not half a reading.
            if read.partial {
                read_rows.pop();
            }
            (read_rows, read.stopped)
        }
        Err(error) => (Vec::new(), Some(error)),
    };
    let unread = match stopped {
        Some(_) => rows[read_rows.len()..].iter().map(|row| row.key).collect(),
        None => Vec::new(),
    };
    Polled {
        rows: read_rows,
        stopped,
        unread,
    }
}

/// Cancel `point`'s subscription, held or possibly made: stop renewing it,
/// then send a subscription with neither a lifetime nor the confirmed
/// flag, which the device takes as a cancellation, to `target`, the way
/// the subscription went. It takes the device's turn first, so no renewal
/// is in flight when the renewals stop. Not charged to the hub's bucket.
/// False when the device didn't answer it.
pub(super) async fn cancel<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    process: u32,
    point: Point,
    target: Station,
    managed: Option<ManagedCOVSubscription>,
) -> bool {
    let client = inner.client.low_level();
    let limits = inner.client.limits();
    let cancel = async {
        let _turn = limits.turn(&point.device).await;
        if let Some(managed) = managed {
            managed.stop().await;
        }
        match point.watched {
            Watched::Object => client.unsubscribe_cov(&target, process, point.object).await,
            Watched::Property { property, index } => {
                client
                    .unsubscribe_cov_property(&target, process, point.object, property, index)
                    .await
            }
        }
    };
    match tokio::time::timeout(CANCEL_WAIT, cancel).await {
        Ok(Ok(())) => {
            tracing::debug!("watch of {point}: COV subscription cancelled");
            true
        }
        Ok(Err(error)) => {
            tracing::debug!(
                "watch of {point}: the cancel failed ({error}); it ends with its lifetime"
            );
            !unreachable(&error)
        }
        Err(_) => {
            tracing::debug!(
                "watch of {point}: the cancel took too long; it ends with its lifetime"
            );
            false
        }
    }
}

/// Where the device last at `address`, claiming `instance`, answers now,
/// after a request there went unanswered (see
/// [`Client::relocated`](crate::Client)); `None` while it may still be
/// there.
pub(super) async fn locate<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    instance: u32,
) -> Option<DeviceAddress> {
    inner.client.relocated(instance, &address).await
}
