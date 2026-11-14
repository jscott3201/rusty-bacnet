//! [`Watch`]: an application's watch of many points, a stream of
//! [`Update`]s ([`Client::watch`]).
//!
//! A watch holds one lease a point on its client's own hub, made on first
//! use and kept while any watch holds a point. Each signalled change of a
//! point marks it in the watch's mailbox, at most once however many
//! changes come before the watch takes it, and wakes the watch; the
//! update is built from the point's latest snapshot when the watch takes
//! it. So a slow consumer gets the latest value of each point, never a
//! queue of old ones, and the mailbox never holds more than one mark a
//! point; the hub never waits for a watch, so a slow one holds up no
//! other.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use futures_core::Stream;

use super::sample::Via;
use super::support::CovSupport;
use super::{
    Hub, Lease, MissingDevice, Point, Reads, Source, State, WatchError, WatchOptions, Why,
};
use crate::client::{Client, lock};
use crate::error::OpsError;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat, units};

/// The most points a client's own hub watches at once, every watch on the
/// client together.
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

/// One change of a watched point: its latest value and how its watch goes.
///
/// An update comes with each value read or notified (the first read, each
/// COV notification, each poll) and each change of the watch's state (the
/// point is subscribed, polled, unreachable, given up). A watch whose
/// consumer falls behind folds a point's changes into one update with the
/// latest of them.
#[derive(Debug, Clone)]
pub struct Update {
    /// The point.
    pub point: Point,
    /// The property's value as last read or notified (an object watch's
    /// Present_Value), its text naming the object's units or state when
    /// the point's first read had them; kept while the device is
    /// unreachable, so it can be old (see [`state`](Self::state)). `None`
    /// before the first value, and while the device answers the property
    /// with an error.
    pub value: Option<Value>,
    /// Why the latest read gave no value: the device's error for the
    /// property, or, with [`State::Failed`], for the whole object, or its
    /// answer leaving the object out.
    pub error: Option<PropertyError>,
    /// How the latest values came: the first read, a COV notification or
    /// a poll.
    pub via: Option<Via>,
    /// Where values come from now: COV while subscribed, polls otherwise;
    /// `None` before the first read ends.
    pub source: Option<Source>,
    /// How the watch goes.
    pub state: State,
    /// Why the point is polled, unreachable or given up: COV refused or
    /// lapsed, no route, or the device moved ([`Why::Moved`]: watch it at
    /// its new address). `None` while subscribed.
    pub why: Option<Why>,
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
/// other watch holds stops: a subscribed one is cancelled on the wire
/// (one SubscribeCOV or SubscribeCOVProperty without a lifetime, which the
/// device takes as a cancellation, sent through the device's gate); if the
/// device doesn't answer the cancel within 30 s it is given up and the
/// subscription ends with its lifetime. A polled point is no longer
/// polled, and nothing is sent for it.
///
/// The stream ends at once for a watch of no points, and otherwise only
/// when its client's hub stops, which it doesn't while a watch holds a
/// point. A point given up ([`State::Gone`]) sends its last update and
/// nothing after.
pub struct Watch {
    leases: Vec<Lease>,
    mailbox: Arc<Mailbox>,
}

impl Watch {
    /// The points watched, in the order named, each once.
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
    /// Mark the point changed in the watch and wake it; false once the
    /// watch is gone.
    pub(super) fn post(&self) -> bool {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return false;
        };
        let waker = {
            let mut state = lock(&mailbox.state);
            if state.queued[self.index] {
                state.folded += 1;
                None
            } else {
                state.queued[self.index] = true;
                state.queue.push_back(self.index);
                state.waker.take()
            }
        };
        if let Some(waker) = waker {
            waker.wake();
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
        if state.snapshot.seq > 0 {
            listener.post();
        }
        state.listeners.push(listener);
    }
}

/// `lease`'s point as it stands: its value from the latest rows, named by
/// the units and state texts among them.
fn update(lease: &Lease) -> Update {
    let point = lease.shared.point.clone();
    let (property, _) = point.property();
    let (rows, via, source, state, why, cov, failure) = {
        let entry = lease.shared.lock();
        let snapshot = &entry.snapshot;
        (
            snapshot.rows.clone(),
            snapshot.via,
            snapshot.source,
            snapshot.state,
            snapshot.why.clone(),
            snapshot.cov,
            entry.failure.clone(),
        )
    };
    let (value, error) = match rows.iter().find(|(p, _)| *p == property) {
        Some((_, Ok(raw))) => (Some(value(&point, &rows, raw.clone())), failure),
        Some((_, Err(error))) => (None, Some(error.clone())),
        None => (None, failure),
    };
    Update {
        point,
        value,
        error,
        via,
        source,
        state,
        why,
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
    /// asking. A point named twice is watched once.
    ///
    /// # Errors
    ///
    /// [`OpsError::Watch`] when the client's hub watches
    /// [`MAX_WATCHED`] points already; nothing is watched then.
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime, as `tokio::spawn` does: the hub starts its
    /// tasks here on first use.
    pub fn watch(&self, points: impl IntoIterator<Item = Point>) -> Result<Watch, OpsError> {
        let mut named: Vec<Point> = Vec::new();
        for point in points {
            if !named.contains(&point) {
                named.push(point);
            }
        }
        let hub = self.own_hub();
        let mailbox = Arc::new(Mailbox {
            state: Mutex::new(MailState {
                queue: VecDeque::with_capacity(named.len()),
                queued: vec![false; named.len()],
                folded: 0,
                waker: None,
            }),
        });
        let mut leases = Vec::with_capacity(named.len());
        for (index, point) in named.into_iter().enumerate() {
            // A refusal drops the leases taken so far, which lets them go.
            let lease = hub.watch(point).map_err(OpsError::Watch)?;
            lease.listen(Listener {
                mailbox: Arc::downgrade(&mailbox),
                index,
            });
            leases.push(lease);
        }
        Ok(Watch { leases, mailbox })
    }

    /// This client's own hub: the one its watches share while any holds a
    /// point, or a new one.
    fn own_hub(&self) -> Hub<T> {
        let mut own = lock(&self.shared.hub);
        if let Some(inner) = own.upgrade() {
            return Hub { inner };
        }
        let hub = Hub::new(self.clone(), settings()).expect("the client hub's settings are valid");
        *own = Arc::downgrade(&hub.inner);
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
