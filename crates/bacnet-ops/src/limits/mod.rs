//! The session's request limits.
//!
//! [`RequestLimits`] is shared by every worker that asks devices anything:
//! a global cap on how many requests a second leave across every device,
//! and one gate per device, so a device never has more than one of our
//! confirmed requests outstanding, whether the browser, the watch list, a
//! write or a managed COV renewal sent it (#1791). Small devices often
//! handle one transaction at a time.
//!
//! The MCP server adds two layers (#2066): a rate per network, since a slow
//! MS/TP trunk behind a router takes far fewer requests than the IP side
//! ([`RequestLimits::with_network_rates`]), and a per-caller [`Account`]
//! with its own [`TokenBucket`] ([`charged`]).

mod account;

pub use self::account::{Account, TokenBucket, charged, refund};

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// How long, in seconds, the request-rate meter ([`RequestLimits::recent`])
/// remembers a request.
pub const METER_WINDOW_SECS: u64 = 10;

/// Spaces requests evenly: at most `per_second` a second across every job,
/// the browser's and the watch list's alike. Each caller takes the next free
/// slot and sleeps until it; the lock is held only to take the slot, never
/// across the sleep. It also remembers when recent requests went, for the
/// request-rate meter.
///
/// It also holds each device's gate: a one-permit semaphore that a request
/// to the device holds from before its slot until its answer (see
/// [`turn`](Self::turn)). The std mutex around the map is held only to look
/// a gate up, never across an await.
pub struct RequestLimits {
    interval: Duration,
    state: Mutex<Slots>,
    gates: Mutex<HashMap<u32, Arc<Semaphore>>>,
    networks: Option<Networks>,
}

/// The rate of each network: requests to it are spaced as the global cap
/// spaces all of them. The lock is held only to take a slot.
struct Networks {
    /// The spacing of a network without a rate of its own; zero for none.
    default: Duration,
    rates: HashMap<u16, Duration>,
    next: Mutex<HashMap<u16, Instant>>,
}

impl Networks {
    /// The instant a request to `network` may go, reserving it; `None`
    /// when the network has no rate.
    fn reserve(&self, network: u16, now: Instant) -> Option<Instant> {
        let interval = self.rates.get(&network).copied().unwrap_or(self.default);
        if interval.is_zero() {
            return None;
        }
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let at = next.entry(network).or_insert(now);
        let slot = (*at).max(now);
        *at = slot + interval;
        Some(slot)
    }
}

/// The spacing of `per_second` requests a second; zero for 0 (no cap).
fn spacing(per_second: u32) -> Duration {
    match per_second {
        0 => Duration::ZERO,
        n => Duration::from_secs(1) / n,
    }
}

/// A request's turn at its device: the device's gate, held until this is
/// dropped, on every path out of the request (an answer, an error, or the
/// request's future dropped).
#[must_use = "the device's gate is held only while the turn lives"]
pub struct Turn {
    _permit: Option<OwnedSemaphorePermit>,
}

#[derive(Default)]
struct Slots {
    /// The next free slot.
    next: Option<Instant>,
    /// The slots handed out within the meter's window, oldest first.
    recent: VecDeque<Instant>,
}

/// How long the meter remembers a request.
const METER_WINDOW: Duration = Duration::from_secs(METER_WINDOW_SECS);

impl RequestLimits {
    /// A cap of `per_second` requests a second; 0 means no cap.
    pub fn new(per_second: u32) -> Self {
        Self {
            interval: spacing(per_second),
            state: Mutex::new(Slots::default()),
            gates: Mutex::new(HashMap::new()),
            networks: None,
        }
    }

    /// These limits, with a rate per network as well: `default` requests a
    /// second for any network (0 for no cap of its own), and `rates` for
    /// the networks named, network 0 being the local one. The network
    /// number doesn't say what link lies behind a router, so a slow MS/TP
    /// trunk has to be named.
    pub fn with_network_rates(
        mut self,
        default: u32,
        rates: impl IntoIterator<Item = (u16, u32)>,
    ) -> Self {
        self.networks = Some(Networks {
            default: spacing(default),
            rates: rates
                .into_iter()
                .map(|(network, per_second)| (network, spacing(per_second)))
                .collect(),
            next: Mutex::new(HashMap::new()),
        });
        self
    }

    /// True when requests are spaced per network too, so a request has to
    /// say which network its device is on ([`turn_on`](Self::turn_on)).
    pub fn paces_networks(&self) -> bool {
        self.networks.is_some()
    }

    /// `device`'s gate: one permit, held by whichever request to it is
    /// outstanding. A managed COV subscription takes it for its renewals.
    pub fn gate(&self, device: u32) -> Arc<Semaphore> {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            gates
                .entry(device)
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    /// Wait for `device`'s gate, then for this request's slot. The turn
    /// holds the gate until it is dropped, so keep it for exactly the one
    /// request: a job that takes a second turn while holding the first
    /// waits on itself.
    ///
    /// The gate comes first so a request queued behind another one to the
    /// same device holds no slot meanwhile. The wait is cancel-safe: a
    /// dropped wait leaves the semaphore's queue.
    pub async fn turn(&self, device: u32) -> Turn {
        self.turn_on(device, None).await
    }

    /// [`turn`](Self::turn) for a device on `network` (0 for the local
    /// one; `None` when unknown, which skips the network's rate). The
    /// caller's token, when its [`Account`] has a bucket, comes first, so
    /// a caller waiting for its own rate doesn't hold the device's gate.
    pub async fn turn_on(&self, device: u32, network: Option<u16>) -> Turn {
        let account = account::current();
        let start = Instant::now();
        if let Some(bucket) = account.as_deref().and_then(Account::bucket) {
            bucket.take().await;
        }
        // The gate is never closed, so the permit always comes.
        let permit = self.gate(device).acquire_owned().await.ok();
        self.pace(network).await;
        if let Some(account) = &account {
            account.count_request();
            account.add_wait(start.elapsed());
        }
        Turn { _permit: permit }
    }

    /// A slot for one more request while the caller already holds its
    /// device's turn, as a log page's later requests do: the caller's
    /// token, then `network`'s rate and the global rate, charged as a
    /// request of its own.
    pub async fn slot(&self, network: Option<u16>) {
        let account = account::current();
        let start = Instant::now();
        if let Some(bucket) = account.as_deref().and_then(Account::bucket) {
            bucket.take().await;
        }
        self.pace(network).await;
        if let Some(account) = &account {
            account.count_request();
            account.add_wait(start.elapsed());
        }
    }

    /// Wait for `network`'s slot, then the global one.
    async fn pace(&self, network: Option<u16>) {
        if let (Some(networks), Some(network)) = (&self.networks, network)
            && let Some(slot) = networks.reserve(network, Instant::now())
        {
            tokio::time::sleep_until(slot).await;
        }
        self.wait_slot().await;
    }

    /// Wait until `device`'s gate is free and a request could go at once,
    /// taking neither; see [`wait_free`](Self::wait_free) for why.
    pub async fn wait_device_free(&self, device: u32) {
        drop(self.gate(device).acquire_owned().await);
        self.wait_free().await;
    }

    /// The instant this request may go, reserving it.
    fn reserve(&self, now: Instant) -> Instant {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = state.next.map_or(now, |next| next.max(now));
        state.next = Some(slot + self.interval);
        while state
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= METER_WINDOW)
        {
            state.recent.pop_front();
        }
        state.recent.push_back(slot);
        slot
    }

    /// Wait for this request's slot under the rate cap alone; a request to
    /// a device takes a [`turn`](Self::turn), which includes one. The call's
    /// [`Account`], if any, is charged as by [`slot`](Self::slot).
    pub async fn acquire(&self) {
        self.slot(None).await;
    }

    async fn wait_slot(&self) {
        let slot = self.reserve(Instant::now());
        if !self.interval.is_zero() {
            tokio::time::sleep_until(slot).await;
        }
    }

    /// Wait until a request could go at once, without reserving a slot: a
    /// caller that decides only then what to send (the name reads) spends
    /// no slot on work withdrawn while it waited.
    ///
    /// That holds only while the caller doesn't await between this and its
    /// [`acquire`](Self::acquire): deciding what to send must be
    /// synchronous (the TUI's name reads take their batch under a std
    /// mutex). An await there lets other jobs book the free slot first, and
    /// the caller's request then waits for a later one, for rows that may
    /// have been withdrawn meanwhile.
    pub async fn wait_free(&self) {
        loop {
            let next = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .next;
            match next {
                Some(at) if at > Instant::now() => tokio::time::sleep_until(at).await,
                _ => return,
            }
        }
    }

    /// Requests whose slot fell in the meter's window up to now.
    pub fn recent(&self) -> usize {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .recent
            .iter()
            .filter(|at| **at <= now && now.saturating_duration_since(**at) < METER_WINDOW)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requests are spaced `1 / per_second` apart, whatever the caller,
    /// and a cap of zero never waits.
    #[tokio::test(start_paused = true)]
    async fn the_rate_cap_spaces_requests_evenly() {
        let cap = RequestLimits::new(4);
        let start = Instant::now();
        let mut slots = Vec::new();
        for _ in 0..3 {
            cap.acquire().await;
            slots.push(Instant::now() - start);
        }
        assert_eq!(
            slots,
            [
                Duration::ZERO,
                Duration::from_millis(250),
                Duration::from_millis(500)
            ]
        );
        // After an idle second the next request goes at once.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let before = Instant::now();
        cap.acquire().await;
        assert_eq!(Instant::now(), before);
        let none = RequestLimits::new(0);
        let before = Instant::now();
        for _ in 0..10 {
            none.acquire().await;
        }
        assert_eq!(Instant::now(), before);
    }

    /// Waiting for a free slot reserves none: the next request still goes
    /// as soon as it would have.
    #[tokio::test(start_paused = true)]
    async fn waiting_for_a_free_slot_takes_none() {
        let cap = RequestLimits::new(4);
        let start = Instant::now();
        cap.acquire().await;
        cap.wait_free().await;
        assert_eq!(Instant::now() - start, Duration::from_millis(250));
        cap.wait_free().await;
        cap.acquire().await;
        assert_eq!(Instant::now() - start, Duration::from_millis(250));
        assert_eq!(cap.recent(), 2);
    }

    /// A network with its own rate is held to it, whatever the global cap,
    /// while another network goes at the default; a request whose network
    /// is unknown takes only the global slot.
    #[tokio::test(start_paused = true)]
    async fn a_slow_network_is_held_to_its_rate() {
        let limits = RequestLimits::new(0).with_network_rates(20, [(2001, 2)]);
        assert!(limits.paces_networks());
        let start = Instant::now();
        let mut slow = Vec::new();
        for device in 0..5 {
            drop(limits.turn_on(device, Some(2001)).await);
            slow.push((Instant::now() - start).as_millis());
        }
        assert_eq!(slow, [0, 500, 1000, 1500, 2000]);
        let before = Instant::now();
        for device in 10..14 {
            drop(limits.turn_on(device, Some(0)).await);
        }
        assert_eq!(Instant::now() - before, Duration::from_millis(150));
        let before = Instant::now();
        drop(limits.turn_on(20, None).await);
        assert_eq!(Instant::now(), before);
    }

    /// A charged call counts each request and the time it waited, and
    /// takes its tokens from the account's bucket before the gate.
    #[tokio::test(start_paused = true)]
    async fn a_charged_call_counts_requests_and_waits() {
        let limits = RequestLimits::new(10);
        let bucket = Arc::new(TokenBucket::new(2, 1));
        let account = Arc::new(Account::new(Some(bucket)));
        charged(Arc::clone(&account), async {
            for _ in 0..3 {
                drop(limits.turn(1).await);
            }
            limits.slot(None).await;
        })
        .await;
        assert_eq!(account.requests(), 4);
        // The bucket's rate, 2 a second, outweighs the global 10.
        assert_eq!(account.throttled(), Duration::from_millis(1500));
        // Uncharged requests count nowhere.
        drop(limits.turn(1).await);
        assert_eq!(account.requests(), 4);
    }
}
