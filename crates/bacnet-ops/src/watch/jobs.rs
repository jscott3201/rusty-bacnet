//! The requests a device's scheduler runs as jobs, one at a time: starting
//! a point (a first read, then a managed COV subscription), cancelling
//! one, polling a batch, and asking where a device that stopped answering
//! went. Every one goes to the point's address: a local station at its
//! MAC, a routed one through the router its row in the discovery table
//! names; none looks the device up by its instance. A start and a poll
//! also say what instance the table names at the address once answered,
//! and a verify reads only that, sending nothing (#2603).
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
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

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
    /// The instance at the address as the start began: what the device
    /// last named itself to the client, else the discovery table's row,
    /// else one read by the wildcard instance once the first read was
    /// answered (#2603).
    pub(super) before: Option<u32>,
    /// `before` is the device's own word, not the table's.
    pub(super) vouched: bool,
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
/// When neither the discovery table nor an earlier read knows the instance
/// at the address, it is read once by the wildcard instance after the
/// first read is answered, and remembered for the address (#2603).
pub(super) async fn start<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    shared: Arc<Shared>,
) -> Started {
    let (process, point) = (shared.process, &shared.point);
    let address = &point.device;
    let client = &inner.client;
    let said = client.said(address);
    let before = match said {
        Some(said) => Some(said),
        None => client.instance_at(address).await,
    };
    // The point's device from here, so a notification overtaking this
    // start is checked against what the device last said of itself.
    shared.learn(before, said.is_some());
    let known = client.cov_known(address, &point.watched);
    let target = path_to(client.low_level(), address).await;
    // No route: nothing is sent, and the point waits for one.
    let Ok(target) = target else {
        return Started {
            target: None,
            before,
            vouched: said.is_some(),
            instance: before,
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
    let first = read(&inner, point, address, before).await;
    let read_at = Some((Instant::now(), SystemTime::now()));
    // The point's device, from what answered at its address, unless the
    // device refused that read before.
    let unknown = shared.identity().is_none() && !client.wildcard_refused(address);
    let (before, vouched) = match before {
        None if unknown && first.is_ok() => (client.ask_instance(address).await.ok(), true),
        known => (known, said.is_some()),
    };
    let (outcome, observed, cov) = if first.as_ref().is_err_and(unreachable) {
        // Subscribing would only time out too.
        (Outcome::NotAsked, None, known)
    } else if known == CovSupport::Refused {
        (Outcome::Refused, None, CovSupport::Refused)
    } else {
        let outcome = subscribe(&inner, (target.clone(), address), process, point).await;
        let (observed, cov) = match &outcome {
            Ok(_) => (Some(true), CovSupport::Accepted),
            // Not the device's last word: it is asked again. A request
            // the client itself failed (a link reconnecting, say) is no
            // word of the device's at all.
            Err(error) if unreachable(error) || failed_locally(error) || fills_table(error) => {
                (None, known)
            }
            Err(error) => (refuses_service(error).then_some(false), CovSupport::Refused),
        };
        (Outcome::Asked(outcome), observed, cov)
    };
    // What the table names at the address now, once the start is answered.
    let instance = client.instance_at(address).await;
    // Kept for the address, unless another device may have answered.
    if let Some(accepted) = observed
        && (before.is_none() || instance.is_none() || before == instance)
    {
        client.cov_observed(address, &point.watched, accepted, before.or(instance));
    }
    Started {
        target: Some(target),
        before,
        vouched,
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
    /// The instance the device named in the same ReadPropertyMultiple, its
    /// Device object read by the wildcard instance: the poll's answer is
    /// that device's. `None` without RPM, or when it gave none.
    pub(super) answered: Option<u32>,
    /// The instance the discovery table, or a read by the wildcard
    /// instance, names at the address once the poll is answered.
    pub(super) table: Option<u32>,
}

/// The key of a poll's row that reads the Device object by the wildcard
/// instance: no subscriber process identifier is this one.
const IDENTITY_ROW: u32 = u32::MAX;

/// The row asking the device which instance it is, by the wildcard
/// instance, in a ReadPropertyMultiple: it rides with the first batch,
/// costs no request of its own, and none is sent for it without RPM.
fn identity_row() -> BatchRow<u32> {
    let any = ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE)
        .expect("the wildcard instance is a valid instance");
    BatchRow {
        key: IDENTITY_ROW,
        object: any,
        wanted: vec![(PropertyIdentifier::OBJECT_IDENTIFIER, None)],
        one_by_one: 0,
    }
}

/// The instance `rows` (an identity row's answer) name, if they name one.
fn named(rows: &[RawRow]) -> Option<u32> {
    let (_, raw) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::OBJECT_IDENTIFIER)?;
    match crate::format::decode_all(raw.as_ref().ok()?).as_deref() {
        Some([PropertyValue::ObjectIdentifier(device)])
            if device.object_type() == ObjectType::DEVICE =>
        {
            Some(device.instance_number())
        }
        _ => None,
    }
}

impl Polled {
    /// The device stopped answering: the poll gave up on the rows left.
    pub(super) fn unreachable(&self) -> bool {
        self.stopped.as_ref().is_some_and(unreachable)
    }
}

/// Poll `rows` of the device at `address` together, by RPM unless it
/// refused it, asking in the same request which instance the device is
/// (#2603), unless it refused the wildcard instance before.
pub(super) async fn poll<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    rows: Vec<BatchRow<u32>>,
) -> Polled {
    let client = &inner.client;
    let rpm = !client.refused_rpm(&address);
    let ask = rpm && !client.wildcard_refused(&address);
    // First, so it rides with the first batch whatever its size.
    let asked: Vec<BatchRow<u32>> = ask
        .then(identity_row)
        .into_iter()
        .chain(rows.iter().cloned())
        .collect();
    let read = batch::read(client.low_level(), client.limits(), &address, &asked, rpm).await;
    if read.as_ref().is_ok_and(|read| read.refused_rpm) {
        client.refuse_rpm(&address);
    }
    let riders_refused = read.as_ref().is_ok_and(|read| read.riders_refused);
    let (mut read_rows, stopped) = match read {
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
    let identity = read_rows
        .iter()
        .position(|(key, _)| *key == IDENTITY_ROW)
        .map(|at| read_rows.remove(at).1);
    let answered = identity
        .as_ref()
        .and_then(|rows| rows.as_deref().ok())
        .and_then(named);
    // Answered with no instance (an error, or left out), or its batch
    // taken only without it: it isn't asked so again.
    if ask && answered.is_none() && (identity.is_some() || riders_refused) {
        client.refuse_wildcard(&address);
    }
    let unread = match stopped {
        Some(_) => rows
            .iter()
            .map(|row| row.key)
            .filter(|key| !read_rows.iter().any(|(read, _)| read == key))
            .collect(),
        None => Vec::new(),
    };
    Polled {
        rows: read_rows,
        stopped,
        unread,
        answered,
        table: client.instance_at(&address).await,
    }
}

/// The instance the discovery table, or a read by the wildcard instance,
/// names at `address` now, after announcements there were missed. Sends
/// nothing.
pub(super) async fn verify<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
) -> Option<u32> {
    inner.client.instance_at(&address).await
}

/// What the device at an address said of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Answer {
    /// It is this instance.
    Is(u32),
    /// It has no Device object of the instance asked for.
    Not,
    /// It answered with another error, which says neither.
    Unclear,
    /// It didn't answer.
    Silent,
}

/// Ask the device at `address` which instance it is (#2603): one
/// ReadProperty of its Device object's Object_Identifier by the wildcard
/// instance, or, for a device that refused that before, of `watched`'s
/// Device object, which only that device has. Through the device's gate
/// and the client's rate, as any request.
pub(super) async fn confirm<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    watched: u32,
) -> Answer {
    let client = &inner.client;
    if !client.wildcard_refused(&address) {
        match client.ask_instance(&address).await {
            Ok(instance) => return Answer::Is(instance),
            Err(error) => {
                if let Some(answer) = silence(&error) {
                    return answer;
                }
            }
        }
    }
    probe(inner, address, watched).await
}

/// Whether the device at `address` has device `instance`'s Device object:
/// one ReadProperty of its Object_Identifier, which only that device
/// answers ([`Answer::Is`]); another answers that it has no such object
/// ([`Answer::Not`]). Through the device's gate and the client's rate.
pub(super) async fn probe<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    instance: u32,
) -> Answer {
    let Ok(device) = ObjectIdentifier::new(ObjectType::DEVICE, instance) else {
        return Answer::Unclear;
    };
    let property = PropertyIdentifier::OBJECT_IDENTIFIER;
    let reader = inner.client.reader(&address, Some(instance));
    match reader
        .rp(device, property, None)
        .await
        .map_err(device_error)
    {
        Ok(raw) => named(&[(property, Ok(raw))]).map_or(Answer::Unclear, Answer::Is),
        Err(error) => silence(&error).unwrap_or(match error.remote() {
            Some(RemoteError::Error { code, .. }) if code == ErrorCode::UNKNOWN_OBJECT => {
                Answer::Not
            }
            _ => Answer::Unclear,
        }),
    }
}

/// The answer `error` makes when the device said nothing: none, or the
/// client's own failure, which says nothing of it.
fn silence(error: &Error) -> Option<Answer> {
    if unreachable(error) {
        Some(Answer::Silent)
    } else if failed_locally(error) {
        Some(Answer::Unclear)
    } else {
        None
    }
}

/// A device's Time_Of_Device_Restart, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Restart {
    /// Its encoded value.
    At(Vec<u8>),
    /// The device has no such property, or wouldn't give it.
    Absent,
    /// No answer.
    Silent,
}

/// Read device `instance`'s Time_Of_Device_Restart at `address`, once.
pub(super) async fn restart_time<T: TransportPort + 'static>(
    inner: Arc<Inner<T>>,
    address: DeviceAddress,
    instance: u32,
) -> Restart {
    let Ok(device) = ObjectIdentifier::new(ObjectType::DEVICE, instance) else {
        return Restart::Absent;
    };
    let property = PropertyIdentifier::TIME_OF_DEVICE_RESTART;
    let reader = inner.client.reader(&address, Some(instance));
    match reader
        .rp(device, property, None)
        .await
        .map_err(device_error)
    {
        Ok(raw) => Restart::At(raw),
        Err(error) if unreachable(&error) || failed_locally(&error) => Restart::Silent,
        Err(_) => Restart::Absent,
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
