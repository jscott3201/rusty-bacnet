//! The requests a device's scheduler runs as jobs, one at a time: starting
//! a point (a first read, then a managed COV subscription), cancelling
//! one, and polling a batch.
//!
//! Every request takes its turn at the device's gate in the engine's
//! limits, so the hub, the browser and the writes never have more than one
//! confirmed request outstanding to a device between them. The managed
//! subscription's renewals, sent by its own task once per lifetime, take
//! the same gate, though no rate slot. No lock is held across an await.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::Instant;

use bacnet_client::client::{
    CovPropertySubscription, CovTarget, ManagedCOVSubscription, ManagedCOVSubscriptionOptions,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;

use super::first::first_read;
use super::options::{Fallback, Reads};
use super::poll::{self, PollRow, Polled};
use super::shared::Shared;
use super::{Inner, Point, Watched};
use crate::engine::{Halt, network_of, unreachable};
use crate::format::{RawRow, RowError};

/// How long one cancel may take before it is given up.
const CANCEL_WAIT: Duration = Duration::from_secs(30);

/// How a point's start went.
pub(super) struct Started {
    /// Where the device is; `None` when it isn't in the device table, and
    /// then nothing was sent.
    pub(super) target: Option<CovTarget>,
    /// The first read, unless the device is missing.
    pub(super) first: Option<Result<Vec<RawRow>, Error>>,
    /// When the first read ended, for its sample.
    pub(super) read_at: Option<(Instant, SystemTime)>,
    /// The subscription, or why there is none; `None` when it wasn't
    /// asked for because the first read had no answer.
    pub(super) outcome: Option<Result<ManagedCOVSubscription, Error>>,
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
pub(super) fn poll_row(reads: &Reads, process: u32, point: &Point) -> PollRow<u32> {
    let wanted = wanted(reads, point);
    let one_by_one = match reads.fallback {
        Fallback::All => wanted.len(),
        Fallback::Watched => 1,
    };
    PollRow {
        key: process,
        object: point.object,
        wanted,
        one_by_one,
    }
}

/// Look `shared`'s device up and note where it is, read the point once,
/// and subscribe unless the read had no answer.
pub(super) async fn start<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    shared: Arc<Shared>,
) -> Started {
    let (process, point) = (shared.process, shared.point);
    let engine = &inner.engine;
    let Some(device) = engine.device(point.device).await else {
        return Started {
            target: None,
            first: None,
            read_at: None,
            outcome: None,
        };
    };
    let target = CovTarget::of_device(&device);
    // Before anything is sent: the device may notify as soon as it takes
    // the subscription, before this job is back with the scheduler, and
    // the receiver only takes notifications from where the device is.
    shared.lock().target = Some(target.clone());
    let first = read(&inner, &point).await;
    let read_at = Some((Instant::now(), SystemTime::now()));
    if let Err(error) = &first
        && unreachable(error)
    {
        // Subscribing would only time out too.
        return Started {
            target: Some(target),
            first: Some(first),
            read_at,
            outcome: None,
        };
    }
    let outcome = subscribe(&inner, target.clone(), process, point).await;
    Started {
        target: Some(target),
        first: Some(first),
        read_at,
        outcome: Some(outcome),
    }
}

/// The first read. A watch of one property alone, or of an array element,
/// is one ReadProperty, a property the device can't give an error row;
/// anything else goes through [`first_read`] with the hub's [`Reads`].
async fn read<T: TransportPort + 'static>(
    inner: &Inner<T>,
    point: &Point,
) -> Result<Vec<RawRow>, Error> {
    let engine = &inner.engine;
    let reads = &inner.options.reads;
    let mut wanted = wanted(reads, point);
    let mut reader = engine.reader(point.device);
    let alone = wanted.len() == 1 && reads.first.is_empty() && !reads.name;
    if let Watched::Property { property, index } = point.watched
        && (alone || index.is_some())
    {
        return match reader.rp(point.object, property, index).await {
            Ok(data) => Ok(vec![(property, Ok(data))]),
            Err(Halt::Device(error)) if unreachable(&error) => Err(error),
            Err(Halt::Device(error)) => Ok(vec![(property, Err(RowError::from(error)))]),
            Err(Halt::Cancelled) => Err(Error::Encoding("the read was stopped".into())),
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
    engine.settle(&reader);
    read.map_err(|halt| match halt {
        Halt::Device(error) => error,
        Halt::Cancelled => Error::Encoding("the read was stopped".into()),
    })
}

/// Ask for the managed subscription: SubscribeCOV for an object,
/// SubscribeCOVProperty for one property, unconfirmed and finite.
async fn subscribe<T: TransportPort + 'static>(
    inner: &Inner<T>,
    target: CovTarget,
    process: u32,
    point: Point,
) -> Result<ManagedCOVSubscription, Error> {
    let engine = &inner.engine;
    let options = &inner.options;
    let client = engine.client();
    let limits = engine.limits();
    let device = point.device;
    let network = network_of(client, limits, device).await;
    let managed = ManagedCOVSubscriptionOptions::default()
        .with_renewal_margin(options.renewal_margin)
        .with_renewal_gate(limits.gate(device));
    // The first request goes under this turn; renewals take the gate
    // themselves.
    let _turn = limits.turn_on(device, network).await;
    match point.watched {
        Watched::Object => {
            Arc::clone(client)
                .manage_cov_subscription(
                    target,
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
                .manage_cov_property_subscription(target, subscription, managed)
                .await
        }
    }
}

/// Poll `rows` of `device` together, by RPM unless it refused it.
pub(super) async fn poll<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    device: u32,
    rows: Vec<PollRow<u32>>,
) -> Polled<u32> {
    let engine = &inner.engine;
    let rpm = !engine.refused_rpm(device);
    let polled = poll::poll(engine.client(), engine.limits(), device, &rows, rpm).await;
    if polled.refused_rpm {
        let mut reader = engine.reader(device);
        reader.rpm = false;
        engine.settle(&reader);
    }
    polled
}

/// Cancel `point`'s subscription, held or possibly made: stop renewing it,
/// then send a subscription with neither a lifetime nor the confirmed
/// flag, which the device takes as a cancellation, to `target`, the way
/// the subscription went. It takes the device's turn first, so no renewal
/// is in flight when the renewals stop. Not charged to the hub's bucket.
pub(super) async fn cancel<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    process: u32,
    point: Point,
    target: CovTarget,
    managed: Option<ManagedCOVSubscription>,
) {
    let engine = &inner.engine;
    let client = engine.client();
    let limits = engine.limits();
    let cancel = async {
        let network = network_of(client, limits, point.device).await;
        let _turn = limits.turn_on(point.device, network).await;
        if let Some(managed) = managed {
            managed.stop().await;
        }
        match point.watched {
            Watched::Object => {
                client
                    .unsubscribe_cov_target(&target, process, point.object)
                    .await
            }
            Watched::Property { property, index } => {
                client
                    .unsubscribe_cov_property_target(
                        &target,
                        process,
                        point.object,
                        property,
                        index,
                    )
                    .await
            }
        }
    };
    match tokio::time::timeout(CANCEL_WAIT, cancel).await {
        Ok(Ok(())) => tracing::debug!("watch of {point:?}: COV subscription cancelled"),
        Ok(Err(error)) => tracing::debug!(
            "watch of {point:?}: the cancel failed ({error}); it ends with its lifetime"
        ),
        Err(_) => tracing::debug!(
            "watch of {point:?}: the cancel took too long; it ends with its lifetime"
        ),
    }
}
