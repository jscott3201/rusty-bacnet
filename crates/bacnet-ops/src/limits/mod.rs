//! The session's request limits.
//!
//! [`RequestLimits`] is shared by every worker that asks devices anything:
//! a global cap on how many requests a second leave across every device,
//! and one gate per device, so a device never has more than one of our
//! confirmed requests outstanding, whether the browser, the watch list, a
//! write or a managed COV renewal sent it (#1791). Small devices often
//! handle one transaction at a time. A gate is keyed by the device's
//! [`DeviceAddress`], so a device named by its instance and by its
//! address has one gate, and two devices sharing an instance have two
//! (#2560).
//!
//! The MCP server adds two layers (#2066): a rate per network, since a slow
//! MS/TP trunk behind a router takes far fewer requests than the IP side
//! ([`RequestLimits::with_network_rates`]), and a per-caller [`Account`]
//! with its own [`TokenBucket`] ([`charged`]).

mod account;
mod schedule;

pub use self::account::{Account, TokenBucket, charged, refund};

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use self::schedule::{Booking, Schedule};
use crate::device::DeviceAddress;

/// How long, in seconds, the request-rate meter ([`RequestLimits::recent`])
/// remembers a request.
pub const METER_WINDOW_SECS: u64 = 10;

/// Spaces requests evenly: at most `per_second` a second across every job,
/// the browser's and the watch list's alike. Each caller takes the next free
/// slot and sleeps until it; the lock is held only to take the slot, never
/// across the sleep. A wait dropped before its slot gives the slot back
/// (#2202). It also remembers when recent requests went, for the
/// request-rate meter.
///
/// It also holds each device's gate: a one-permit semaphore that a request
/// to the device holds from before its slot until its answer (see
/// [`turn`](Self::turn)). The std mutex around the map is held only to look
/// a gate up, never across an await.
pub struct RequestLimits {
    interval: Duration,
    state: Mutex<Slots>,
    gates: Mutex<HashMap<DeviceAddress, Arc<Semaphore>>>,
    networks: Option<Networks>,
    /// Limits that join these for a while ([`set_overlay`](Self::set_overlay)).
    /// The lock is held only to swap or clone the layer, never across an
    /// await.
    overlay: Mutex<Option<Arc<Layer>>>,
}

/// Limits that join a session's own for a while, as an attached agent's
/// do while it is attached (#2185): a global rate, the stricter of it and
/// the session's own counting, and a rate per network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// Requests a second across every network (0 for no cap of its own).
    pub global: u32,
    /// Requests a second to a network without a rate of its own (0 for no
    /// cap).
    pub network: u32,
    /// The networks with rates of their own.
    pub rates: Vec<(u16, u32)>,
}

/// An [`Overlay`] in force.
struct Layer {
    interval: Duration,
    networks: Networks,
}

impl Networks {
    fn new(default: u32, rates: impl IntoIterator<Item = (u16, u32)>) -> Self {
        Self {
            default: spacing(default),
            rates: rates
                .into_iter()
                .map(|(network, per_second)| (network, spacing(per_second)))
                .collect(),
            next: Mutex::new(HashMap::new()),
        }
    }
}

/// The rate of each network: requests to it are spaced as the global cap
/// spaces all of them. The lock is held only to take a slot.
struct Networks {
    /// The spacing of a network without a rate of its own; zero for none.
    default: Duration,
    rates: HashMap<u16, Duration>,
    next: Mutex<HashMap<u16, Schedule>>,
}

impl Networks {
    /// The spacing of requests to `network`; zero for none.
    fn interval(&self, network: u16) -> Duration {
        self.rates.get(&network).copied().unwrap_or(self.default)
    }

    /// These rates joined with `other`'s: each network at the stricter.
    fn join(&self, other: &Self) -> Self {
        let networks = self.rates.keys().chain(other.rates.keys());
        Self {
            default: self.default.max(other.default),
            rates: networks
                .map(|&n| (n, self.interval(n).max(other.interval(n))))
                .collect(),
            next: Mutex::new(HashMap::new()),
        }
    }

    /// The slot a request to `network` may go in, reserving it; `None`
    /// when the network has no rate.
    fn reserve(&self, network: u16, now: Instant) -> Option<Booking> {
        let interval = self.interval(network);
        if interval.is_zero() {
            return None;
        }
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        Some(next.entry(network).or_default().book(now, interval))
    }

    /// Give back `network`'s slot `booking`, unless its instant has come.
    fn release(&self, network: u16, booking: Booking, now: Instant) {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(schedule) = next.get_mut(&network) {
            schedule.release(booking, now);
        }
    }
}

/// A slot booked and not yet reached. Dropped before the slot's instant
/// (its wait abandoned), it gives the slot back, so later requests don't
/// queue behind it; once the instant has come it changes nothing.
struct Hold<'a> {
    owner: Owner<'a>,
    booking: Booking,
}

/// Whose schedule a [`Hold`]'s slot is on.
enum Owner<'a> {
    Global(&'a RequestLimits),
    Network(&'a Networks, u16),
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        let now = Instant::now();
        // The usual case, a request whose slot came: no lock to take.
        if self.booking.at <= now {
            return;
        }
        match self.owner {
            Owner::Global(limits) => limits.release(self.booking, now),
            Owner::Network(networks, network) => networks.release(network, self.booking, now),
        }
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
    /// The slots booked and given back.
    schedule: Schedule,
    /// The slots handed out within the meter's window, oldest first, each
    /// marked when an agent's account was charged for it; a slot given back
    /// leaves it.
    recent: VecDeque<(Instant, bool)>,
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
            overlay: Mutex::new(None),
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
        self.networks = Some(Networks::new(default, rates));
        self
    }

    /// Let `overlay` join these limits until it is replaced or cleared
    /// (`None`): requests then keep to the stricter of the two global
    /// rates, and per network to the stricter of the two. The switch
    /// takes effect from the next request; no client is rebuilt.
    pub fn set_overlay(&self, overlay: Option<&Overlay>) {
        let layer = overlay.map(|overlay| {
            let networks = Networks::new(overlay.network, overlay.rates.iter().copied());
            Arc::new(Layer {
                interval: spacing(overlay.global),
                networks: match &self.networks {
                    Some(own) => own.join(&networks),
                    None => networks,
                },
            })
        });
        *self.overlay.lock().unwrap_or_else(PoisonError::into_inner) = layer;
    }

    /// The overlay in force, if any.
    fn layer(&self) -> Option<Arc<Layer>> {
        self.overlay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// True when requests are spaced per network too.
    pub fn paces_networks(&self) -> bool {
        self.networks.is_some() || self.layer().is_some()
    }

    /// `device`'s gate: one permit, held by whichever request to it is
    /// outstanding. A managed COV subscription takes it for its renewals.
    pub fn gate(&self, device: &DeviceAddress) -> Arc<Semaphore> {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(gate) = gates.get(device) {
            return Arc::clone(gate);
        }
        let gate = Arc::new(Semaphore::new(1));
        gates.insert(device.clone(), Arc::clone(&gate));
        gate
    }

    /// Wait for `device`'s gate, then for this request's slot under its
    /// network's rate (when the limits pace networks) and the global one.
    /// The turn holds the gate until it is dropped, so keep it for exactly
    /// the one request: a job that takes a second turn while holding the
    /// first waits on itself.
    ///
    /// The caller's token, when its [`Account`] has a bucket, comes first,
    /// so a caller waiting for its own rate doesn't hold the device's gate.
    /// The gate comes next so a request queued behind another one to the
    /// same device holds no slot meanwhile. The wait is cancel-safe: a
    /// dropped wait leaves the semaphore's queue.
    pub async fn turn(&self, device: &DeviceAddress) -> Turn {
        let network = Some(device.network());
        let account = account::current();
        let start = Instant::now();
        // Held until the request goes: a wait dropped before gives it back.
        let token = match account.as_deref().and_then(Account::bucket) {
            Some(bucket) => bucket.hold().await,
            None => None,
        };
        // The gate is never closed, so the permit always comes.
        let permit = self.gate(device).acquire_owned().await.ok();
        self.pace(network, account.as_deref().is_some_and(Account::is_agent))
            .await;
        if let Some(token) = token {
            token.keep();
        }
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
        let token = match account.as_deref().and_then(Account::bucket) {
            Some(bucket) => bucket.hold().await,
            None => None,
        };
        self.pace(network, account.as_deref().is_some_and(Account::is_agent))
            .await;
        if let Some(token) = token {
            token.keep();
        }
        if let Some(account) = &account {
            account.count_request();
            account.add_wait(start.elapsed());
        }
    }

    /// Wait for `network`'s slot, then the global one, under the overlay
    /// too while one is in force; on the meter as an agent's when `agent`.
    async fn pace(&self, network: Option<u16>, agent: bool) {
        let layer = self.layer();
        let networks = layer
            .as_deref()
            .map(|layer| &layer.networks)
            .or(self.networks.as_ref());
        if let (Some(networks), Some(network)) = (networks, network)
            && let Some(booking) = networks.reserve(network, Instant::now())
        {
            let _hold = Hold {
                owner: Owner::Network(networks, network),
                booking,
            };
            tokio::time::sleep_until(booking.at).await;
        }
        let interval = self.global_spacing(layer.as_deref());
        if let Some(booking) = self.reserve(Instant::now(), interval, agent) {
            let _hold = Hold {
                owner: Owner::Global(self),
                booking,
            };
            tokio::time::sleep_until(booking.at).await;
        }
    }

    /// Wait until `device`'s gate is free and a request could go at once,
    /// taking neither; see [`wait_free`](Self::wait_free) for why.
    pub async fn wait_device_free(&self, device: &DeviceAddress) {
        drop(self.gate(device).acquire_owned().await);
        self.wait_free().await;
    }

    /// The global spacing in force: the stricter of these limits' own and
    /// `layer`'s.
    fn global_spacing(&self, layer: Option<&Layer>) -> Duration {
        layer.map_or(self.interval, |layer| layer.interval.max(self.interval))
    }

    /// The slot this request may go in, `interval` before the next,
    /// reserving it, on the meter as an agent's when `agent`. Without
    /// spacing it goes now (`None`), whatever slots spaced requests left
    /// booked.
    fn reserve(&self, now: Instant, interval: Duration, agent: bool) -> Option<Booking> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let booking = if interval.is_zero() {
            state.schedule.clear(now);
            None
        } else {
            Some(state.schedule.book(now, interval))
        };
        while state
            .recent
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= METER_WINDOW)
        {
            state.recent.pop_front();
        }
        // A slot given back and taken again lies before later bookings.
        let at = booking.map_or(now, |booking| booking.at);
        let i = state.recent.partition_point(|(recent, _)| *recent <= at);
        state.recent.insert(i, (at, agent));
        booking
    }

    /// Give back the global slot `booking`, and its place on the meter,
    /// unless its instant has come.
    fn release(&self, booking: Booking, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.schedule.release(booking, now)
            && let Some(i) = state.recent.iter().rposition(|(at, _)| *at == booking.at)
        {
            state.recent.remove(i);
        }
    }

    /// Wait for this request's slot under the rate cap alone; a request to
    /// a device takes a [`turn`](Self::turn), which includes one. The call's
    /// [`Account`], if any, is charged as by [`slot`](Self::slot).
    pub async fn acquire(&self) {
        self.slot(None).await;
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
            // Without spacing a request could always go at once.
            let interval = self.global_spacing(self.layer().as_deref());
            if interval.is_zero() {
                return;
            }
            // Best effort: a hole ahead counts as the instant a request
            // could go, but on a real clock the wake can land just past
            // it (when `book` no longer offers it), and `book` may take a
            // later hole while the next slot is earlier, so a request
            // made on return may still wait a little. Without holes this
            // is exact.
            let next = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .schedule
                .next_free(Instant::now(), interval);
            match next {
                Some(at) if at > Instant::now() => tokio::time::sleep_until(at).await,
                _ => return,
            }
        }
    }

    /// Requests whose slot fell in the meter's window up to now.
    pub fn recent(&self) -> usize {
        self.metered(|_| true)
    }

    /// Of [`recent`](Self::recent), those charged to an agent's
    /// [`Account`] (#2185): the requests of its tool calls, the re-read
    /// before its writes included, and of a watch hub that serves agents.
    pub fn recent_agents(&self) -> usize {
        self.metered(|agent| agent)
    }

    fn metered(&self, which: impl Fn(bool) -> bool) -> usize {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .recent
            .iter()
            .filter(|(at, agent)| {
                *at <= now && now.saturating_duration_since(*at) < METER_WINDOW && which(*agent)
            })
            .count()
    }
}

#[cfg(test)]
mod giveback_tests;

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Device `device` on `network`, as a test names it: a six-octet MAC
    /// holding the number.
    pub(in crate::limits) fn at(network: u16, device: u32) -> DeviceAddress {
        let mut mac = [0; 6];
        mac[2..].copy_from_slice(&device.to_be_bytes());
        DeviceAddress::new(network, &mac).unwrap()
    }

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
    /// while another network goes at the default. A device's network is
    /// its address's, so every request is paced by it.
    #[tokio::test(start_paused = true)]
    async fn a_slow_network_is_held_to_its_rate() {
        let limits = RequestLimits::new(0).with_network_rates(20, [(2001, 2)]);
        assert!(limits.paces_networks());
        let start = Instant::now();
        let mut slow = Vec::new();
        for device in 0..5 {
            drop(limits.turn(&at(2001, device)).await);
            slow.push((Instant::now() - start).as_millis());
        }
        assert_eq!(slow, [0, 500, 1000, 1500, 2000]);
        let before = Instant::now();
        for device in 10..14 {
            drop(limits.turn(&at(0, device)).await);
        }
        assert_eq!(Instant::now() - before, Duration::from_millis(150));
    }

    /// An overlay holds requests to its rates while it is in force, the
    /// stricter global rate winning, and the limits' own apply again once
    /// it is cleared.
    #[tokio::test(start_paused = true)]
    async fn an_overlay_joins_the_limits_while_it_is_in_force() {
        let limits = RequestLimits::new(50);
        let spaced = async |limits: &RequestLimits, network| {
            let start = Instant::now();
            for device in 0..3 {
                drop(limits.turn(&at(network, device)).await);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            (Instant::now() - start - Duration::from_secs(1)).as_millis()
        };
        assert!(!limits.paces_networks());
        assert_eq!(spaced(&limits, 0).await, 40, "its own 50 a second");
        let overlay = Overlay {
            global: 10,
            network: 20,
            rates: vec![(2001, 2)],
        };
        limits.set_overlay(Some(&overlay));
        assert!(limits.paces_networks());
        assert_eq!(spaced(&limits, 0).await, 200, "the overlay's 10");
        assert_eq!(spaced(&limits, 2001).await, 1000, "network 2001's 2");
        limits.set_overlay(Some(&Overlay {
            global: 100,
            network: 0,
            ..overlay
        }));
        assert_eq!(spaced(&limits, 0).await, 40, "the stricter, its own");
        limits.set_overlay(None);
        assert!(!limits.paces_networks());
        assert_eq!(spaced(&limits, 2001).await, 40, "its own again");
    }

    /// Per network the overlay joins the limits' own rates, the stricter
    /// winning; without a global cap, a request goes at once once the
    /// overlay is cleared, though overlay-spaced slots were booked ahead.
    #[tokio::test(start_paused = true)]
    async fn an_overlay_joins_network_rates_and_leaves_no_wait_behind() {
        let limits = RequestLimits::new(0).with_network_rates(0, [(2001, 4), (2003, 5)]);
        let spaced = async |limits: &RequestLimits, network| {
            let start = Instant::now();
            for device in 0..3 {
                drop(limits.turn(&at(network, device)).await);
            }
            let spent = (Instant::now() - start).as_millis();
            tokio::time::sleep(Duration::from_secs(1)).await;
            spent
        };
        let overlay = Overlay {
            global: 0,
            network: 0,
            rates: vec![(2001, 2), (2003, 10)],
        };
        limits.set_overlay(Some(&overlay));
        assert_eq!(spaced(&limits, 2001).await, 1000, "the overlay's 2");
        assert_eq!(spaced(&limits, 2003).await, 400, "its own 5");
        limits.set_overlay(Some(&Overlay {
            global: 1,
            ..overlay
        }));
        for device in 0..3 {
            drop(limits.turn(&at(0, device)).await);
        }
        limits.set_overlay(None);
        let start = Instant::now();
        limits.wait_free().await;
        let before = limits.recent();
        drop(limits.turn(&at(0, 9)).await);
        assert_eq!(Instant::now(), start, "no wait on slots booked ahead");
        assert_eq!(limits.recent(), before + 1, "and on the meter now");
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
                drop(limits.turn(&at(0, 1)).await);
            }
            limits.slot(None).await;
        })
        .await;
        assert_eq!(account.requests(), 4);
        // The bucket's rate, 2 a second, outweighs the global 10.
        assert_eq!(account.throttled(), Duration::from_millis(1500));
        // Uncharged requests count nowhere.
        drop(limits.turn(&at(0, 1)).await);
        assert_eq!(account.requests(), 4);
    }

    /// The meter tells an agent's requests from the rest (#2185): those
    /// charged to an agent's account, at once or paced, and nothing else.
    #[tokio::test(start_paused = true)]
    async fn the_meter_counts_agents_requests_apart() {
        let limits = RequestLimits::new(10);
        drop(limits.turn(&at(0, 1)).await);
        charged(Arc::new(Account::new(None)), limits.slot(None)).await;
        charged(Arc::new(Account::agent(None)), async {
            drop(limits.turn(&at(0, 2)).await);
            limits.slot(None).await;
        })
        .await;
        assert_eq!((limits.recent(), limits.recent_agents()), (4, 2));
        tokio::time::advance(METER_WINDOW).await;
        assert_eq!((limits.recent(), limits.recent_agents()), (0, 0));
    }
}
