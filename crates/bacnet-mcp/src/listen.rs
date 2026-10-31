//! Subscriptions (design §6): `subscriptions/listen` of the 2026-07-28
//! revision, which `rmcp` 3.5.1 implements. Updates go out as
//! `notifications/resources/updated` carrying only the URI; the client then
//! reads the resource.
//!
//! The SDK asks for the accepted filter synchronously, before it sends the
//! acknowledgment, so the watches are taken there ([`Subs::reserve`]): the
//! acknowledgment lists only the URIs honoured. A point URI is honoured
//! when it parses, its device is in the discovery table and in reach, and
//! both caps leave room: [`Caps::watches`](crate::policy::Caps::watches)
//! points per principal (the same point on two streams counts once) and
//! the watch hub's cap for the process. The leases wait for the stream's
//! `listen` call ([`Subs::claim`]), matched by the [`Ticket`] of the request
//! that made them and their URI list, so a stream refused at its start
//! frees its own reservation, never another's with the same URIs (#2157).
//! A reservation no stream claims within [`UNCLAIMED`] (the acknowledgment
//! couldn't be sent) is dropped.
//!
//! A session runs at most [`MAX_STREAMS`] streams at once; one more is
//! refused when its `listen` call begins. A stream's per-device alarm URIs
//! count with the session's legacy URIs against one bound
//! ([`crate::legacy::max_uris`]), held until the stream (or its unclaimed
//! reservation) goes, so the 60-second alarm poll has bounded work.
//!
//! The listen loop waits on a [`Follow`]: the leases' signals, the
//! session's and the ledger's (#2160), the discovery table's events and an
//! alarm tick. Its arms send notifications
//! and refresh alarms; the futures it leaves pending meanwhile wait on
//! watch and broadcast channels only, never a lock (#1498). Legacy
//! subscriptions ([`crate::legacy`]) follow theirs the same way, and their
//! holds live here too, so every way of watching shares the caps.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bacnet_ops::watch::{Changes, Lease, Point, WatchError};
use bacnet_transport::port::TransportPort;
use rmcp::ErrorData;
use rmcp::service::SubscriptionContext;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::context::{Ctx, Listening};
use crate::errors::ToolError;
use crate::follow::{Follow, Wake};
use crate::resources::uri::{self, Uri};

/// Why a watch is refused once the session has ended or the hub stopped.
pub(crate) const SHUTTING_DOWN: &str = "shutting-down";

/// How long a reservation waits for its stream.
const UNCLAIMED: Duration = Duration::from_secs(30);

/// Reservations waiting at once; past it the oldest is dropped.
const MAX_PENDING: usize = 16;

/// The most listen streams one session runs at once. Every stream
/// following a point is told of each of its changes, and each one polls
/// its alarm URIs, so the streams multiply that work; a host needs one
/// stream, and four leave room for one split by topic (points, the device
/// list, alarms, the session) while keeping the multiple small, like the
/// other per-session bounds here ([`MAX_PENDING`], [`MAX_DEVICE_PAGES`]).
pub(crate) const MAX_STREAMS: usize = 4;

/// The most device-list pages (`bacnet://devices` and its `after` pages,
/// counted by spelling) one listen stream follows. Each followed page is
/// announced on every change to the list, so this bounds one change's
/// fan-out and what a stream holds; 16 pages of the default 200 devices
/// cover 3,200 devices, and a client past that reads the later pages
/// again when the first is announced. Legacy subscriptions count pages
/// against the session's URI bound instead.
pub(crate) const MAX_DEVICE_PAGES: usize = 16;

/// How long a `bacnet_watch` point stays watched without being asked for:
/// as long as its cursor lasts.
pub(crate) const TOOL_IDLE: Duration = Duration::from_secs(600);

/// How often subscribed alarm resources are polled.
pub(crate) const ALARMS_EVERY: Duration = Duration::from_secs(60);

/// What the principal watches, through listen streams, legacy
/// subscriptions and `bacnet_watch`.
pub(crate) struct Subs {
    state: Mutex<SubsState>,
    /// Bumped whenever the legacy subscriptions change, for the session's
    /// notifier ([`crate::legacy::notify`]).
    legacy_changed: watch::Sender<u64>,
}

impl Default for Subs {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            legacy_changed: watch::Sender::new(0),
        }
    }
}

#[derive(Default)]
struct SubsState {
    /// Each point watched, and how many holders (streams, the tool) hold
    /// it: the per-principal cap counts the points.
    held: HashMap<Point, usize>,
    /// Reservations made for acknowledgments, waiting for their streams.
    pending: Vec<Pending>,
    /// `bacnet_watch`'s points and when each was last asked for.
    tool: HashMap<Point, (Instant, Lease)>,
    /// The session's legacy `resources/subscribe` URIs, each point URI
    /// with its hold.
    legacy: HashMap<String, Option<Hold>>,
    /// Per-device alarm URIs held by listen streams and their waiting
    /// reservations, one a stream each: they count with the legacy URIs
    /// against the session's URI bound ([`Subs::uris`]).
    alarm_uris: usize,
    /// Set when the session ends ([`Subs::clear`]): nothing new is
    /// watched after it.
    closed: bool,
}

/// One legacy subscription's watch on a point: counted against the
/// principal's cap, and its lease held, until it is dropped. It points
/// back weakly, so a hold left in a dropped server's state frees itself.
pub(crate) struct Hold {
    subs: Weak<Subs>,
    point: Point,
    lease: Lease,
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(subs) = self.subs.upgrade() {
            subs.release(&[self.point]);
        }
    }
}

struct Pending {
    at: Instant,
    ticket: Ticket,
    uris: Vec<String>,
    claim: Claim,
}

/// Which `subscriptions/listen` request a reservation is for. The SDK asks
/// for the accepted filter and then runs the stream within the one task it
/// spawned for the request, and the filter hook isn't told the request's
/// id, so the task's id names the request at both ends. Tokio only
/// promises an id unique among running tasks; a reservation outlives its
/// task only when its stream never began, and the sweep drops it. Calls
/// made outside any task (`None`) share one ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket(Option<tokio::task::Id>);

impl Ticket {
    /// The ticket of the request this task handles.
    pub(crate) fn current() -> Self {
        Self(tokio::task::try_id())
    }
}

/// The points one stream holds, counted against the principal's cap
/// until it is dropped, and their leases until the stream takes them;
/// likewise its per-device alarm URIs, counted against the session's URI
/// bound.
pub(crate) struct Claim {
    subs: Arc<Subs>,
    points: Vec<Point>,
    leases: Vec<(String, Lease)>,
    alarms: usize,
}

impl Claim {
    /// The lease set aside for `uri`, for a test.
    #[cfg(test)]
    pub(crate) fn lease(&self, uri: &str) -> Option<&Lease> {
        self.leases
            .iter()
            .find(|(held, _)| held == uri)
            .map(|(_, lease)| lease)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut state = self.subs.lock();
        state.release(&self.points);
        state.alarm_uris = state.alarm_uris.saturating_sub(self.alarms);
    }
}

impl SubsState {
    /// One holder fewer on each of `points`.
    fn release(&mut self, points: &[Point]) {
        for point in points {
            if let Some(count) = self.held.get_mut(point) {
                *count -= 1;
                if *count == 0 {
                    self.held.remove(point);
                }
            }
        }
    }
}

impl Subs {
    fn lock(&self) -> MutexGuard<'_, SubsState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Points the principal watches now.
    pub(crate) fn held(&self) -> usize {
        self.lock().held.len()
    }

    fn release(&self, points: &[Point]) {
        self.lock().release(points);
    }

    /// A lease on `point` for a new holder, if the principal's cap and the
    /// hub's leave room.
    pub(crate) fn take<T: TransportPort + 'static>(
        &self,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<Lease, &'static str> {
        let hub = ctx.hub.as_ref().ok_or("passive")?;
        let mut state = self.lock();
        if state.closed {
            return Err(SHUTTING_DOWN);
        }
        if !state.held.contains_key(&point) && state.held.len() >= ctx.policy.caps.watches {
            return Err("principal-cap");
        }
        let lease = hub.watch(point).map_err(|e| match e {
            WatchError::Shut => SHUTTING_DOWN,
            WatchError::Full { .. } => "process-cap",
        })?;
        *state.held.entry(point).or_default() += 1;
        Ok(lease)
    }

    /// A hold on `point` for a legacy subscription, under both caps.
    pub(crate) fn hold<T: TransportPort + 'static>(
        self: &Arc<Self>,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<Hold, &'static str> {
        let lease = self.take(ctx, point)?;
        Ok(Hold {
            subs: Arc::downgrade(self),
            point,
            lease,
        })
    }

    /// Whether the session subscribes to `uri` (legacy).
    pub(crate) fn legacy_has(&self, uri: &str) -> bool {
        self.lock().legacy.contains_key(uri)
    }

    /// Add the legacy subscription `uri`, with its point's hold, unless the
    /// session already holds `max` URIs. One added meanwhile by another
    /// call stays, and `hold` goes.
    pub(crate) fn legacy_add(
        &self,
        uri: &str,
        hold: Option<Hold>,
        max: usize,
    ) -> Result<(), &'static str> {
        let mut state = self.lock();
        let added = if state.closed {
            Err(SHUTTING_DOWN)
        } else if state.legacy.contains_key(uri) {
            Ok(false)
        } else if state.legacy.len() + state.alarm_uris >= max {
            Err("principal-cap")
        } else {
            state.legacy.insert(uri.to_string(), hold);
            Ok(true)
        };
        drop(state);
        // A hold not stored goes when this returns, after the lock: its
        // release takes it.
        if added? {
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        Ok(())
    }

    /// Drop the legacy subscription `uri`, if the session has it.
    pub(crate) fn legacy_remove(&self, uri: &str) {
        let removed = self.lock().legacy.remove(uri);
        if removed.is_some() {
            // The guard is gone: the hold's release takes the lock.
            drop(removed);
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
    }

    /// The legacy URIs, each point URI with its change signal.
    pub(crate) fn legacy_snapshot(&self) -> Vec<(String, Option<Changes>)> {
        self.lock()
            .legacy
            .iter()
            .map(|(uri, hold)| (uri.clone(), hold.as_ref().map(|h| h.lease.changes())))
            .collect()
    }

    /// Legacy URIs the session holds.
    pub(crate) fn legacy_count(&self) -> usize {
        self.lock().legacy.len()
    }

    /// URIs counted against the session's bound: the legacy ones and the
    /// listen streams' per-device alarm URIs.
    pub(crate) fn uris(&self) -> usize {
        let state = self.lock();
        state.legacy.len() + state.alarm_uris
    }

    /// Count one more per-device alarm URI for a listen stream, if the
    /// session's URI bound `max` leaves room.
    fn take_alarm_uri(&self, max: usize) -> bool {
        let mut state = self.lock();
        let room = !state.closed && state.legacy.len() + state.alarm_uris < max;
        if room {
            state.alarm_uris += 1;
        }
        room
    }

    /// Wakes when the legacy subscriptions change.
    pub(crate) fn legacy_watch(&self) -> watch::Receiver<u64> {
        self.legacy_changed.subscribe()
    }

    /// Drop `bacnet_watch` points not asked for within `idle`.
    pub(crate) fn expire_tool(&self, idle: Duration) {
        let expired: Vec<(Point, Lease)> = {
            let mut state = self.lock();
            let now = Instant::now();
            let old: Vec<Point> = state
                .tool
                .iter()
                .filter(|(_, (at, _))| now.saturating_duration_since(*at) >= idle)
                .map(|(point, _)| *point)
                .collect();
            old.into_iter()
                .filter_map(|point| state.tool.remove(&point).map(|(_, l)| (point, l)))
                .collect()
        };
        let points: Vec<Point> = expired.iter().map(|(p, _)| *p).collect();
        self.release(&points);
    }

    /// Keep `point` watched for `bacnet_watch`, marking it asked for now.
    pub(crate) fn tool_watch<T: TransportPort + 'static>(
        &self,
        ctx: &Ctx<T>,
        point: Point,
    ) -> Result<(), &'static str> {
        let now = Instant::now();
        if let Some((at, _)) = self.lock().tool.get_mut(&point) {
            *at = now;
            return Ok(());
        }
        let lease = self.take(ctx, point)?;
        let mut state = self.lock();
        if state.closed {
            // The session ended meanwhile; the lease goes after the lock.
            drop(state);
            self.release(&[point]);
            return Err(SHUTTING_DOWN);
        }
        if let Some((at, _)) = state.tool.get_mut(&point) {
            // Another call took it meanwhile: keep theirs.
            *at = now;
            drop(state);
            self.release(&[point]);
            return Ok(());
        }
        state.tool.insert(point, (now, lease));
        Ok(())
    }

    /// The URIs of `requested` this server honours, distinct and in order,
    /// with the leases they need taken and set aside for the stream of
    /// `ticket`.
    pub(crate) fn reserve<T: TransportPort + 'static>(
        self: &Arc<Self>,
        ctx: &Ctx<T>,
        ticket: Ticket,
        requested: &[String],
    ) -> Vec<String> {
        self.prune();
        if *ctx.listening.borrow() >= MAX_STREAMS {
            // The stream will be refused ([`run`]): take nothing for it.
            return Vec::new();
        }
        let max_uris = crate::legacy::max_uris(ctx);
        let mut accepted = Vec::new();
        let mut claim = Claim {
            subs: Arc::clone(self),
            points: Vec::new(),
            leases: Vec::new(),
            alarms: 0,
        };
        let mut seen = HashSet::new();
        let mut pages = 0;
        for text in requested {
            if !seen.insert(text.as_str()) {
                continue;
            }
            let honoured = match uri::parse(text) {
                Ok(Uri::Devices | Uri::DevicesAfter(_)) if ctx.hub.is_some() => {
                    pages += 1;
                    pages <= MAX_DEVICE_PAGES
                }
                Ok(Uri::Point(point)) if ctx.reach_known(point.device) => {
                    match self.take(ctx, point) {
                        Ok(lease) => {
                            claim.points.push(point);
                            claim.leases.push((text.clone(), lease));
                            true
                        }
                        Err(_) => false,
                    }
                }
                Ok(Uri::Alarms) => ctx.hub.is_some(),
                Ok(Uri::DeviceAlarms(device)) => {
                    let taken = ctx.hub.is_some()
                        && ctx.reach_known(device)
                        && self.take_alarm_uri(max_uris);
                    claim.alarms += usize::from(taken);
                    taken
                }
                Ok(Uri::Session | Uri::Ledger) => true,
                // A device summary has no updates; a device-list page past
                // the cap or on a passive server isn't honoured; anything
                // else isn't ours.
                Ok(Uri::Device(_) | Uri::Point(_) | Uri::Devices | Uri::DevicesAfter(_))
                | Err(_) => false,
            };
            if honoured {
                accepted.push(text.clone());
            }
        }
        if !accepted.is_empty() {
            let mut state = self.lock();
            if state.closed {
                // The session ended meanwhile: honour nothing, and let the
                // claim's leases go after the lock.
                drop(state);
                drop(claim);
                return Vec::new();
            }
            let mut oldest = Vec::new();
            while state.pending.len() >= MAX_PENDING {
                oldest.push(state.pending.remove(0));
            }
            state.pending.push(Pending {
                at: Instant::now(),
                ticket,
                uris: accepted.clone(),
                claim,
            });
            drop(state);
            // Dropped after the lock: their leases' release takes it.
            drop(oldest);
        }
        accepted
    }

    /// The leases reserved for the stream of `ticket`, acknowledged with
    /// `uris`.
    pub(crate) fn claim(&self, ticket: Ticket, uris: &[String]) -> Option<Claim> {
        let mut state = self.lock();
        let at = state
            .pending
            .iter()
            .position(|p| p.ticket == ticket && p.uris == uris)?;
        Some(state.pending.remove(at).claim)
    }

    /// The tickets of the reservations waiting, oldest first, for a test.
    #[cfg(test)]
    pub(crate) fn pending_tickets(&self) -> Vec<Ticket> {
        self.lock().pending.iter().map(|p| p.ticket).collect()
    }

    /// Drop reservations no stream claimed in time.
    pub(crate) fn prune(&self) {
        let stale: Vec<Pending> = {
            let mut state = self.lock();
            let now = Instant::now();
            let (old, kept) = std::mem::take(&mut state.pending)
                .into_iter()
                .partition(|p| now.saturating_duration_since(p.at) >= UNCLAIMED);
            state.pending = kept;
            old
        };
        drop(stale);
    }

    /// Drop every reservation, legacy subscription and tool watch, when
    /// the session ends.
    pub(crate) fn clear(&self) {
        let (pending, legacy, tool) = {
            let mut state = self.lock();
            state.closed = true;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.legacy),
                std::mem::take(&mut state.tool),
            )
        };
        drop(pending);
        if !legacy.is_empty() {
            drop(legacy);
            self.legacy_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        let points: Vec<Point> = tool.keys().copied().collect();
        drop(tool);
        self.release(&points);
    }
}

/// The accepted URIs of a stream, distinct, in the order the
/// acknowledgment lists them.
fn accepted(context: &SubscriptionContext) -> Vec<String> {
    let mut seen = HashSet::new();
    context
        .accepted()
        .resource_subscriptions
        .iter()
        .flatten()
        .filter(|uri| seen.insert(uri.as_str()))
        .cloned()
        .collect()
}

/// The stream of `ticket`, acknowledged with `uris`, beginning: counted
/// among the session's streams, with what [`Subs::reserve`] set aside for
/// it. Past [`MAX_STREAMS`] it is refused, and anything set aside for it
/// goes now, not at the sweep, so its watches and alarm URIs free their
/// room at once.
pub(crate) fn open<'a, T: TransportPort + 'static>(
    ctx: &'a Ctx<T>,
    ticket: Ticket,
    uris: &[String],
) -> Result<(Listening<'a>, Option<Claim>), ErrorData> {
    let Some(listening) = ctx.listening(MAX_STREAMS) else {
        drop(ctx.subs.claim(ticket, uris));
        let error = ToolError::refused(
            "stream-cap",
            format!(
                "this session runs {MAX_STREAMS} listen streams, as many as it may; \
                 cancel one first"
            ),
        );
        return Err(ErrorData::invalid_params(
            error.summary(),
            Some(error.json()),
        ));
    };
    Ok((listening, ctx.subs.claim(ticket, uris)))
}

/// Serve one stream until the client cancels it or the server closes.
/// `Ok` on a graceful close, which sends the stream's final result.
pub(crate) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    context: SubscriptionContext,
) -> Result<(), ErrorData> {
    let uris = accepted(&context);
    // Both held to the end: the stream's count, and the claim, whose
    // leases keep the points watched.
    let (_listening, claim) = open(ctx, Ticket::current(), &uris)?;
    if claim.is_none() && !uris.is_empty() {
        // Only when the reservation was dropped before the stream began.
        tracing::warn!(
            "a listen stream found no watches set aside for it; its points won't update"
        );
    }
    let leases = claim.as_ref().map_or(&[][..], |c| &c.leases[..]);
    let mut follow = Follow::new();
    follow.set(
        ctx.engine.as_deref(),
        &ctx.writes.signals,
        uris.iter()
            .map(|name| {
                let changes = leases
                    .iter()
                    .find(|(held, _)| held == name)
                    .map(|(_, lease)| lease.changes());
                (name.clone(), changes)
            })
            .collect(),
    );
    let sink = context.sink().clone();
    tracing::debug!(
        streams = uris.len(),
        quiet = follow.quiet(),
        "listen stream opened"
    );
    loop {
        let wake = tokio::select! {
            biased;
            () = context.cancelled() => return Ok(()),
            () = ctx.closing.cancelled() => return Ok(()),
            wake = follow.next() => wake,
        };
        match wake {
            Wake::Updated(name) => {
                if sink.notify_resource_updated(name).await.is_err() {
                    return Ok(());
                }
            }
            Wake::Devices => {
                for name in follow.device_uris() {
                    if sink.notify_resource_updated(name.as_str()).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Wake::Alarms => {
                for name in follow.alarm_uris() {
                    if crate::resources::refresh_alarms(ctx, name).await
                        && sink.notify_resource_updated(name.as_str()).await.is_err()
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
}
