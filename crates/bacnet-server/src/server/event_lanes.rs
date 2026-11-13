//! Ordered event notification sends: one lane per destination (#2537).
//!
//! Every notification the server originates or forwards is pushed onto one
//! lane per destination, the recipient a Recipient_List entry or a forwarder
//! names, under the database guard that committed it (a relay under the
//! read guard its forwarders were read under). Each lane is a FIFO drained
//! by one task of its own, so one destination's notifications are first
//! handed to the network in the order they were pushed, whichever task
//! committed them, and a destination that stalls (an offline peer, a slow
//! route) holds up its own lane only. A confirmed entry holds its lane until
//! its first attempt is handed off; its retries run in its worker.
//!
//! The lane map is a `std::sync::Mutex`, taken after the database guard and
//! never held across an await. A push never waits, so a writer holding the
//! database guard never blocks here: past [`MAX_LANE_ENTRIES`] on its lane,
//! [`MAX_LANED_ENTRIES`] in all or [`MAX_LANES`] destinations, the newest
//! entry is dropped and counted. A lane's task takes no database lock.
//!
//! A writer that reports a change by COV after its event notifications
//! (#889) gathers its pushes in a [`Settle`]: each entry carries a token,
//! and [`settled`] returns once the last one is gone (handed off, refused
//! or dropped), or after the APDU timeout, which is counted. `stop()` seals
//! the lanes and aborts their tasks: what they hold is dropped unsent, and
//! every writer waiting on them goes on.

use super::super::event_forwarding::ForwardingBudget;
use super::super::event_suppression::{EventSuppression, EventSuppressions};
use super::awaited::OwnedDelivery;
use super::*;
use bacnet_encoding::constructed::encode_event_notification;
use bacnet_services::alarm_event::{EventNotificationRequest, ForwardedEventNotification};
use std::collections::VecDeque;
use std::sync::OnceLock;
use tokio::task::JoinSet;

/// The most entries one destination's lane holds; one more is dropped and
/// counted in `lane_full`.
pub(crate) const MAX_LANE_ENTRIES: usize = 64;
/// The most entries across every lane; one more is dropped and counted in
/// `lanes_entries_full`.
pub(crate) const MAX_LANED_ENTRIES: usize = 1024;
/// The most destinations with a lane at once; an entry for one more is
/// dropped and counted in `lanes_full`.
pub(crate) const MAX_LANES: usize = 256;

/// One notification as its lanes send it: what each copy is encoded from,
/// shared by its destinations.
pub(in crate::server) struct LanePayload {
    notification_class: u32,
    priority: u8,
    request: LaneRequest,
    admits: AdmitsRoute,
    budget: Option<Arc<ForwardingBudget>>,
}

enum LaneRequest {
    /// A notification this device made.
    Local(EventNotificationRequest),
    /// A copy a Notification Forwarder sends on.
    Forwarded(ForwardedEventNotification),
}

impl LanePayload {
    /// A notification this device made, which every route admits.
    pub(in crate::server) fn local(notification: EventNotificationRequest) -> Self {
        static EVERY_ROUTE: OnceLock<AdmitsRoute> = OnceLock::new();
        Self {
            notification_class: notification.notification_class,
            priority: notification.priority,
            admits: Arc::clone(EVERY_ROUTE.get_or_init(|| Arc::new(|_| true))),
            request: LaneRequest::Local(notification),
            budget: None,
        }
    }

    /// A forwarded copy, kept to the routes `admits` lets it take and drawing
    /// on its notification's forwarding cap, `budget`.
    pub(in crate::server) fn forwarded(
        notification: ForwardedEventNotification,
        admits: AdmitsRoute,
        budget: Arc<ForwardingBudget>,
    ) -> Self {
        Self {
            notification_class: notification.notification_class,
            priority: notification.priority,
            admits,
            request: LaneRequest::Forwarded(notification),
            budget: Some(budget),
        }
    }

    fn encode_for(&self, process_identifier: u32) -> Result<Bytes, Error> {
        match &self.request {
            LaneRequest::Local(notification) => {
                let mut targeted = notification.clone();
                targeted.process_identifier = process_identifier;
                let mut buf = BytesMut::new();
                encode_event_notification(&targeted, &mut buf).map(|()| buf.freeze())
            }
            LaneRequest::Forwarded(notification) => Ok(notification.encode_for(process_identifier)),
        }
    }
}

/// One notification waiting in one destination's lane.
pub(in crate::server) struct LaneEntry {
    payload: Arc<LanePayload>,
    process_id: u32,
    confirmed: bool,
    _settle: Option<Arc<SettleToken>>,
}

impl LaneEntry {
    /// `payload` for `process_id`, confirmed or not, counted in `settle`
    /// when its writer waits on it. A writer that doesn't (the Event
    /// Enrollment pass, an acknowledgment, a relay) passes `None` and makes
    /// no token.
    pub(in crate::server) fn new(
        payload: &Arc<LanePayload>,
        process_id: u32,
        confirmed: bool,
        settle: Option<&mut Settle>,
    ) -> Self {
        Self {
            payload: Arc::clone(payload),
            process_id,
            confirmed,
            _settle: settle.map(Settle::token),
        }
    }
}

/// Held by each entry a writer pushed; dropping the last closes the
/// writer's channel. Nothing is ever sent on it.
pub(in crate::server) struct SettleToken {
    _channel: oneshot::Sender<()>,
}

/// What one writer pushed onto the lanes, for its COV fanout to wait on
/// (#889). A writer that pushed nothing has nothing to wait for: no token,
/// no channel, no await.
#[derive(Default)]
pub(in crate::server) struct Settle(Option<(Arc<SettleToken>, oneshot::Receiver<()>)>);

impl Settle {
    fn token(&mut self) -> Arc<SettleToken> {
        let (token, _) = self.0.get_or_insert_with(|| {
            let (sender, receiver) = oneshot::channel();
            (Arc::new(SettleToken { _channel: sender }), receiver)
        });
        Arc::clone(token)
    }

    /// The receiver that resolves once every entry this writer pushed has
    /// left its lane, or `None` when it pushed none.
    pub(in crate::server) fn into_wait(self) -> Option<oneshot::Receiver<()>> {
        self.0.map(|(_, receiver)| receiver)
    }
}

/// Wait for `settle`, at most `timeout_ms` (the APDU timeout), so a change's
/// event notifications are handed off before its COV fanout (#889). When the
/// time runs out the fanout goes on, and the wait is counted in
/// `cov_wait_expired`, so a stalled destination can't hold every writer's
/// request for longer.
pub(in crate::server) async fn settled(
    settle: Option<oneshot::Receiver<()>>,
    timeout_ms: u64,
    suppressions: &EventSuppressions,
) {
    if !settled_within(settle, timeout_ms).await {
        suppressions.record(EventSuppression::CovWaitExpired);
    }
}

/// Wait for `settle`, at most `timeout_ms`: whether it settled in time. For
/// a sender with no COV fanout to order (an acknowledgment, a relay), which
/// waits as its inline sends once did and counts nothing.
pub(in crate::server) async fn settled_within(
    settle: Option<oneshot::Receiver<()>>,
    timeout_ms: u64,
) -> bool {
    let Some(receiver) = settle else {
        return true;
    };
    let timeout = Duration::from_millis(timeout_ms);
    tokio::time::timeout(timeout, receiver).await.is_ok()
}

/// The lanes: each destination's waiting entries, and the tasks that drain
/// them, kept with the server's notification transactions.
#[derive(Default)]
pub(in crate::server) struct EventLanes {
    lanes: Vec<Lane>,
    /// Entries across every lane.
    entries: usize,
    tasks: JoinSet<()>,
    /// Set by `stop()`: nothing more is pushed or drained.
    sealed: bool,
}

struct Lane {
    destination: BACnetRecipient,
    entries: VecDeque<LaneEntry>,
}

/// What [`EventLanes::push`] did with an entry.
enum Pushed {
    /// Behind the lane's other entries.
    Joined,
    /// First in a new lane, whose task the caller starts.
    Opened,
    /// Dropped, as counted.
    Dropped(EventSuppression),
    /// Dropped: the server is stopping.
    Sealed,
}

impl EventLanes {
    fn push(&mut self, destination: &BACnetRecipient, entry: LaneEntry) -> Pushed {
        if self.sealed {
            return Pushed::Sealed;
        }
        if self.entries >= MAX_LANED_ENTRIES {
            return Pushed::Dropped(EventSuppression::LanesEntriesFull);
        }
        match self.position(destination) {
            Some(at) if self.lanes[at].entries.len() >= MAX_LANE_ENTRIES => {
                Pushed::Dropped(EventSuppression::LaneFull)
            }
            Some(at) => {
                self.lanes[at].entries.push_back(entry);
                self.entries += 1;
                Pushed::Joined
            }
            None if self.lanes.len() >= MAX_LANES => Pushed::Dropped(EventSuppression::LanesFull),
            None => {
                self.lanes.push(Lane {
                    destination: destination.clone(),
                    entries: VecDeque::from([entry]),
                });
                self.entries += 1;
                Pushed::Opened
            }
        }
    }

    /// `destination`'s next entry; `None` once its lane is empty, which
    /// closes the lane, so the next push opens it again with a task of its
    /// own.
    fn next(&mut self, destination: &BACnetRecipient) -> Option<LaneEntry> {
        let at = self.position(destination)?;
        let next = self.lanes[at].entries.pop_front();
        match next {
            Some(_) => self.entries -= 1,
            None => drop(self.lanes.swap_remove(at)),
        }
        next
    }

    fn position(&self, destination: &BACnetRecipient) -> Option<usize> {
        self.lanes
            .iter()
            .position(|lane| lane.destination == *destination)
    }

    /// Seal the lanes, dropping what they hold, and abort their tasks. The
    /// tasks stay in the set until joined ([`join_sealed`]), so a `stop()`
    /// cancelled part way leaves them for the next to join. Sealing again
    /// does nothing more.
    pub(in crate::server) fn seal(&mut self) {
        self.sealed = true;
        self.lanes.clear();
        self.entries = 0;
        self.tasks.abort_all();
    }

    /// The sealed lanes' tasks, for a drop with no runtime to join them on:
    /// dropping the set aborts them.
    pub(in crate::server) fn take_tasks(&mut self) -> JoinSet<()> {
        std::mem::take(&mut self.tasks)
    }

    /// Entries across every lane, for tests.
    #[cfg(test)]
    pub(in crate::server) fn waiting(&self) -> usize {
        self.entries
    }

    /// Destinations with a lane, for tests.
    #[cfg(test)]
    pub(in crate::server) fn open(&self) -> usize {
        self.lanes.len()
    }
}

/// The handles the lanes one notification opens share, made once, the first
/// time it opens one: a notification to many idle destinations clones the
/// server's handles once, not once per lane.
pub(in crate::server) struct LaneHandles<T: TransportPort + 'static>(Option<Arc<OwnedDelivery<T>>>);

impl<T: TransportPort + 'static> Default for LaneHandles<T> {
    fn default() -> Self {
        Self(None)
    }
}

/// Join the tasks of `transactions`' sealed lanes, logging any that
/// panicked. Taking the lock only for each poll, never across the await, it
/// can be cancelled and started again.
pub(in crate::server) async fn join_sealed(transactions: &NotificationTransactions) {
    let next = |cx: &mut std::task::Context<'_>| match made_lanes(transactions) {
        Some(mut lanes) => lanes.tasks.poll_join_next(cx),
        None => std::task::Poll::Ready(None),
    };
    while let Some(result) = std::future::poll_fn(next).await {
        observe(result);
    }
}

/// Wait until no lane is open: every notification pushed so far is handed
/// off, or waits for its device's I-Am. For tests whose senders no longer
/// wait on their lanes.
///
/// It yields first, which moves no clock, so a test on paused time that
/// measures a window isn't moved past it; a lane that waits on a timer is
/// then given time, 1 ms at a step.
#[cfg(test)]
pub(in crate::server) async fn lanes_drained(transactions: &NotificationTransactions) {
    for step in 0..2_000 {
        if lanes(transactions).lanes.is_empty() {
            return;
        }
        if step < 100 {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    panic!("a lane is still open");
}

/// Log a lane task that panicked, as it is reaped.
fn observe(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result
        && error.is_panic()
    {
        warn!(%error, "An event notification lane's task panicked");
    }
}

/// `transactions`' lanes, under their lock, made the first time they are
/// needed: one structure, whichever caller gets there first.
pub(in crate::server) fn lanes(
    transactions: &NotificationTransactions,
) -> std::sync::MutexGuard<'_, EventLanes> {
    transactions
        .event_lanes
        .get_or_init(Box::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `transactions`' lanes, under their lock, if anything has made them: a
/// server that never pushed an event notification has none to join.
pub(in crate::server) fn made_lanes(
    transactions: &NotificationTransactions,
) -> Option<std::sync::MutexGuard<'_, EventLanes>> {
    let lanes = transactions.event_lanes.get()?;
    Some(
        lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Push `entry` onto `destination`'s lane, opening the lane, and the task
    /// that drains it, when it has none. Never waits.
    pub(in crate::server) fn push_to_lane(
        ctx: &EventDelivery<'_, T>,
        destination: &BACnetRecipient,
        entry: LaneEntry,
        handles: &mut LaneHandles<T>,
    ) {
        let mut lanes = lanes(ctx.notification_transactions);
        match lanes.push(destination, entry) {
            Pushed::Joined | Pushed::Sealed => {}
            Pushed::Opened => {
                let owned = Arc::clone(
                    handles
                        .0
                        .get_or_insert_with(|| Arc::new(OwnedDelivery::of(ctx))),
                );
                let destination = destination.clone();
                // Lanes come and go; reap the tasks that have finished.
                while let Some(result) = lanes.tasks.try_join_next() {
                    observe(result);
                }
                lanes
                    .tasks
                    .spawn(async move { Self::drain_lane(owned, destination).await });
            }
            Pushed::Dropped(suppression) => {
                drop(lanes);
                ctx.suppressions.record(suppression);
                warn_throttled!(
                    ctx.suppressions.lane_warning,
                    ?suppression,
                    "Event notification lanes full; notification dropped"
                );
            }
        }
    }

    /// Send `destination`'s entries, one at a time, until its lane is empty.
    async fn drain_lane(owned: Arc<OwnedDelivery<T>>, destination: BACnetRecipient) {
        let mut task = LaneTask {
            owned: Arc::clone(&owned),
            destination: destination.clone(),
            closed: false,
        };
        let ctx = owned.delivery();
        loop {
            let Some(entry) = lanes(ctx.notification_transactions).next(&destination) else {
                task.closed = true;
                return;
            };
            // DeviceCommunicationControl drops what it would refuse,
            // uncounted, as it always has.
            if ctx.comm_state.initiation_restricted() {
                continue;
            }
            let payload = &entry.payload;
            let encode_for = |process_identifier| payload.encode_for(process_identifier);
            let outbound = OutboundNotification {
                notification_class: payload.notification_class,
                priority: payload.priority,
                encode_for: &encode_for,
                admits: Arc::clone(&payload.admits),
                budget: payload.budget.clone(),
            };
            let handed_off = Self::send_to_recipient(
                &ctx,
                &outbound,
                &destination,
                entry.process_id,
                entry.confirmed,
            )
            .await;
            // A confirmed entry holds the lane until its first attempt is
            // handed off; its retries run in its worker.
            if let Some(handed_off) = handed_off {
                let _ = handed_off.await;
            }
        }
    }
}

/// A lane's task, which leaves its lane to another when it ends before the
/// lane is empty: a send that panicked, as the reaped task logs, loses the
/// entry it was sending, and a new task takes the rest, so the lane isn't
/// left with no task to drain it. With no runtime to start one on, the lane
/// goes, and its entries with it, which resolves their writers' waits.
/// Nothing is done for lanes that were sealed: `stop()` or the server's drop
/// aborted the task.
struct LaneTask<T: TransportPort + 'static> {
    owned: Arc<OwnedDelivery<T>>,
    destination: BACnetRecipient,
    /// The lane emptied and closed.
    closed: bool,
}

impl<T: TransportPort + 'static> Drop for LaneTask<T> {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let ctx = self.owned.delivery();
        let mut lanes = lanes(ctx.notification_transactions);
        let Some(at) = lanes.position(&self.destination).filter(|_| !lanes.sealed) else {
            return;
        };
        // Said now, not when the task is reaped. Under Tokio a panicking
        // task's state drops after the panic is caught, so `panicking()` is
        // mostly false here; the early end is what shows it.
        warn!(
            destination = ?self.destination,
            panicking = std::thread::panicking(),
            waiting = lanes.lanes[at].entries.len(),
            "An event notification lane's task ended before its lane was empty; \
             the entry it was sending is lost"
        );
        if lanes.lanes[at].entries.is_empty() || tokio::runtime::Handle::try_current().is_err() {
            let lane = lanes.lanes.swap_remove(at);
            lanes.entries -= lane.entries.len();
            drop(lanes);
            drop(lane);
            return;
        }
        let owned = Arc::clone(&self.owned);
        let destination = self.destination.clone();
        lanes
            .tasks
            .spawn(async move { BACnetServer::<T>::drain_lane(owned, destination).await });
    }
}

#[cfg(test)]
#[path = "event_lanes_tests.rs"]
mod tests;
