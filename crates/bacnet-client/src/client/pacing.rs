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
//! don't wait on each other, and an interval of zero changes nothing.
//!
//! Pacing runs before a routed request takes its path lease. That lease is
//! shared by every device on the same network behind the same router, so
//! there a request waiting for the lease goes as soon as the lease frees: the
//! pause after a reply holds for requests made one after another, not for
//! concurrent ones.
//!
//! The endpoint client paces through this same type (#1542), before it
//! reserves an invoke ID, so a request waiting for its turn holds nothing
//! from the device's shared pool.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
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
        return Err(Error::Encoding(format!(
            "invalid min-request-interval {interval_ms} ms; expected 0..={MAX_MIN_REQUEST_INTERVAL_MS}"
        )));
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

impl RequestPacer {
    pub(crate) fn new(interval: Duration) -> Self {
        Self::with_capacity(interval, MAX_PACED_DESTINATIONS)
    }

    fn with_capacity(interval: Duration, capacity: usize) -> Self {
        Self {
            interval,
            capacity,
            lanes: Mutex::new(Lanes::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lanes> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
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
    pub(crate) async fn wait(self: &Arc<Self>, key: PaceKey) -> PaceGuard {
        if self.interval.is_zero() {
            return PaceGuard { sent: None };
        }
        let interval = self.interval;
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
                        return PaceGuard {
                            sent: Some((Arc::clone(self), key, generation, ticket)),
                        };
                    }
                }
            };
            tokio::time::sleep_until(due).await;
        }
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
