//! What a [`Watch`](super::Watch) and its [`WatchControl`]s share (#2603):
//! the watch's points, each with its lease on the hub, and its mailbox of
//! the points changed since the watch last took them.
//!
//! One std mutex guards it all, held for a few field updates, never across
//! an await, a call into the hub or a lease's drop (which takes the hub's
//! table): it comes last in the hub's lock order, after an entry's lock
//! ([`shared`](super::shared)). Each point has a place (an index into
//! `items`) and a generation, bumped when it is removed, so a listener of
//! a removed point, or of one an index held before, posts nothing.
//!
//! **Removal.** [`WatchControl::remove`] takes a point's lease out, bumps
//! its generation and clears its mark from the mailbox, all under the
//! lock; [`Watch::next`](super::Watch::next) takes a mark under the lock,
//! builds the update without it, then checks the point's place and
//! generation under it again before handing the update out. So a `next`
//! begun after `remove` returned never yields the point. A `next` already
//! past that last check when the removal takes the lock still returns its
//! update, which may be after `remove` returned: that update was handed
//! out before the removal took effect.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::Waker;
use std::time::Duration;

use bacnet_types::sync::MutexExt;

use super::hub::{Lease, Phase};
use super::shared::Shared;
use super::stream::Mark;
use super::{Point, WatchError};

/// What a watch needs of its hub, whatever its transport: the keyed form
/// of a point, and leases on new ones.
pub(super) trait Port: Send + Sync {
    /// `point` as the hub keys it.
    fn keyed(&self, point: Point) -> Point;
    /// A lease on each of `points`, keyed and distinct, or none.
    fn watch_all(&self, points: Vec<Point>) -> Result<Vec<Lease>, WatchError>;
}

/// A watch's points and mailbox, shared with its controls.
pub(super) struct Core {
    state: Mutex<CoreState>,
}

struct CoreState {
    /// Each point's place; a removed one's is free for the next add.
    items: Vec<Item>,
    /// The places free.
    free: Vec<usize>,
    /// Where each point watched is.
    places: HashMap<Point, usize>,
    /// The places changed since the watch took them, each once, in order.
    queue: VecDeque<usize>,
    folded: u64,
    waker: Option<Waker>,
    /// [`WatchControl`]s alive: a watch of no points waits while one is.
    controls: usize,
    /// Set once the watch has ended: dropped, closed, its stream ended, or
    /// its hub shut down. It stays so.
    ended: bool,
    /// The hub, while the watch goes on: a pooled hub lives as long as a
    /// watch on it does.
    hub: Option<Arc<dyn Port>>,
}

struct Item {
    lease: Option<Lease>,
    generation: u64,
    queued: bool,
}

/// What an ended watch let go of, dropped by the caller once the lock is
/// let go: leases take the hub's table lock as they drop.
pub(super) struct Left {
    pub(super) leases: Vec<Lease>,
    pub(super) waker: Option<Waker>,
    _hub: Option<Arc<dyn Port>>,
}

impl Core {
    /// A watch on `hub` of no points yet.
    pub(super) fn new(hub: Arc<dyn Port>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(CoreState {
                items: Vec::new(),
                free: Vec::new(),
                places: HashMap::new(),
                queue: VecDeque::new(),
                folded: 0,
                waker: None,
                controls: 0,
                ended: false,
                hub: Some(hub),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, CoreState> {
        self.state.locked()
    }

    /// Take `leases` in, each a point not held yet; one the watch holds
    /// already (a racing add took it) goes back in `extra`. With each
    /// taken: its entry, and the listener to register on it once the lock
    /// is let go. `Err(leases)` once the watch has ended.
    pub(super) fn take_in(self: &Arc<Self>, leases: Vec<Lease>) -> Result<TakenIn, Vec<Lease>> {
        let mut state = self.lock();
        if state.ended {
            return Err(leases);
        }
        let (mut taken, mut extra) = (Vec::new(), Vec::new());
        for lease in leases {
            let point = lease.shared.point.clone();
            if state.places.contains_key(&point) {
                extra.push(lease);
                continue;
            }
            let shared = Arc::clone(&lease.shared);
            let index = match state.free.pop() {
                Some(index) => index,
                None => {
                    state.items.push(Item {
                        lease: None,
                        generation: 0,
                        queued: false,
                    });
                    state.items.len() - 1
                }
            };
            let item = &mut state.items[index];
            item.lease = Some(lease);
            let generation = item.generation;
            state.places.insert(point, index);
            let listener = Listener {
                core: Arc::downgrade(self),
                index,
                generation,
            };
            taken.push((shared, listener));
        }
        Ok((taken, extra))
    }
}

/// What [`Core::take_in`] took: each point's entry with the listener to
/// register on it, and the leases of points held already, to drop.
pub(super) type TakenIn = (Vec<(Arc<Shared>, Listener)>, Vec<Lease>);

impl Core {
    /// The hub, while the watch goes on.
    fn hub(&self) -> Result<Arc<dyn Port>, WatchError> {
        let state = self.lock();
        match (&state.hub, state.ended) {
            (Some(hub), false) => Ok(Arc::clone(hub)),
            _ => Err(WatchError::Closed),
        }
    }

    /// Whether the watch holds `point`, keyed.
    fn has_point(&self, point: &Point) -> bool {
        self.lock().places.contains_key(point)
    }

    /// End the watch: what it held, for the caller to drop.
    pub(super) fn end(&self) -> Left {
        let mut state = self.lock();
        state.ended = true;
        state.places.clear();
        state.queue.clear();
        state.free.clear();
        let leases = state
            .items
            .drain(..)
            .filter_map(|item| item.lease)
            .collect();
        Left {
            leases,
            waker: state.waker.take(),
            _hub: state.hub.take(),
        }
    }

    /// The next update's place, generation and entry; else whether the
    /// watch has ended, or waits (`waker` kept for the next post).
    pub(super) fn take(&self, waker: &Waker) -> Taken {
        let mut state = self.lock();
        if state.ended {
            return Taken::Ended;
        }
        if state.places.is_empty() && state.controls == 0 {
            // No point, and nothing can add one: the stream ends.
            state.ended = true;
            let hub = state.hub.take();
            drop(state);
            drop(hub);
            return Taken::Ended;
        }
        loop {
            let Some(index) = state.queue.pop_front() else {
                state.waker = Some(waker.clone());
                return Taken::Wait;
            };
            let item = &mut state.items[index];
            item.queued = false;
            if let Some(lease) = &item.lease {
                return Taken::Point(index, item.generation, Arc::clone(&lease.shared));
            }
        }
    }

    /// Whether the point `mark` names is still in its place, not removed
    /// since, and the watch goes on: an update taken for it may be handed
    /// out.
    pub(super) fn holds(&self, mark: Mark) -> bool {
        let state = self.lock();
        !state.ended
            && state
                .items
                .get(mark.index)
                .is_some_and(|item| item.generation == mark.generation && item.lease.is_some())
    }

    /// The points, in the order of their places.
    pub(super) fn points(&self) -> Vec<Point> {
        self.lock()
            .items
            .iter()
            .filter_map(|item| item.lease.as_ref())
            .map(|lease| lease.shared.point.clone())
            .collect()
    }

    pub(super) fn folded(&self) -> u64 {
        self.lock().folded
    }

    /// A control is made.
    pub(super) fn control(self: &Arc<Self>) -> WatchControl {
        self.lock().controls += 1;
        WatchControl {
            core: Arc::clone(self),
        }
    }
}

/// What [`Core::take`] found.
pub(super) enum Taken {
    Ended,
    Wait,
    Point(usize, u64, Arc<Shared>),
}

/// One watch's place among a point's listeners: its core, and the point's
/// place and generation in that watch.
pub(crate) struct Listener {
    core: Weak<Core>,
    index: usize,
    generation: u64,
}

impl Listener {
    /// Mark the point changed in the watch, adding the watch's waker to
    /// `wakers` for the caller to wake once its lock is let go; false once
    /// the watch is gone or no longer holds the point.
    pub(super) fn post(&self, wakers: &mut Vec<Waker>) -> bool {
        let Some(core) = self.core.upgrade() else {
            return false;
        };
        let mut state = core.lock();
        let state = &mut *state;
        let Some(item) = state
            .items
            .get_mut(self.index)
            .filter(|item| item.generation == self.generation && item.lease.is_some())
        else {
            return false;
        };
        if item.queued {
            state.folded += 1;
        } else {
            item.queued = true;
            state.queue.push_back(self.index);
            wakers.extend(state.waker.take());
        }
        true
    }

    /// Whether its watch may still be alive.
    pub(super) fn alive(&self) -> bool {
        self.core.strong_count() > 0
    }
}

/// A handle that adds points to a live [`Watch`](super::Watch) and removes
/// them, from any task, while another awaits the watch's next update
/// ([`Watch::control`](super::Watch::control)). Cheap to clone; clones
/// control one watch. A watch of no points waits for an add while a
/// control lives, and ends once none does.
///
/// Once the watch has ended (dropped, [`close`](Self::close)d, its stream
/// ended, or its hub shut down) every call is refused with
/// [`WatchError::Closed`].
pub struct WatchControl {
    core: Arc<Core>,
}

impl Clone for WatchControl {
    fn clone(&self) -> Self {
        self.core.control()
    }
}

impl Drop for WatchControl {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.core.lock();
            state.controls -= 1;
            match state.controls == 0 && state.places.is_empty() {
                true => state.waker.take(),
                false => None,
            }
        };
        // The watch's wait for an add ends: nothing can add one now.
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl std::fmt::Debug for WatchControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchControl").finish_non_exhaustive()
    }
}

impl WatchControl {
    /// Watch `points` too: those the watch doesn't hold yet, each once,
    /// as its hub keys them. All or none: the count newly watched, 0 when
    /// it holds every one already (nothing changes then).
    ///
    /// A point no other watch on the hub holds starts as at any watch: read
    /// first, then subscribed, and its first update comes from that read
    /// (or its first change of state). One another watch holds joins its
    /// subscription, and its first update, at once, is where the point
    /// stands. A point removed and added again is a new point of the
    /// watch: unless another watch held it meanwhile, it is a new
    /// subscription, with a subscriber process identifier of its own, and
    /// nothing of the old one reaches it. Every change after `add` returns
    /// comes as an update; none comes from before the add.
    ///
    /// # Errors
    ///
    /// [`WatchError::Full`] when the points would take the hub past the
    /// most it watches at once, cancelling ones included ([`MAX_WATCHED`]
    /// for a client's pooled hubs); nothing is watched then, and nothing
    /// sent. [`WatchError::Closed`] once the watch has ended,
    /// [`WatchError::Shut`] while its hub shuts down, and
    /// [`WatchError::NoRuntime`] outside a Tokio runtime.
    ///
    /// [`MAX_WATCHED`]: super::MAX_WATCHED
    pub fn add(&self, points: impl IntoIterator<Item = Point>) -> Result<usize, WatchError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(WatchError::NoRuntime);
        }
        let hub = self.core.hub()?;
        let mut seen = std::collections::HashSet::new();
        let new: Vec<Point> = points
            .into_iter()
            .map(|point| hub.keyed(point))
            .filter(|point| !self.core.has_point(point) && seen.insert(point.clone()))
            .collect();
        if new.is_empty() {
            return Ok(0);
        }
        let leases = hub.watch_all(new)?;
        drop(hub);
        let (taken, extra) = self.core.take_in(leases).map_err(|leases| {
            drop(leases);
            WatchError::Closed
        })?;
        drop(extra);
        let added = taken.len();
        for (shared, listener) in taken {
            super::stream::listen(&shared, listener);
        }
        Ok(added)
    }

    /// Stop watching `points` in this watch: the count it held. When this
    /// returns, no `next` begun after yields any of them, a change already
    /// waiting included. A `next` that had already passed its last check
    /// when the removal took effect still returns that update, possibly
    /// after this returns: it was handed out before the removal. A point no other watch on the hub
    /// holds stops there: its subscription is cancelled on the wire by the
    /// hub's scheduler, after this returns and without delaying it, best
    /// effort as when a watch is dropped; a point another watch holds goes
    /// on for that one. Removing a point not held does nothing.
    ///
    /// # Errors
    ///
    /// [`WatchError::Closed`] once the watch has ended.
    pub fn remove<'a>(
        &self,
        points: impl IntoIterator<Item = &'a Point>,
    ) -> Result<usize, WatchError> {
        let hub = self.core.hub()?;
        let keyed: Vec<Point> = points.into_iter().map(|p| hub.keyed(p.clone())).collect();
        drop(hub);
        let gone: Vec<Lease> = {
            let mut state = self.core.lock();
            if state.ended {
                return Err(WatchError::Closed);
            }
            let state = &mut *state;
            let mut gone = Vec::new();
            for point in keyed {
                let Some(index) = state.places.remove(&point) else {
                    continue;
                };
                let item = &mut state.items[index];
                gone.extend(item.lease.take());
                item.generation += 1;
                if std::mem::take(&mut item.queued) {
                    state.queue.retain(|queued| *queued != index);
                }
                state.free.push(index);
            }
            gone
        };
        // Dropped once the lock is let go: the last lease on a point stops
        // its entry under the hub's table lock.
        Ok(gone.len())
    }

    /// Whether the watch still holds the point `mark` names in the place it
    /// had when an update was taken for it ([`Update::mark`]): false once
    /// that point was removed, even if added again, and once the watch has
    /// ended. A consumer that holds updates back drops those this refuses
    /// after a [`remove`](Self::remove), so none gets out after it returned.
    ///
    /// [`Update::mark`]: super::Update::mark
    pub fn holds(&self, mark: Mark) -> bool {
        self.core.holds(mark)
    }

    /// The points watched now, in the order of their places: as named,
    /// an added point taking a removed one's place if one is free.
    pub fn points(&self) -> Vec<Point> {
        self.core.points()
    }

    /// End the watch: its stream ends at once (`next` gives `None`), every
    /// point is let go, and this waits up to `wait` for the points no other
    /// watch holds to finish, their cancels answered or given up: true when
    /// all did in time. Those still cancelling after go on without the
    /// wait. Every later call on the watch's controls is refused with
    /// [`WatchError::Closed`]; closing again does nothing and gives true.
    pub async fn close(&self, wait: Duration) -> bool {
        let left = self.core.end();
        if let Some(waker) = left.waker {
            waker.wake();
        }
        let stopping: Vec<_> = left
            .leases
            .iter()
            .map(|lease| (Arc::clone(&lease.shared), lease.shared.signal.subscribe()))
            .collect();
        drop(left.leases);
        let finished = async {
            for (shared, mut signal) in stopping {
                // Held by another watch: it goes on.
                if !shared.is_stopped() {
                    continue;
                }
                while shared.lock().snapshot.state != Phase::Stopped {
                    if signal.changed().await.is_err() {
                        break;
                    }
                }
            }
        };
        tokio::time::timeout(wait, finished).await.is_ok()
    }
}
