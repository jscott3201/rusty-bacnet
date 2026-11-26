//! [`Watch`]: an application's watch of many points, a stream of
//! [`Update`]s ([`Client::watch`](crate::Client::watch)).
//!
//! A watch holds one lease a point on its hub (a client's pooled hub, or
//! one a front end runs). Each signalled change of a point marks it in the
//! watch's mailbox ([`control`](super::control)), at most once however
//! many changes come before the watch takes it, and wakes the watch once
//! the point's lock is let go; the update is built from the point's latest
//! snapshot when the watch takes it. So a slow consumer gets the latest
//! value of each point, never a queue of old ones, and the mailbox never
//! holds more than one mark a point; the hub never waits for a watch, so a
//! slow one holds up no other.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, Waker};
use std::time::SystemTime;

use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::status_flags_from_bacnet;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{PropertyValue, StatusFlags};
use bacnet_types::sync::MutexExt;
use futures_core::Stream;

use super::control::{Core, Listener, Port, Taken, WatchControl};
use super::hub::{self, Hub, Lease, Snapshot, Source};
use super::sample::Via;
use super::shared::Shared;
use super::support::CovSupport;
use super::{Moved, Point, WatchError, Why};
use crate::device::DeviceAddress;
use crate::format::{Floats, PropertyError, RawRow, StateTexts, Value, ValueFormat, units};

/// The most points one of a client's pooled hubs watches at once, every
/// watch on it together, those still cancelling included.
pub const MAX_WATCHED: usize = 4096;

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
    /// Polled, with the device's other polled points, every `poll_every`
    /// at most ([`WatchSettings`](super::WatchSettings)), because of `why`:
    ///
    /// - [`Why::Refused`]: the device said no (or refused the service for
    ///   an earlier point); not asked again.
    /// - [`Why::PollOnly`]: the watch's settings poll every point.
    /// - [`Why::ForeignSender`]: its notifications name a device that
    ///   can't be told from another at its address; subscribed again when
    ///   the point starts again.
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
    /// remote address is known. A following point
    /// ([`Binding::Follows`](super::Binding::Follows)) whose address another
    /// device answers at now is unreachable for [`Why::Replaced`] while the
    /// hub looks for its device (#2795).
    Unreachable {
        /// Why it isn't subscribed.
        why: Why,
    },
    /// Given up: nothing more is sent for it, and nothing its address
    /// sends is taken as its own. [`Why::Moved`]: a pinned point's device
    /// answers at another address now, so watch the point there (or follow
    /// its instance); [`Why::Replaced`]: another device answers at a pinned
    /// point's address now. Either way the point starts again if its
    /// address answers for its instance again. [`Why::Missing`]: no route
    /// to the address is known, and the hub's settings say not to wait. A
    /// following point is never given up for a move or for another device
    /// at its address ([`Update::moved`]).
    Gone {
        /// Why it was given up.
        why: Why,
    },
}

impl State {
    /// The application's state of a point the hub reports as `snapshot`.
    /// A stopped point is never handed out: its hub shut down, which ends
    /// the watch first, or no watch holds it.
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
    /// The point, as the watch holds it ([`Watch::points`]): the key a
    /// caller's map of points keeps across a followed device's move. It is
    /// the point as named, but for an address on the client's own network,
    /// which is the local station once the client knows the network's
    /// number, and a following point named on that network, which is
    /// network 0 then.
    pub point: Point,
    /// Where the point is watched now: a pinned point's address, or where
    /// the hub follows a following point's device (#2795).
    pub at: DeviceAddress,
    /// The device's instance, when known: a following point's own, else
    /// the one the device at the address named, or the discovery table's
    /// row there.
    pub instance: Option<u32>,
    /// The latest move of a following point's device the hub took: it
    /// watches the point at `moved.to` (`at`) since `moved.when`, and
    /// nothing from `moved.from` is the point's. The update a move makes
    /// is [`State::Starting`], with the values read before; values from the
    /// new address follow. A consumer that falls behind sees the latest
    /// move only: compare its `count` to tell a new one. `None` for a pinned
    /// point, and before any move.
    pub moved: Option<Moved>,
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
    /// The point's place in its watch when the update was taken.
    mark: Mark,
}

/// Which of its watch's points an [`Update`] was taken for: the point's
/// place in the watch, and that place's generation, which a removal moves
/// on ([`Update::mark`]). A consumer that holds updates back (a buffer, a
/// slot for one a cancelled read put back) asks
/// [`WatchControl::holds`] before handing one out: false once its point
/// was removed, even if added again, so nothing of a removed point gets
/// out after the removal returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mark {
    pub(super) index: usize,
    pub(super) generation: u64,
}

impl Update {
    /// Which of its watch's points this update was taken for.
    pub fn mark(&self) -> Mark {
        self.mark
    }
}

/// A watch of many points: a [`Stream`] of their [`Update`]s.
///
/// Made by [`Client::watch`](crate::Client::watch) (or
/// [`watch_with`](crate::Client::watch_with), or
/// [`Device::watch`](crate::Device::watch)). Each point is watched once
/// however many watches on the hub hold it: one COV subscription, renewed
/// before its lifetime ends, or, where the device refuses COV, a poll that
/// reads all of a device's polled points together.
///
/// **Changing the set.** [`control`](Self::control) gives a
/// [`WatchControl`] that adds and removes points from any task while
/// another awaits [`next`](Self::next).
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
/// it. [`WatchControl::close`] does the same and waits for the cancels.
///
/// **Ending.** The stream ends (`None`, and `None` from then on) once the
/// watch has no point and no control lives to add one, once it is closed,
/// and at once when its hub shuts down
/// ([`Client::shutdown_watches`](crate::Client::shutdown_watches),
/// [`Hub::shutdown`]): updates still waiting then are dropped, since the
/// hub watches nothing more. Otherwise it never ends on its own: a point
/// given up ([`State::Gone`]) sends its last update and stays, taking
/// nothing more from its address while its old subscription lapses, and
/// starts again if its device comes back there.
pub struct Watch {
    core: Arc<Core>,
}

impl Watch {
    /// The points watched, in the order of their places: as named, each
    /// once, with its device's address as the hub keys it; an added point
    /// takes a removed one's place when one is free.
    pub fn points(&self) -> Vec<Point> {
        self.core.points()
    }

    /// Changes folded into a later update of the same point because this
    /// watch hadn't taken the earlier one yet.
    pub fn folded(&self) -> u64 {
        self.core.folded()
    }

    /// A handle that adds points to this watch and removes them while
    /// another task awaits [`next`](Self::next).
    pub fn control(&self) -> WatchControl {
        self.core.control()
    }

    /// The next update, as the [`Stream`] gives it, without a stream
    /// adapter: `None` once the watch has ended (see the type's docs),
    /// else an update whenever one comes. Cancel safe: an update leaves the
    /// mailbox only on the poll that returns it, so a dropped call loses
    /// none.
    pub async fn next(&mut self) -> Option<Update> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // The leases go once the core's lock is let go.
        drop(self.core.end());
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch")
            .field("points", &self.core.points().len())
            .finish()
    }
}

impl Stream for Watch {
    type Item = Update;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Update>> {
        let core = &self.get_mut().core;
        loop {
            let (index, generation, shared) = match core.take(cx.waker()) {
                Taken::Ended => return Poll::Ready(None),
                Taken::Wait => return Poll::Pending,
                Taken::Point(index, generation, shared) => (index, generation, shared),
            };
            // Built outside the core's lock (it takes the entry's), then
            // handed out only if the point is still this watch's: a removal
            // meanwhile drops it.
            let mark = Mark { index, generation };
            let update = update(&shared, mark);
            if core.holds(mark) {
                return Poll::Ready(Some(update));
            }
        }
    }
}

/// Tell `listener` of every change of `shared` signalled from now on, and
/// of where the point stands now if it has changed before.
pub(super) fn listen(shared: &Shared, listener: Listener) {
    let mut state = shared.lock();
    state.listeners.retain(Listener::alive);
    let mut wakers = Vec::new();
    if state.snapshot.seq > 0 {
        listener.post(&mut wakers);
    }
    state.listeners.push(listener);
    drop(state);
    wakers.into_iter().for_each(Waker::wake);
}

/// `shared`'s point as it stands: its value from the latest rows, named by
/// the units and state texts among them.
fn update(shared: &Shared, mark: Mark) -> Update {
    let point = shared.point.clone();
    let (property, _) = point.property();
    let ((rows, via, state, cov, failure, updated), (at, instance, moved)) = {
        let entry = shared.lock();
        let snapshot = &entry.snapshot;
        (
            (
                snapshot.rows.clone(),
                snapshot.via,
                State::of(snapshot),
                snapshot.cov,
                entry.failure.clone(),
                entry.wall,
            ),
            (entry.at.clone(), entry.identity, snapshot.moved.clone()),
        )
    };
    let (value, error) = match rows.iter().find(|(p, _)| *p == property) {
        Some((_, Ok(raw))) => (Some(value(&point, &rows, raw.clone())), failure),
        Some((_, Err(error))) => (None, Some(error.clone())),
        None => (None, failure),
    };
    Update {
        point,
        at,
        instance,
        moved,
        state,
        value,
        error,
        status_flags: status_flags(&rows),
        via,
        updated,
        cov,
        mark,
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

impl<T: TransportPort + 'static> Port for Hub<T> {
    fn keyed(&self, point: Point) -> Point {
        Hub::keyed(self, point)
    }

    fn watch_all(&self, points: Vec<Point>) -> Result<Vec<Lease>, WatchError> {
        Hub::watch_all(self, points)
    }
}

impl<T: TransportPort + 'static> Hub<T> {
    /// Watch `points` on this hub: a [`Watch`] streaming each point's
    /// [`Update`]s, as [`Client::watch`](crate::Client::watch) gives on a
    /// client's pooled hub, for a front end that runs a hub of its own
    /// settings. A point named twice, or one station named two ways, is
    /// watched once. The watch holds the hub while it lives; the hub's
    /// [`shutdown`](Self::shutdown) ends it.
    ///
    /// # Errors
    ///
    /// [`WatchError::Full`] when the points would take the hub past
    /// [`WatchOptions::max_points`](hub::WatchOptions::max_points)
    /// (nothing is watched then, and nothing sent), and
    /// [`WatchError::Shut`] once the hub has shut down.
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime, when a point's device has no scheduler
    /// yet: the scheduler is a task.
    pub fn stream(&self, points: impl IntoIterator<Item = Point>) -> Result<Watch, WatchError> {
        let mut seen = HashSet::new();
        let named: Vec<Point> = points
            .into_iter()
            .map(|point| self.keyed(point))
            .filter(|point| seen.insert(point.clone()))
            .collect();
        let leases = self.watch_all(named)?;
        let port: Arc<dyn Port> = Arc::new(self.clone());
        let core = Core::new(port);
        let Ok((taken, _)) = core.take_in(leases) else {
            unreachable!("a new watch hasn't ended");
        };
        for (shared, listener) in taken {
            listen(&shared, listener);
        }
        let registry = &self.inner.registry;
        {
            let mut watches = registry.watches.locked();
            watches.retain(|watch| watch.strong_count() > 0);
            watches.push(Arc::downgrade(&core));
        }
        // A shutdown that took the list before this watch joined it: the
        // watch ends here instead.
        if registry.table().shut {
            end(&core);
        }
        Ok(Watch { core })
    }
}

/// End `core`'s watch, its hub having shut down: its stream ends, and what
/// it held goes once its lock is let go.
pub(super) fn end(core: &Core) {
    let left = core.end();
    if let Some(waker) = left.waker {
        waker.wake();
    }
}

/// End every watch in `watches`: their hub has shut down.
pub(super) fn end_all(watches: Vec<Weak<Core>>) {
    for core in watches.iter().filter_map(Weak::upgrade) {
        end(&core);
    }
}

#[cfg(test)]
mod tests {
    use crate::watch::WatchSettings;

    #[test]
    fn the_default_settings_are_valid() {
        assert!(WatchSettings::default().validate().is_ok());
    }
}
