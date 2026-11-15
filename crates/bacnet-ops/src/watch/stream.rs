//! [`Watch`]: an application's watch of many points, a stream of
//! [`Update`]s ([`Client::watch`]).
//!
//! A watch holds one lease a point on its client's own hub, made on first
//! use and kept while any watch holds a point. Each signalled change of a
//! point marks it in the watch's mailbox, at most once however many
//! changes come before the watch takes it, and wakes the watch once the
//! point's lock is let go; the update is built from the point's latest
//! snapshot when the watch takes it. So a slow consumer gets the latest
//! value of each point, never a queue of old ones, and the mailbox never
//! holds more than one mark a point; the hub never waits for a watch, so a
//! slow one holds up no other.

use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime};

use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::status_flags_from_bacnet;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{PropertyValue, StatusFlags};
use futures_core::Stream;

use super::hub::{
    self, Hub, Inner, Lease, MissingDevice, PROCESS_RANGES, Reads, Snapshot, Source, WatchOptions,
};
use super::sample::Via;
use super::support::CovSupport;
use super::{Point, WatchError, Why};
use crate::client::{Client, lock};
use crate::error::OpsError;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat, units};

/// The most points a client's own hub watches at once, every watch on the
/// client together, those still cancelling included.
pub const MAX_WATCHED: usize = 4096;

/// The settings of a client's own hub: five-minute subscriptions renewed
/// before they lapse; a device's polled points read together, every 10 s
/// at most, backing off to a minute when it stops answering; a point read
/// for its value with its Status_Flags, its Units, name and state texts;
/// every change signalled at once.
fn settings() -> WatchOptions {
    WatchOptions {
        reads: Reads::value(),
        coalesce: Duration::ZERO,
        missing: MissingDevice::Poll,
        max_points: MAX_WATCHED,
        ..WatchOptions::default()
    }
}

/// How a watched point's watch goes, with the reason when it isn't
/// subscribed. One `match` says what to do: wait, read on, or watch the
/// point somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// The first read and the subscription haven't finished.
    Starting,
    /// Subscribed: values come as COV notifications, the subscription
    /// renewed before each lifetime ends.
    Subscribed,
    /// Polled, with the device's other polled points, every 10 s at most,
    /// because of `why`:
    ///
    /// - [`Why::Refused`]: the device said no (or refused the service for
    ///   an earlier point); not asked again.
    /// - [`Why::NoAnswer`]: the first read or the subscription had no
    ///   answer; subscribed again after a backoff.
    /// - [`Why::Full`]: the device's subscription table was full;
    ///   subscribed again after a backoff.
    /// - [`Why::Lapsed`]: a renewal failed; subscribed again after a
    ///   backoff.
    Polled {
        /// Why it isn't subscribed.
        why: Why,
    },
    /// The device didn't answer the last request: polled again after a
    /// backoff that grows to a minute, then as [`Polled`](Self::Polled)
    /// once it answers. `why` it isn't subscribed: any of the reasons a
    /// polled point carries, or [`Why::Missing`] while no route to a
    /// remote address is known.
    Unreachable {
        /// Why it isn't subscribed.
        why: Why,
    },
    /// Given up: nothing more is sent for it. [`Why::Moved`]: the device
    /// answers at another address now, so watch the point there; the
    /// point starts again if its old address answers for the instance
    /// again.
    Gone {
        /// Why it was given up.
        why: Why,
    },
}

impl State {
    /// The application's state of a point the hub reports as `snapshot`.
    fn of(snapshot: &Snapshot) -> Self {
        let why = snapshot.why.clone();
        match (snapshot.state, snapshot.source, why) {
            (hub::Phase::Starting, ..) => Self::Starting,
            (hub::Phase::Gone | hub::Phase::Stopped, _, why) => Self::Gone {
                why: why.unwrap_or(Why::Missing),
            },
            (hub::Phase::Unreachable, _, Some(why)) => Self::Unreachable { why },
            (_, Some(Source::Cov), _) => Self::Subscribed,
            (_, _, Some(why)) => Self::Polled { why },
            // The hub gives every polled or unreachable point its reason
            // in the same step; a point read between two steps of its
            // start hasn't started yet.
            (_, _, None) => Self::Starting,
        }
    }
}

/// One change of a watched point: its latest value and how its watch goes.
///
/// An update comes with each value read or notified (the first read, each
/// COV notification, each poll) and each change of the watch's state. A
/// watch whose consumer falls behind folds a point's changes into one
/// update with the latest of them.
///
/// `value` and `error` together say where the point's value stands:
/// `value` alone is the value as last read or notified; `value` and
/// `error` together, a value read earlier while the latest read failed
/// (the error says why); `error` alone, no value read yet, or the device
/// answers the property with an error; neither, no value yet.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Update {
    /// The point.
    pub point: Point,
    /// How its watch goes.
    pub state: State,
    /// The property's value as last read or notified (an object watch's
    /// Present_Value), its text naming the object's units or state when
    /// the first read had them.
    pub value: Option<Value>,
    /// Why the latest read gave no value: the device's error for the
    /// property, for the whole object, or its answer leaving the object
    /// out. Cleared by the next value read or notified.
    pub error: Option<PropertyError>,
    /// The object's Status_Flags as last read or notified with the value
    /// ([`StatusFlags::IN_ALARM`], `FAULT`, `OVERRIDDEN`,
    /// `OUT_OF_SERVICE`).
    pub status_flags: Option<StatusFlags>,
    /// How the latest values came: the first read, a COV notification or
    /// a poll.
    pub via: Option<Via>,
    /// When the latest values came, by the wall clock: what an application
    /// shows, logs or hands on with the value; `elapsed()` is its age
    /// unless the clock was set meanwhile.
    pub updated: Option<SystemTime>,
    /// What is known of this point's COV subscription.
    pub cov: CovSupport,
}

/// A watch of many points: a [`Stream`] of their [`Update`]s.
///
/// Made by [`Client::watch`]. Each point is watched once however many
/// watches hold it: one COV subscription, renewed before its lifetime
/// ends, or, where the device refuses COV, a poll that reads all of a
/// device's polled points together.
///
/// **Back-pressure.** The latest value of each point wins: a consumer
/// that falls behind gets one update a point with its latest value and
/// state, never a queue of old ones ([`folded`](Self::folded) counts what
/// was folded). Nothing grows past one pending update a point, and the hub
/// never waits for a consumer, so a slow watch holds up no other.
///
/// **Dropping.** Dropping the watch lets go of its points. A point no
/// other watch holds stops: a subscribed one is cancelled on the wire with
/// the request that made it, without a lifetime (SubscribeCOV, or
/// SubscribeCOVProperty for a property watch), sent through the device's
/// gate. A device in its backoff, or one that leaves a cancel unanswered,
/// is sent no further cancels: those subscriptions end with their
/// lifetimes. A polled point is no longer polled, and nothing is sent for
/// it.
///
/// **Ending.** The stream of a watch of no points ends at once. Any other
/// never ends on its own: a point given up ([`State::Gone`]) sends its last
/// update and stays, and while its old subscription lapses the device may
/// still notify it. Drop the watch to stop.
pub struct Watch {
    leases: Vec<Lease>,
    mailbox: Arc<Mailbox>,
}

impl Watch {
    /// The points watched, in the order named, each once, with its
    /// device's address as the hub keys it.
    pub fn points(&self) -> impl Iterator<Item = &Point> + '_ {
        self.leases.iter().map(|lease| &lease.shared.point)
    }

    /// Changes folded into a later update of the same point because this
    /// watch hadn't taken the earlier one yet.
    pub fn folded(&self) -> u64 {
        lock(&self.mailbox.state).folded
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch")
            .field("points", &self.leases.len())
            .finish()
    }
}

impl Stream for Watch {
    type Item = Update;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Update>> {
        let this = self.get_mut();
        if this.leases.is_empty() {
            return Poll::Ready(None);
        }
        let index = {
            let mut state = lock(&this.mailbox.state);
            match state.queue.pop_front() {
                Some(index) => {
                    state.queued[index] = false;
                    index
                }
                None => {
                    // Under the lock a post takes too: none slips between.
                    state.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        Poll::Ready(Some(update(&this.leases[index])))
    }
}

/// A watch's mailbox: the points changed since it last took them, each
/// once, in the order they changed.
pub(super) struct Mailbox {
    state: Mutex<MailState>,
}

struct MailState {
    queue: VecDeque<usize>,
    queued: Vec<bool>,
    folded: u64,
    waker: Option<Waker>,
}

/// One watch's place among a point's listeners: its mailbox, and the
/// point's index in that watch.
pub(crate) struct Listener {
    mailbox: Weak<Mailbox>,
    index: usize,
}

impl Listener {
    /// Mark the point changed in the watch, adding the watch's waker to
    /// `wakers` for the caller to wake once its lock is let go; false once
    /// the watch is gone.
    pub(super) fn post(&self, wakers: &mut Vec<Waker>) -> bool {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return false;
        };
        let mut state = lock(&mailbox.state);
        if state.queued[self.index] {
            state.folded += 1;
        } else {
            state.queued[self.index] = true;
            state.queue.push_back(self.index);
            wakers.extend(state.waker.take());
        }
        true
    }
}

impl Lease {
    /// Tell `listener` of every change signalled from now on, and of where
    /// the point stands now if it has changed before.
    fn listen(&self, listener: Listener) {
        let mut state = self.shared.lock();
        state
            .listeners
            .retain(|held| held.mailbox.strong_count() > 0);
        let mut wakers = Vec::new();
        if state.snapshot.seq > 0 {
            listener.post(&mut wakers);
        }
        state.listeners.push(listener);
        drop(state);
        wakers.into_iter().for_each(Waker::wake);
    }
}

/// `lease`'s point as it stands: its value from the latest rows, named by
/// the units and state texts among them.
fn update(lease: &Lease) -> Update {
    let point = lease.shared.point.clone();
    let (property, _) = point.property();
    let (rows, via, state, cov, failure, updated) = {
        let entry = lease.shared.lock();
        let snapshot = &entry.snapshot;
        (
            snapshot.rows.clone(),
            snapshot.via,
            State::of(snapshot),
            snapshot.cov,
            entry.failure.clone(),
            entry.wall,
        )
    };
    let (value, error) = match rows.iter().find(|(p, _)| *p == property) {
        Some((_, Ok(raw))) => (Some(value(&point, &rows, raw.clone())), failure),
        Some((_, Err(error))) => (None, Some(error.clone())),
        None => (None, failure),
    };
    Update {
        point,
        state,
        value,
        error,
        status_flags: status_flags(&rows),
        via,
        updated,
        cov,
    }
}

/// The watched property's `raw` octets as a [`Value`], every digit, with
/// the units and state texts `rows` hold.
fn value(point: &Point, rows: &[RawRow], raw: Vec<u8>) -> Value {
    let (property, index) = point.property();
    let texts = StateTexts::from_rows(rows).cleaned();
    let format = ValueFormat::new(point.object.object_type(), &texts, Floats::Exact).whole_lists();
    Value::new(&format, property, index, raw, units(rows))
}

/// The Status_Flags among `rows`, when they hold a bit string.
fn status_flags(rows: &[RawRow]) -> Option<StatusFlags> {
    let (_, raw) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::STATUS_FLAGS)?;
    let raw = raw.as_deref().ok()?;
    match bacnet_encoding::primitives::decode_application_value(raw, 0) {
        Ok((PropertyValue::BitString { data, .. }, _)) => Some(status_flags_from_bacnet(&data)),
        _ => None,
    }
}

/// What a client keeps of its own hub: the hub while any watch holds a
/// point, weak since the hub holds the client, and the range of subscriber
/// process identifiers every one of its hubs takes, the same for the
/// client's life so no re-made hub ever takes another hub's.
pub(crate) struct OwnHub<T: TransportPort + 'static> {
    pub(in crate::watch) hub: Weak<Inner<T>>,
    range: Option<u32>,
}

impl<T: TransportPort + 'static> OwnHub<T> {
    pub(crate) fn new() -> Self {
        Self {
            hub: Weak::new(),
            range: None,
        }
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// Watch `points`, from [`Device::point`](crate::Device::point): a
    /// [`Watch`] streaming each point's [`Update`]s.
    ///
    /// The points go to this client's own hub, made on first use and
    /// shared by every watch on the client: a point two watches hold is
    /// one subscription. Each is read once for its value, Status_Flags,
    /// Units, name and state texts, then subscribed for five minutes at a
    /// time, renewed before each lifetime ends; a point the device won't
    /// subscribe is polled with the device's other polled points, every
    /// 10 s at most. A device that refuses the COV service itself is
    /// remembered by its address, and its later points are polled without
    /// asking. A point named twice, or one station named two ways, is
    /// watched once.
    ///
    /// The hub works with the client's base settings, whichever clone
    /// calls: a following handle's device that stops answering is looked
    /// for with [`RESOLVE_WAIT`](crate::client::RESOLVE_WAIT), as by a
    /// clone [`Client::new`] made.
    ///
    /// # Errors
    ///
    /// [`OpsError::Watch`]: [`WatchError::Full`] when the points would take
    /// the client's hub past [`MAX_WATCHED`] (nothing is watched then, and
    /// nothing sent), and [`WatchError::NoRuntime`] outside a Tokio
    /// runtime, which the hub's tasks need.
    pub fn watch(&self, points: impl IntoIterator<Item = Point>) -> Result<Watch, OpsError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(OpsError::Watch(WatchError::NoRuntime));
        }
        let hub = self.own_hub();
        let mut seen = HashSet::new();
        let named: Vec<Point> = points
            .into_iter()
            .map(|point| hub.keyed(point))
            .filter(|point| seen.insert(point.clone()))
            .collect();
        let leases = hub.watch_all(named).map_err(OpsError::Watch)?;
        let mailbox = Arc::new(Mailbox {
            state: Mutex::new(MailState {
                queue: VecDeque::with_capacity(leases.len()),
                queued: vec![false; leases.len()],
                folded: 0,
                waker: None,
            }),
        });
        for (index, lease) in leases.iter().enumerate() {
            lease.listen(Listener {
                mailbox: Arc::downgrade(&mailbox),
                index,
            });
        }
        Ok(Watch { leases, mailbox })
    }

    /// The range of subscriber process identifiers for another hub made on
    /// this client: the next one, never the client's own hub's, which it
    /// keeps for the client's life however the ranges wrap.
    pub(crate) fn hub_range(&self) -> u32 {
        let pinned = lock(&self.shared.hub).range;
        loop {
            let range = self.next_hub();
            if pinned.is_none_or(|pinned| pinned % PROCESS_RANGES != range % PROCESS_RANGES) {
                return range;
            }
        }
    }

    /// This client's own hub: the one its watches share while any holds a
    /// point, or a new one in the client's own range, over the client's
    /// base settings.
    fn own_hub(&self) -> Hub<T> {
        let mut own = lock(&self.shared.hub);
        if let Some(inner) = own.hub.upgrade() {
            return Hub { inner };
        }
        let range = *own.range.get_or_insert_with(|| self.next_hub());
        let hub = Hub::in_range(self.base(), settings(), range)
            .expect("the client hub's settings are valid");
        own.hub = Arc::downgrade(&hub.inner);
        hub
    }
}

impl From<WatchError> for OpsError {
    fn from(error: WatchError) -> Self {
        Self::Watch(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_hub_settings_are_valid() {
        assert!(settings().validate().is_ok());
    }
}
