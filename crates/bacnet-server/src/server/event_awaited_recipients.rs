//! Event notifications to a Device recipient the server holds no fresh
//! binding for (#1368).
//!
//! Such a recipient used to be skipped, and counted in
//! `device_recipient_unbound`. Now the server looks for its device through
//! the binding table's probes ([`DeviceLookup`]), as a Command or Channel
//! write in another device does (#1322): one Who-Is limited to the device's
//! instance, at most one a minute per device, shared by everything that
//! misses while it is out, and none while DeviceCommunicationControl
//! restricts initiation.
//!
//! The notification waits for the I-Am, for the probe's bounded wait, the
//! APDU timeout from the Who-Is. A notification's recipients are served one
//! after another, so the wait is not made there. Each device being looked
//! for has a queue of the notifications waiting for it, in the order they
//! were made, and one task in the notification task set that waits on the
//! device's probe and then sends them, one at a time, along the same path as
//! every other recipient ([`BACnetServer::send_on_route`]). While a device's
//! queue holds anything, a new notification for it joins the queue rather
//! than going straight out, so a transition made just after the I-Am can't
//! overtake the ones before it. Each device drains on its own: one that
//! answered doesn't wait for another's probe. The notification's other
//! recipients, and the caller, never wait, and `stop()` aborts the tasks
//! with the other notification workers.
//!
//! The order kept is the order the server makes one device's notifications
//! in, one after another, a probe's wait included. Notifications made at
//! once from different tasks have no order between them, and a confirmed
//! notification's attempts run in a transaction task of its own, so the
//! first attempts of two confirmed ones may go out in either order.
//!
//! At most [`MAX_AWAITING_NOTIFICATIONS`] notifications wait at once, across
//! every device; one more is skipped and counted at once, with no Who-Is.
//!
//! The counters stay as they were for one destination: a recipient whose
//! device stays silent counts once in `device_recipient_unbound`, as does
//! one that can't be looked for (within the hold-off of a fruitless Who-Is,
//! or with too many devices or notifications waiting), and a notification
//! sent after the I-Am counts as any other send does. Nothing is counted for
//! one DCC holds back: DCC is checked again before each send.

use super::super::binding_probes::{
    DeviceLookup, LookupMiss, LookupStart, ProbeWait, can_look_for,
};
use super::super::device_bindings::DeviceResolution;
use super::*;
use std::collections::VecDeque;

/// The most event notifications waiting for their devices' I-Am at once,
/// across every device; one past it is skipped and counted in
/// `device_recipient_unbound` (#1368).
pub(crate) const MAX_AWAITING_NOTIFICATIONS: usize = 1024;

/// How a Device recipient's notification goes on.
pub(super) enum DeviceRoute {
    /// Along the device's binding, or the lack of one, as the table holds
    /// it now.
    Now(DeviceResolution),
    /// It waits in the device's queue for the device's I-Am.
    Queued,
    /// Nothing is sent, and nothing more counted: DCC restricts initiation,
    /// or the notification was skipped and counted here.
    Done,
}

/// One notification waiting for its Device recipient's I-Am: the request,
/// already encoded for the recipient's process identifier, and what the send
/// path needs of the notification it came from.
struct Pending {
    process_id: u32,
    confirmed: bool,
    request: Bytes,
    notification_class: u32,
    priority: u8,
    admits: AdmitsRoute,
    budget: Option<Arc<ForwardingBudget>>,
}

impl Pending {
    /// `outbound` for `process_id`, or `None` for one that doesn't encode,
    /// which the send path never meets for a committed transition.
    fn of(outbound: &OutboundNotification<'_>, process_id: u32, confirmed: bool) -> Option<Self> {
        match (outbound.encode_for)(process_id) {
            Ok(request) => Some(Self {
                process_id,
                confirmed,
                request,
                notification_class: outbound.notification_class,
                priority: outbound.priority,
                admits: Arc::clone(&outbound.admits),
                budget: outbound.budget.clone(),
            }),
            Err(e) => {
                warn!(error = %e, "Failed to encode EventNotification");
                None
            }
        }
    }
}

/// The notifications waiting for their devices' I-Am: one queue per device,
/// kept with the server's notification transactions, under a lock never
/// held across an await.
#[derive(Default)]
pub(in crate::server) struct AwaitingRecipients {
    queues: HashMap<ObjectIdentifier, VecDeque<Pending>>,
    /// Notifications across every queue.
    waiting: usize,
}

/// What [`AwaitingRecipients::enqueue`] did.
enum Enqueued {
    /// Behind the device's waiting notifications.
    Joined,
    /// First in a new queue for the device, whose drain the caller starts.
    Opened,
    /// The device has no queue, and none was to be opened.
    NoQueue,
    /// Not queued: [`MAX_AWAITING_NOTIFICATIONS`] are waiting.
    Full,
}

impl AwaitingRecipients {
    /// Whether notifications wait for `device`.
    fn has_queue(&self, device: &ObjectIdentifier) -> bool {
        self.queues.contains_key(device)
    }

    /// Whether [`MAX_AWAITING_NOTIFICATIONS`] are waiting.
    fn is_full(&self) -> bool {
        self.waiting >= MAX_AWAITING_NOTIFICATIONS
    }

    /// Queue `pending` behind `device`'s waiting notifications, or, with
    /// `open`, first in a new queue when it has none.
    fn enqueue(&mut self, device: ObjectIdentifier, open: bool, pending: Pending) -> Enqueued {
        let opened = !self.has_queue(&device);
        if opened && !open {
            return Enqueued::NoQueue;
        }
        if self.is_full() {
            return Enqueued::Full;
        }
        self.queues.entry(device).or_default().push_back(pending);
        self.waiting += 1;
        if opened {
            Enqueued::Opened
        } else {
            Enqueued::Joined
        }
    }

    /// The next notification waiting for `device`, or `None`, closing its
    /// queue, when none is left.
    fn next(&mut self, device: &ObjectIdentifier) -> Option<Pending> {
        let queue = self.queues.get_mut(device)?;
        let next = queue.pop_front();
        match next {
            Some(_) => self.waiting -= 1,
            None => {
                self.queues.remove(device);
            }
        }
        next
    }

    /// Drop `device`'s queue, its notifications unsent and uncounted.
    fn close(&mut self, device: &ObjectIdentifier) {
        if let Some(queue) = self.queues.remove(device) {
            self.waiting -= queue.len();
        }
    }

    #[cfg(test)]
    pub(in crate::server) fn waiting(&self) -> usize {
        self.waiting
    }
}

/// A device's open queue, owned by the task that drains it. Dropped before
/// the queue is empty (the task aborted by `stop()`, or refused by a closed
/// task set), it closes the queue, so later notifications for the device
/// don't wait on a task that is gone.
struct OpenQueue {
    transactions: Arc<NotificationTransactions>,
    device: ObjectIdentifier,
    closed: bool,
}

impl OpenQueue {
    fn next(&mut self) -> Option<Pending> {
        let next = awaiting(&self.transactions).next(&self.device);
        self.closed = next.is_none();
        next
    }
}

impl Drop for OpenQueue {
    fn drop(&mut self) {
        if !self.closed {
            awaiting(&self.transactions).close(&self.device);
        }
    }
}

fn awaiting(
    transactions: &NotificationTransactions,
) -> std::sync::MutexGuard<'_, AwaitingRecipients> {
    transactions
        .awaiting_recipients
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The handles a device's drain task owns, for an [`EventDelivery`] once the
/// device has answered.
struct OwnedDelivery<T: TransportPort + 'static> {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<T>>,
    comm_state: Arc<CommState>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    notification_transactions: Arc<NotificationTransactions>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    suppressions: Arc<super::super::event_suppression::EventSuppressions>,
    retry_timeout_ms: u64,
    local_apdu_capacity: u32,
}

impl<T: TransportPort + 'static> OwnedDelivery<T> {
    fn of(ctx: &EventDelivery<'_, T>) -> Self {
        Self {
            db: Arc::clone(ctx.db),
            network: Arc::clone(ctx.network),
            comm_state: Arc::clone(ctx.comm_state),
            learned_routers: Arc::clone(ctx.learned_routers),
            notification_transactions: Arc::clone(ctx.notification_transactions),
            device_bindings: Arc::clone(ctx.device_bindings),
            suppressions: Arc::clone(ctx.suppressions),
            retry_timeout_ms: ctx.retry_timeout_ms,
            local_apdu_capacity: ctx.local_apdu_capacity,
        }
    }

    fn delivery(&self) -> EventDelivery<'_, T> {
        EventDelivery {
            db: &self.db,
            network: &self.network,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            notification_transactions: &self.notification_transactions,
            device_bindings: &self.device_bindings,
            suppressions: &self.suppressions,
            retry_timeout_ms: self.retry_timeout_ms,
            local_apdu_capacity: self.local_apdu_capacity,
        }
    }
}

/// The probes of the server `ctx` is, with the APDU timeout as their wait.
fn lookup<'a, T: TransportPort + 'static>(ctx: &EventDelivery<'a, T>) -> DeviceLookup<'a, T> {
    DeviceLookup {
        network: ctx.network,
        bindings: ctx.device_bindings,
        comm_state: ctx.comm_state,
        wait: Duration::from_millis(ctx.retry_timeout_ms),
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// How `outbound` goes on to Device `device`, as `process_id`: after the
    /// notifications already waiting for the device, if it has any; along
    /// its binding; or, with none fresh, in a new queue behind a Who-Is for
    /// it. A device that can't be looked for keeps the resolution it has,
    /// which the send path skips and counts.
    pub(super) async fn device_route(
        ctx: &EventDelivery<'_, T>,
        outbound: &OutboundNotification<'_>,
        device: ObjectIdentifier,
        process_id: u32,
        confirmed: bool,
    ) -> DeviceRoute {
        // The request is encoded only for a notification that waits, and
        // before the lock, which is held for nothing but the queue itself.
        let pending = || Pending::of(outbound, process_id, confirmed);
        let awaiting = || awaiting(ctx.notification_transactions);
        // A device with notifications waiting takes this one after them.
        if awaiting().has_queue(&device) {
            let Some(pending) = pending() else {
                return DeviceRoute::Done;
            };
            let enqueued = awaiting().enqueue(device, false, pending);
            // A queue that closed meanwhile leaves the device to the path
            // below, as one with none.
            if !matches!(enqueued, Enqueued::NoQueue) {
                return Self::queued(ctx, enqueued, device, None);
            }
        }
        let lookup = lookup(ctx);
        let resolution = lookup.resolve(device).await;
        if !can_look_for(device, &resolution) {
            return DeviceRoute::Now(resolution);
        }
        // With every place taken, the notification can only be skipped, so
        // no Who-Is goes out for it.
        if awaiting().is_full() {
            return Self::queued(ctx, Enqueued::Full, device, None);
        }
        let wait = match lookup.start(device).await {
            LookupStart::Resolved(resolution) => return DeviceRoute::Now(resolution),
            LookupStart::NotLooking => return DeviceRoute::Now(resolution),
            LookupStart::Disabled => return DeviceRoute::Done,
            LookupStart::Waiting(wait) => wait,
        };
        let Some(pending) = pending() else {
            return DeviceRoute::Done;
        };
        // Another notification may have opened the device's queue while the
        // Who-Is went out; this one then joins it.
        let enqueued = awaiting().enqueue(device, true, pending);
        Self::queued(ctx, enqueued, device, Some(wait))
    }

    /// The route of a notification [`AwaitingRecipients::enqueue`] took, and
    /// the drain of a queue it opened, waiting on `wait`.
    fn queued(
        ctx: &EventDelivery<'_, T>,
        enqueued: Enqueued,
        device: ObjectIdentifier,
        wait: Option<ProbeWait>,
    ) -> DeviceRoute {
        match (enqueued, wait) {
            (Enqueued::Joined, _) => DeviceRoute::Queued,
            (Enqueued::Opened, Some(wait)) => {
                Self::drain(ctx, device, wait);
                DeviceRoute::Queued
            }
            (Enqueued::Full, _) => {
                ctx.suppressions
                    .record(EventSuppression::DeviceRecipientUnbound);
                warn!(
                    %device,
                    cap = MAX_AWAITING_NOTIFICATIONS,
                    "Skipping Device recipient: too many notifications wait for their devices"
                );
                DeviceRoute::Done
            }
            // Opening takes a probe to wait on, and a queue never opens
            // without one.
            (Enqueued::Opened | Enqueued::NoQueue, _) => DeviceRoute::Done,
        }
    }

    /// Start the task that drains `device`'s queue: it waits on the probe,
    /// then sends each waiting notification in turn to the device, if it
    /// answered, or counts it once, until the queue is empty.
    fn drain(ctx: &EventDelivery<'_, T>, device: ObjectIdentifier, wait: ProbeWait) {
        let owned = OwnedDelivery::of(ctx);
        let mut queue = OpenQueue {
            transactions: Arc::clone(ctx.notification_transactions),
            device,
            closed: false,
        };
        ctx.notification_transactions.spawn(async move {
            wait.answered().await;
            let ctx = owned.delivery();
            let lookup = lookup(&ctx);
            while let Some(pending) = queue.next() {
                // A notification DCC holds back is dropped, uncounted.
                if ctx.comm_state.initiation_restricted() {
                    continue;
                }
                let Pending {
                    process_id,
                    confirmed,
                    request,
                    notification_class,
                    priority,
                    admits,
                    budget,
                } = pending;
                match lookup.found(device).await {
                    Ok(resolution) => {
                        let network = ctx.network;
                        let route = RecipientRoute::from_device_resolution(resolution).localize(
                            network.local_network_number().get(),
                            |mac| network.transport().is_broadcast_mac(mac),
                            |mac| network.transport().is_group_destination(mac),
                        );
                        let encode_for = |_| Ok(request.clone());
                        let outbound = OutboundNotification {
                            notification_class,
                            priority,
                            encode_for: &encode_for,
                            admits,
                            budget,
                        };
                        Self::send_on_route(&ctx, &outbound, route, process_id, confirmed).await;
                    }
                    Err(LookupMiss::Undiscovered) => {
                        ctx.suppressions
                            .record(EventSuppression::DeviceRecipientUnbound);
                        warn!(
                            notification_class,
                            %device,
                            "Skipping Device recipient: its device didn't answer a Who-Is"
                        );
                    }
                    Err(LookupMiss::Disabled) => {}
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "event_awaited_recipients_tests.rs"]
mod tests;
