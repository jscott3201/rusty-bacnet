//! The COV fanout a background commit owes, sent after its guard is dropped,
//! and the follow-up fanout an acknowledged confirmed report owes (#896).
use super::cov_notify_context::CovNotifyContext;
use super::event_delivery::EventDelivery;
use super::*;
use crate::committed_cov::CommittedCov;
use bacnet_device_core::cov::CovSubscriptionKey;

/// Shared handles for fanning COV out from a background task (#889). It uses
/// the write path's post-commit fanout, so the ordering, budget and DCC rules
/// are the same as for a network write.
pub(super) struct CovFanout<T: TransportPort + 'static> {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) cov_table: Arc<RwLock<CovSubscriptionTable>>,
    network: Arc<NetworkLayer<T>>,
    cov_in_flight: Arc<Semaphore>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    comm_state: Arc<CommState>,
    event_suppressions: Arc<super::event_suppression::EventSuppressions>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    config: Arc<ServerConfig>,
}

impl<T: TransportPort + 'static> Clone for CovFanout<T> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            cov_table: Arc::clone(&self.cov_table),
            network: Arc::clone(&self.network),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            notification_transactions: Arc::clone(&self.notification_transactions),
            comm_state: Arc::clone(&self.comm_state),
            event_suppressions: Arc::clone(&self.event_suppressions),
            learned_routers: Arc::clone(&self.learned_routers),
            device_bindings: Arc::clone(&self.device_bindings),
            config: Arc::clone(&self.config),
        }
    }
}

impl<T: TransportPort + 'static> CovFanout<T> {
    /// Own the handles of `ctx`, plus those the background tasks'
    /// EventNotification sends read: the event counters, the learned routers
    /// and the Device bindings.
    pub(super) fn new(
        ctx: &CovNotifyContext<'_, T>,
        event_suppressions: &Arc<super::event_suppression::EventSuppressions>,
        learned_routers: &Arc<Mutex<LearnedRouterCache>>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
    ) -> Self {
        Self {
            db: Arc::clone(ctx.db),
            cov_table: Arc::clone(ctx.cov_table),
            network: Arc::clone(ctx.network),
            cov_in_flight: Arc::clone(ctx.cov_in_flight),
            notification_transactions: Arc::clone(ctx.notification_transactions),
            comm_state: Arc::clone(ctx.comm_state),
            event_suppressions: Arc::clone(event_suppressions),
            learned_routers: Arc::clone(learned_routers),
            device_bindings: Arc::clone(device_bindings),
            config: Arc::clone(ctx.config),
        }
    }

    /// Borrow the handles a COV notification pass reads.
    pub(super) fn notify_context(&self) -> CovNotifyContext<'_, T> {
        CovNotifyContext {
            db: &self.db,
            network: &self.network,
            cov_table: &self.cov_table,
            cov_in_flight: &self.cov_in_flight,
            notification_transactions: &self.notification_transactions,
            comm_state: &self.comm_state,
            config: &self.config,
        }
    }

    /// Borrow the handles an EventNotification send reads. The retry timeout
    /// and APDU capacity come from the server config.
    pub(super) fn event_delivery(&self) -> EventDelivery<'_, T> {
        EventDelivery {
            db: &self.db,
            network: &self.network,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            notification_transactions: &self.notification_transactions,
            device_bindings: &self.device_bindings,
            suppressions: &self.event_suppressions,
            retry_timeout_ms: self.config.cov_retry_timeout_ms,
            local_apdu_capacity: u32::from(self.config.device.max_apdu_length()),
            device: crate::local_device::initiating_device(&self.config.device),
        }
    }

    pub(super) async fn fire(&self, committed: &CommittedCov) {
        if committed.is_empty() {
            return;
        }
        BACnetServer::<T>::fire_post_write_cov_notifications(
            &self.notify_context(),
            &committed.coarse,
            &committed.life_safety,
        )
        .await;
    }

    /// Fan out again each reference whose confirmed report was acknowledged,
    /// so changes held back while it was outstanding reach the subscriber
    /// (#896), each context whose queued timestamped changes reached
    /// their Max_Notification_Delay deadline (#856), and each reference
    /// whose confirmed report found no room, once there is some: an invoke
    /// ID (#2661), a slot of the pool and one of its peer's (#2671). Runs
    /// until the server aborts it.
    pub(super) async fn run_revisits(self) {
        use futures_util::FutureExt;
        let (revisits, timed, in_flight) = {
            let table = self.cov_table.read().await;
            (
                Arc::clone(table.revisits()),
                table.timed().clone(),
                Arc::clone(table.in_flight_tracker()),
            )
        };
        let room = OwedRoom {
            ids: &self.notification_transactions,
            in_flight: &in_flight,
            pool: &self.cov_in_flight,
            per_peer: self.config.cov_policy.max_confirmed_in_flight_per_peer,
        };
        // Set after a look refused for want of room: the next pass waits for
        // room to come back after that look began.
        let mut hold = None;
        // Passes of waiters in a row.
        let mut streak = 0_u32;
        loop {
            // Waiters first, so one beats this task's own follow-up to the
            // room that a report's end freed (#2671). After OWED_STREAK
            // passes of waiters in a row the look yields once, so the
            // follow-ups and the backstop, taken in random order between
            // themselves as before, get a turn however busy the waiters are.
            // No wait loses work when another wins: revisits stay queued, a
            // due context comes back after its spacing, and references owed
            // room keep their places and any hold. Each takes what it
            // returns in the poll that returns it, and none holds a lock.
            let (keys, wakes) = tokio::select! {
                biased;
                (keys, wakes) = owed_look(&revisits, &room, &mut hold, streak >= OWED_STREAK) => {
                    (keys, Some(wakes))
                }
                keys = async {
                    tokio::select! {
                        keys = revisits.next() => keys,
                        keys = timed.next_due() => keys,
                    }
                } => (keys, None),
            };
            streak = if wakes.is_some() {
                streak.saturating_add(1)
            } else {
                0
            };
            let ctx = self.notify_context();
            let batch = BACnetServer::<T>::fire_cov_revisits(&ctx, &keys);
            // A panic ends only this batch; later acknowledgments still follow up.
            if std::panic::AssertUnwindSafe(batch)
                .catch_unwind()
                .await
                .is_err()
            {
                warn!(
                    references = keys.len(),
                    "COV follow-up fanout panicked; those references report on their next fanout"
                );
            }
            // A look refused for want of room after the pass found some
            // (other work took it first) is looked at again once room comes
            // back, not at once, so a wrong estimate can't spin.
            if let Some([released, ended]) = wakes
                && revisits.looked(&keys)
            {
                hold = Some([Box::pin(released), Box::pin(ended)]);
            }
        }
    }
}

/// The waits a pass made before it read the room: an ID released, a
/// confirmed notification in flight ended.
type Wakes<'a> = [tokio::sync::futures::Notified<'a>; 2];

/// Passes of references owed room the revisit task runs in a row before
/// the follow-ups and the backstop get a turn.
const OWED_STREAK: u32 = 8;

/// What lets references owed room go (#2661, #2671): the IDs free, and the
/// confirmed COV notifications in flight, in the pool and per peer.
struct OwedRoom<'a> {
    ids: &'a NotificationTransactions,
    in_flight: &'a crate::cov::CovInFlightTracker,
    pool: &'a Semaphore,
    per_peer: usize,
}

impl OwedRoom<'_> {
    /// One pass's admission, from the room there is now. Each coordinate
    /// (a subscription, or a whole COV-multiple context) takes an ID, a slot
    /// of the pool and one of its peer's, as its report will. One that
    /// can't go keeps its place, and those behind it are still looked at,
    /// so no reservation is tried that would be refused for want of room,
    /// and a peer at its limit holds back none but its own. One held back
    /// costs a lookup and no copy: a busy peer's references may wait
    /// through many passes. Also whether no invoke ID was free, so that
    /// every reference it holds back needs one first.
    fn admit(&self) -> (bool, impl FnMut(&CovSubscriptionKey) -> bool + use<>) {
        let (mut ids, mut pool) = (self.ids.free_invoke_ids(), self.pool.available_permits());
        let no_ids = ids == 0;
        let mut peers = self.in_flight.in_flight();
        // A wait made after this point would miss what the hook frees.
        #[cfg(test)]
        owed_gap::run();
        let per_peer = self.per_peer;
        let mut contexts = std::collections::HashSet::new();
        let admit = move |key: &CovSubscriptionKey| {
            let context = key.multiple_context();
            if context.is_some_and(|context| contexts.contains(context)) {
                return true;
            }
            #[cfg(test)]
            if owed_gap::overtaking() {
                return true;
            }
            if ids == 0 || pool == 0 {
                return false;
            }
            let peer = recipient(key);
            if peers.get(peer).is_some_and(|&count| count >= per_peer) {
                return false;
            }
            *peers.entry(peer.clone()).or_default() += 1;
            ids -= 1;
            pool -= 1;
            contexts.extend(context.cloned());
            true
        };
        (no_ids, admit)
    }
}

/// The peer a reference's confirmed reports count against.
fn recipient(key: &CovSubscriptionKey) -> &bacnet_device_core::cov::CovRecipient {
    match key {
        CovSubscriptionKey::Object { recipient, .. }
        | CovSubscriptionKey::Property { recipient, .. } => recipient,
        CovSubscriptionKey::Multiple { context, .. } => &context.recipient,
    }
}

/// Wait until references owed room can be looked at again, and take those
/// the room lets go, oldest first (#2661, #2671), with the waits the pass
/// made before it read the room. A pass follows a lease ending, a confirmed
/// notification in flight ending, or a reference owed, and takes only what
/// can go, so at a limit it waits rather than spins: a reference held back,
/// or refused again, keeps its place. While none is owed it waits for one
/// to be, and nothing coming back costs anything. A `hold` from a refused
/// look is waited out first, and kept if this is dropped meanwhile. With
/// `yield_first` it yields once before anything, so a caller that polls it
/// first lets its other work go if there is any. Cancel-safe: references
/// are taken only in the poll that returns them.
async fn owed_look<'r>(
    revisits: &crate::cov::CovRevisits,
    room: &OwedRoom<'r>,
    hold: &mut Option<[std::pin::Pin<Box<tokio::sync::futures::Notified<'r>>>; 2]>,
    yield_first: bool,
) -> (Vec<CovSubscriptionKey>, Wakes<'r>) {
    if yield_first {
        tokio::task::yield_now().await;
    }
    if let Some([released, ended]) = hold {
        tokio::select! {
            () = released.as_mut() => {}
            () = ended.as_mut() => {}
        }
        *hold = None;
    }
    loop {
        let owed = revisits.owed_notified();
        if !revisits.owes_any() {
            owed.await;
            continue;
        }
        // Made before the room is read, so an ID or a slot that comes back
        // after it still ends this wait.
        let released = room.ids.released();
        let ended = room.in_flight.ended();
        let (no_ids, admit) = room.admit();
        let keys = revisits.take_owed(admit);
        if !keys.is_empty() {
            return (keys, [released, ended]);
        }
        // None could go. With no ID free each needs one first, and only one
        // coming back can let it go; otherwise each lacks a slot of the pool
        // or of its peer, which only a report ending frees. So a reference
        // held by its peer costs nothing when an event, Audit or read lease
        // ends. Both waits were made before the room was read, and a
        // reference owed meanwhile starts a pass of its own; whatever then
        // changes, that pass reads the room again.
        if no_ids {
            tokio::select! {
                () = released => {}
                () = owed => {}
            }
        } else {
            tokio::select! {
                () = ended => {}
                () = owed => {}
            }
        }
    }
}

/// Test hooks into a pass: one run right after it reads the room, where a
/// lease or a slot coming back must still end the wait the pass made
/// before; and an estimate of the room that admits everything.
#[cfg(test)]
pub(super) mod owed_gap {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static GAP: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
        static OVERTAKING: Cell<bool> = const { Cell::new(false) };
    }

    /// Run `hook` once, after the next pass on this thread reads the room.
    pub(in crate::server) fn set(hook: impl FnOnce() + 'static) {
        GAP.with(|gap| *gap.borrow_mut() = Some(Box::new(hook)));
    }

    /// Have passes on this thread admit every reference, room or not.
    pub(in crate::server) fn overtake() {
        OVERTAKING.with(|overtaking| overtaking.set(true));
    }

    pub(super) fn run() {
        if let Some(hook) = GAP.with(|gap| gap.borrow_mut().take()) {
            hook();
        }
    }

    pub(super) fn overtaking() -> bool {
        OVERTAKING.with(Cell::get)
    }
}
