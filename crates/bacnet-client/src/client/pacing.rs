//! A minimum interval between confirmed requests to one destination (#1535).
//!
//! A slow device answers its other clients late while one client reads it
//! back to back, even with one request outstanding. With an interval set, a
//! confirmed request to a destination goes only once the interval has passed
//! since the latest request sent to it finished, by a reply, an error or its
//! caller giving up; while that one is still outstanding, since it was sent.
//! A request that has to wait sleeps until then and checks again, so a reply
//! that comes late still gets the whole pause. Requests waiting together go
//! one at a time, the interval apart, in no particular order. Destinations
//! don't wait on each other, and an interval of zero changes nothing. A
//! request still waiting when the client's tasks end, by its stop or its
//! node's, fails then and takes no turn (#2720).
//!
//! Pacing runs before a routed request takes its path lease. That lease is
//! shared by every device on the same network behind the same router, so
//! there a request waiting for the lease goes as soon as the lease frees: the
//! pause after a reply holds for requests made one after another, not for
//! concurrent ones.
//!
//! The endpoint client paces through this same type (#1542), before it
//! reserves an invoke ID, so a request waiting for its turn holds nothing
//! from the device's shared pool. On a node that audits the client's
//! requests, the node takes the turn ([`Turn`]) as it admits the request
//! (#2652): it waits for the turn to come due holding nothing, then for its
//! admission, and then takes the turn if it is still due, or lets the
//! admission go and waits again. So a request that waited for admission
//! still gets the whole interval after the one before it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use bacnet_types::MacAddr;
use bacnet_types::error::{Error, Refusal};
use bacnet_types::sync::MutexExt;
use tokio::sync::watch;
use tokio::time::{Duration, Instant};

use super::ConfirmedTarget;

/// Most destinations the pacer remembers at once. Past it, an idle one goes
/// first (its latest request finished), the one free soonest of those; only
/// when every one has a request outstanding does the one free soonest of all.
const MAX_PACED_DESTINATIONS: usize = 4_096;

/// The longest interval a client takes: an hour.
pub const MAX_MIN_REQUEST_INTERVAL_MS: u64 = 3_600_000;

/// Refuse an interval past [`MAX_MIN_REQUEST_INTERVAL_MS`].
pub(crate) fn validate_interval_ms(interval_ms: u64) -> Result<(), Error> {
    if interval_ms > MAX_MIN_REQUEST_INTERVAL_MS {
        return Err(Error::InvalidConfig(
            Refusal::new("invalid min-request-interval {} ms; expected 0..={}")
                .with_values(interval_ms, MAX_MIN_REQUEST_INTERVAL_MS),
        ));
    }
    Ok(())
}

/// The device a confirmed request goes to: its network, `None` for this
/// one, and its MAC.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PaceKey {
    network: Option<u16>,
    mac: MacAddr,
}

impl PaceKey {
    /// The device at `mac` on `network`, `None` for this one. Name a station
    /// on this network as a local one, as each client does once it has
    /// localized the destination, so both spellings share one lane.
    pub(crate) fn new(network: Option<u16>, mac: &[u8]) -> Self {
        Self {
            network,
            mac: MacAddr::from_slice(mac),
        }
    }

    pub(super) fn of(target: ConfirmedTarget<'_>) -> Self {
        match target {
            ConfirmedTarget::Local { mac } => Self::new(None, mac),
            ConfirmedTarget::Routed {
                dest_network,
                dest_mac,
                ..
            } => Self::new(Some(dest_network), dest_mac),
        }
    }
}

/// One destination: the latest request sent to it. A lane exists only once
/// a request has gone; waiting leaves no state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lane {
    /// Which lane this is, so a guard from a lane that was forgotten and
    /// made again touches nothing.
    generation: u64,
    /// Which request in this lane went last; counts from 1 in each lane.
    ticket: u64,
    sent: Instant,
    /// When that request finished; `None` while it is outstanding.
    finished: Option<Instant>,
}

impl Lane {
    /// When the next request to this destination may go.
    fn due(&self, interval: Duration) -> Instant {
        self.finished.unwrap_or(self.sent) + interval
    }

    fn idle(&self) -> bool {
        self.finished.is_some()
    }
}

#[derive(Debug, Default)]
struct Lanes {
    by_destination: HashMap<PaceKey, Lane>,
    next_generation: u64,
}

/// Spaces the confirmed requests to each destination by a fixed interval.
#[derive(Debug)]
pub(crate) struct RequestPacer {
    interval: Duration,
    capacity: usize,
    lanes: Mutex<Lanes>,
    /// The client's part in its node's source Audit (#2617), set once on a
    /// seated client whose node audits it and on each copy that runs one of
    /// its requests. It lives here, in the allocation each client has to
    /// itself and every request passes first, so a client is no larger for
    /// it; the request path reads it only where a registration is refused.
    pub(super) audit: OnceLock<super::source_audit::Role>,
}

/// Held for the life of a request that went; dropping it, when the request
/// finishes or its caller gives up, starts the interval for the next. It
/// keeps its pacer alive, so a request handed to a task of its own, as an
/// audited endpoint request is, takes its guard along.
#[must_use = "the request counts as finished when the guard drops"]
pub(crate) struct PaceGuard {
    /// The pacer, the destination, the lane's generation and the request's
    /// ticket; `None` when the pacer doesn't pace.
    sent: Option<(Arc<RequestPacer>, PaceKey, u64, u64)>,
}

impl PaceGuard {
    /// The turn of a request its client doesn't pace: it ends nothing.
    #[inline]
    pub(crate) fn unpaced() -> Self {
        Self { sent: None }
    }
}

impl Drop for PaceGuard {
    fn drop(&mut self) {
        let Some((pacer, key, generation, ticket)) = self.sent.take() else {
            return;
        };
        let mut lanes = pacer.lock();
        if let Some(lane) = lanes.by_destination.get_mut(&key) {
            // Only the latest request's finish moves the next one; an
            // earlier request finishing late, or one from a lane forgotten
            // and made again, changes nothing.
            if lane.generation == generation && lane.ticket == ticket {
                lane.finished = Some(Instant::now());
            }
        }
    }
}

/// A request's turn at its destination, not yet taken: an audited client's,
/// which its node takes once it has admitted the request (#2652).
pub(crate) struct Turn {
    pacer: Arc<RequestPacer>,
    key: PaceKey,
    /// The client's tasks, as [`RequestPacer::wait`] watches them.
    ended: watch::Receiver<()>,
}

impl Turn {
    pub(crate) fn new(
        pacer: &Arc<RequestPacer>,
        key: PaceKey,
        ended: &watch::Receiver<()>,
    ) -> Self {
        Self {
            pacer: Arc::clone(pacer),
            key,
            ended: ended.clone(),
        }
    }

    /// Wait for the turn and take it, as [`RequestPacer::wait`] does:
    /// `None` once the client's tasks end first. Dropping the wait takes
    /// nothing.
    pub(crate) async fn take(&self) -> Option<PaceGuard> {
        self.pacer.wait(self.key.clone(), &self.ended).await
    }

    /// Wait until the turn is due, without taking it: `false` once the
    /// client's tasks end first. Another request to the destination may
    /// take it before this one does.
    pub(crate) async fn due(&self) -> bool {
        let mut ending = None;
        while let Some(due) = self.pacer.due(&self.key) {
            let ending = ending.get_or_insert_with(|| self.ended.clone());
            tokio::select! {
                biased;
                // Never sent on, so this resolves only when the sender drops.
                _ = ending.changed() => return false,
                () = tokio::time::sleep_until(due) => {}
            }
        }
        true
    }

    /// Take the turn if it is due now.
    pub(crate) fn try_take(&self) -> Option<PaceGuard> {
        self.pacer.try_claim(&self.key)
    }
}

impl RequestPacer {
    pub(crate) fn new(interval: Duration) -> Self {
        Self::with_capacity(interval, MAX_PACED_DESTINATIONS)
    }

    fn with_capacity(interval: Duration, capacity: usize) -> Self {
        Self {
            interval,
            capacity,
            lanes: Mutex::new(Lanes::default()),
            audit: OnceLock::new(),
        }
    }

    /// Whether this pacer spaces requests at all: an interval of zero
    /// paces nothing, and a request needs no turn from it.
    #[inline]
    pub(crate) fn paces(&self) -> bool {
        !self.interval.is_zero()
    }

    /// Whether a request waits for its turn in its client's request path:
    /// one of a client this pacer spaces, unless the client's node audits
    /// its requests. That node takes the turn once it has admitted the
    /// request (#2652), so a request that waits there for its admission
    /// takes its turn after that wait, not before it.
    #[inline]
    pub(crate) fn paces_in_request_path(&self) -> bool {
        self.paces()
            && !matches!(
                self.audit.get(),
                Some(super::source_audit::Role::Audited(_))
            )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lanes> {
        self.lanes.locked()
    }

    /// Make room for one more destination.
    fn evict(&self, lanes: &mut Lanes, now: Instant) {
        let interval = self.interval;
        lanes
            .by_destination
            .retain(|_, lane| !(lane.idle() && lane.due(interval) <= now));
        if lanes.by_destination.len() < self.capacity {
            return;
        }
        let soonest = |idle_only: bool| {
            lanes
                .by_destination
                .iter()
                .filter(|(_, lane)| !idle_only || lane.idle())
                .min_by_key(|(_, lane)| lane.due(interval))
                .map(|(key, _)| key.clone())
        };
        if let Some(key) = soonest(true).or_else(|| soonest(false)) {
            lanes.by_destination.remove(&key);
        }
    }

    /// Wait until a request to `key` may go, make it the destination's
    /// latest, and return the guard the request holds until it finishes.
    ///
    /// The check and the claim happen under one lock, so two requests that
    /// wake together can't both go: the second sees the first and sleeps
    /// again. The lock is never held while sleeping.
    ///
    /// `None` once `ended` ends (its sender is dropped: the client's
    /// dispatch task has ended, by a stop or its node's stop or drop) while
    /// the request sleeps (#2720). Such a request never went, so it claims
    /// no turn: the destination's next request is due when it was before.
    /// A request that doesn't have to sleep never looks at `ended`.
    pub(crate) async fn wait(
        self: &Arc<Self>,
        key: PaceKey,
        ended: &watch::Receiver<()>,
    ) -> Option<PaceGuard> {
        if self.interval.is_zero() {
            return Some(PaceGuard { sent: None });
        }
        let interval = self.interval;
        // Cloned once the request has to sleep, so one that goes at once
        // pays nothing for it.
        let mut ending = None;
        loop {
            let due = {
                let mut lanes = self.lock();
                let now = Instant::now();
                match lanes
                    .by_destination
                    .get(&key)
                    .map(|lane| lane.due(interval))
                {
                    Some(due) if due > now => due,
                    _ => {
                        let (key, generation, ticket) = self.claim(&mut lanes, &key, now);
                        return Some(PaceGuard {
                            sent: Some((Arc::clone(self), key, generation, ticket)),
                        });
                    }
                }
            };
            let ending = ending.get_or_insert_with(|| ended.clone());
            // Neither branch takes a lock: the pacer's is a std mutex, held
            // only above, between polls.
            tokio::select! {
                biased;
                // Never sent on, so this resolves only when the sender drops.
                _ = ending.changed() => return None,
                () = tokio::time::sleep_until(due) => {}
            }
        }
    }

    /// When a request to `key` may go, or `None` when it may go now.
    fn due(&self, key: &PaceKey) -> Option<Instant> {
        let lanes = self.lock();
        let due = lanes.by_destination.get(key)?.due(self.interval);
        (due > Instant::now()).then_some(due)
    }

    /// A turn for `key` if one is due now, taken as [`Self::wait`] takes
    /// it; the check and the claim under one lock.
    fn try_claim(self: &Arc<Self>, key: &PaceKey) -> Option<PaceGuard> {
        let mut lanes = self.lock();
        let now = Instant::now();
        if (lanes.by_destination.get(key)).is_some_and(|lane| lane.due(self.interval) > now) {
            return None;
        }
        let (key, generation, ticket) = self.claim(&mut lanes, key, now);
        Some(PaceGuard {
            sent: Some((Arc::clone(self), key, generation, ticket)),
        })
    }

    /// Make a request sent `now` the latest to `key`, returning the lane's
    /// generation and the request's ticket.
    fn claim(&self, lanes: &mut Lanes, key: &PaceKey, now: Instant) -> (PaceKey, u64, u64) {
        if let Some(lane) = lanes.by_destination.get_mut(key) {
            lane.ticket += 1;
            lane.sent = now;
            lane.finished = None;
            return (key.clone(), lane.generation, lane.ticket);
        }
        if lanes.by_destination.len() >= self.capacity {
            self.evict(lanes, now);
        }
        lanes.next_generation += 1;
        let lane = Lane {
            generation: lanes.next_generation,
            ticket: 1,
            sent: now,
            finished: None,
        };
        lanes.by_destination.insert(key.clone(), lane);
        (key.clone(), lane.generation, lane.ticket)
    }

    /// [`Self::wait`] for a client whose tasks never end, for tests of the
    /// pacing alone.
    #[cfg(test)]
    pub(crate) async fn turn(self: &Arc<Self>, key: PaceKey) -> PaceGuard {
        static RUNNING: OnceLock<(watch::Sender<()>, watch::Receiver<()>)> = OnceLock::new();
        let running = &RUNNING.get_or_init(|| watch::channel(())).1;
        self.wait(key, running)
            .await
            .expect("a client whose tasks run")
    }

    #[cfg(test)]
    pub(crate) fn remembered(&self) -> usize {
        self.lock().by_destination.len()
    }

    /// The destination's latest request: when it was sent and finished.
    #[cfg(test)]
    pub(crate) fn latest(&self, key: &PaceKey) -> Option<(Instant, Option<Instant>)> {
        self.lock()
            .by_destination
            .get(key)
            .map(|lane| (lane.sent, lane.finished))
    }
}

#[cfg(test)]
pub(super) fn with_capacity(interval: Duration, capacity: usize) -> Arc<RequestPacer> {
    Arc::new(RequestPacer::with_capacity(interval, capacity))
}
