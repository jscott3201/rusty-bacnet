//! COV reporting for the objects of the session's database (#2490), over
//! bacnet-device-core's [`Reporter`].
//!
//! The session owns a [`CovRole`]: the opt-in
//! ([`EndpointSession::with_cov`]), the `Reporter` its preflight builds, and
//! once it starts a [`CovHost`] and the task that sends what the host queues.
//! The host keeps the `Reporter` under a std mutex, with the invoke IDs its
//! confirmed notifications hold. The responder reaches it for SubscribeCOV
//! and for reads of the Device's Active_COV_Subscriptions; the dispatch task
//! for the answers to its confirmed notifications; the local writes for the
//! changes the application makes.
//!
//! - **Locks.** Every caller takes the database guard first and the host's
//!   mutex after it. The mutex is never held across an await, nor while
//!   waiting for the database, so it can't invert the order.
//! - **Time.** Tokio's clock, mapped onto the core's timeline when the host
//!   starts, so paused-time tests move lifetimes and retries too.
//! - **Invoke IDs.** A confirmed notification leases its ID from the
//!   session's one coordinator, the pool the client role draws from, so the
//!   two roles never use one ID at once; the answer comes back through the
//!   dispatch task's single admission. At most
//!   [`COV_CONFIRMED_IN_FLIGHT`] are out at once, so silent subscribers
//!   can't take every ID from the client role.
//! - **Order.** The first notification of a subscription waits until the
//!   SubscribeCOV's reply has been handed off ([`EndpointCov::replied`]).
//!
//! [`EndpointSession::with_cov`]: super::EndpointSession::with_cov

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_device_core::cov::active::LiveDeviceCov;
use bacnet_device_core::cov::{Delivery, SubscriberEndpoint, Token};
use bacnet_device_core::read::LiveLists;
use bacnet_device_core::report::{Bindings, Destination, InvokeIds, ReportConfig, Reporter};
use bacnet_device_core::responder::{Inbound, Limits, Outcome, Responder};
use bacnet_device_core::write::Applied;
use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_endpoint_core::coordinator::{
    Admission, CanonicalPeer, LeaseMetadata, LeaseToken, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{
    EndpointApduDestination, EndpointEgress, EndpointEgressAdmissionError,
};
use bacnet_network::layer::ReceivedApdu;
use bacnet_npdu::NetworkPriority;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::endpoint_cov::EndpointCov;
use bacnet_time::HostClock;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;

use crate::held_database::HeldDatabase;

/// How long the task waits, at the least, before it looks again at a report
/// owed for want of an invoke ID, unless one comes back sooner.
const STARVED_RETRY: std::time::Duration = std::time::Duration::from_millis(50);

/// The most confirmed COV notifications a session with
/// [`with_cov`](super::EndpointSession::with_cov) has out at once, all
/// subscribers together: a quarter of the 256 invoke IDs its client role
/// draws from too. A silent subscriber holds an ID for each notification
/// until its retries run out, up to `CovPolicy::max_confirmed_in_flight_per_peer`
/// of them (16 by default), so without this bound sixteen silent
/// subscribers could lease every ID and every request of the client role
/// would fail for want of one. With it, four of them fill the bound at the
/// default limits; a confirmed report past it waits, oldest owed first,
/// until one of the notifications out ends, which for a silent subscriber
/// is when its retries run out (12 s at the default timing). Nothing is
/// dropped, and unconfirmed notifications don't count against it, though
/// while the host is at the bound one that found the outbox full goes at
/// the task's next wake (a change, an answer or a timeout), which can be
/// seconds later: size the outbox for the subscriptions expected. Lower
/// `max_confirmed_in_flight_per_peer` when many confirmed subscribers are
/// expected, so that more silent ones are needed to fill the bound.
pub const COV_CONFIRMED_IN_FLIGHT: usize = 64;

/// The session's COV opt-in and, once it starts, what runs it.
#[derive(Default)]
pub(crate) struct CovRole {
    /// What [`EndpointSession::with_cov`](super::EndpointSession::with_cov)
    /// enabled reporting with.
    pub(super) config: Option<ReportConfig>,
    /// The `Reporter` the preflight built, until start hands it to the host.
    reporter: Option<Reporter>,
    host: Option<Arc<CovHost>>,
    task: Option<JoinHandle<()>>,
}

impl CovRole {
    /// Build the `Reporter` for the local Device `device` of `db`: under
    /// the session's identity when it has one, otherwise under one read
    /// from the Device object. Does nothing unless COV is enabled. Refuses
    /// an invalid policy, and an outbox with no entry or no octet, which
    /// could queue nothing.
    pub(super) fn prepare(
        &mut self,
        identity: Option<&DeviceIdentity>,
        db: &ObjectDatabase,
        device: ObjectIdentifier,
    ) -> Result<(), Error> {
        let Some(config) = self.config.clone() else {
            return Ok(());
        };
        config.cov.validate()?;
        if config.outbox.entries == 0 || config.outbox.octets == 0 {
            return Err(Error::Encoding(
                "SubscribeCOV requires an outbox of at least one entry and one octet".into(),
            ));
        }
        let identity = match identity {
            Some(identity) => identity.clone(),
            None => device_identity(db, device)?,
        };
        let responder = Responder::new(identity, Limits::default())?;
        self.reporter = Some(Reporter::new(responder, config)?);
        Ok(())
    }

    /// Start the host over what the preflight built, its notifications
    /// leasing invoke IDs from `coordinator` and leaving through `egress`:
    /// the host, for the responder and the dispatch task. `None` when COV
    /// isn't enabled.
    pub(super) fn start(
        &mut self,
        coordinator: &Arc<OutboundTransactionCoordinator>,
        egress: &EndpointEgress,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Option<Arc<CovHost>> {
        let reporter = self.reporter.take()?;
        let timeout = self.config.as_ref()?.confirmed.timeout;
        let origin = tokio::time::Instant::now().into_std();
        let host = Arc::new(CovHost {
            state: Mutex::new(CovState {
                reporter,
                leases: Leases {
                    coordinator: Arc::clone(coordinator),
                    out: Vec::new(),
                    starved: Starved::No,
                    timeout,
                },
                held: 0,
                closed: false,
            }),
            wake: Notify::new(),
            clock: HostClock::new(origin).reading(|| tokio::time::Instant::now().into_std()),
            coordinator: Arc::clone(coordinator),
            egress: egress.clone(),
            #[cfg(test)]
            passes: std::sync::atomic::AtomicUsize::new(0),
        });
        let db = HeldDatabase::new(Arc::clone(db));
        self.task = Some(tokio::spawn(Arc::clone(&host).run(db)));
        self.host = Some(Arc::clone(&host));
        Some(host)
    }

    /// The running host, for the application's own changes.
    pub(super) fn host(&self) -> Option<&Arc<CovHost>> {
        self.host.as_ref()
    }

    /// Stop sending and give back every invoke ID a notification holds.
    /// What is still queued is dropped.
    pub(super) async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(host) = self.host.take() {
            host.close();
        }
    }

    /// [`stop`](Self::stop) without waiting for the task, for `Drop`.
    pub(super) fn abort(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if let Some(host) = self.host.take() {
            host.close();
        }
    }
}

/// A Device identity for the `Reporter` from the Device object itself, for a
/// session composed without one.
fn device_identity(db: &ObjectDatabase, device: ObjectIdentifier) -> Result<DeviceIdentity, Error> {
    let object = db
        .get(&device)
        .ok_or_else(|| Error::Encoding("SubscribeCOV requires a local Device".into()))?;
    let unsigned = |property| match object.read_property(property, None) {
        Ok(PropertyValue::Unsigned(value)) => u16::try_from(value).ok(),
        _ => None,
    };
    let (Some(vendor), Some(max_apdu)) = (
        unsigned(PropertyIdentifier::VENDOR_IDENTIFIER),
        unsigned(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED),
    ) else {
        return Err(Error::Encoding(
            "SubscribeCOV requires the local Device's Vendor_Identifier and Max_APDU_Length_Accepted"
                .into(),
        ));
    };
    DeviceIdentity::new(device.instance_number(), vendor)?.with_max_apdu(max_apdu)
}

/// The running COV reporting of one session.
pub(crate) struct CovHost {
    state: Mutex<CovState>,
    /// Something was queued, or a reply went: the task looks again.
    wake: Notify,
    clock: HostClock,
    coordinator: Arc<OutboundTransactionCoordinator>,
    egress: EndpointEgress,
    /// How many times the task has woken to work, for the tests.
    #[cfg(test)]
    passes: std::sync::atomic::AtomicUsize,
}

struct CovState {
    reporter: Reporter,
    leases: Leases,
    /// SubscribeCOV replies not yet handed off: nothing is sent meanwhile,
    /// so a first notification never overtakes its reply.
    held: usize,
    /// The session stopped: nothing more goes out.
    closed: bool,
}

/// One notification [`Reporter::pop`] handed out, ready for the egress.
struct Outgoing {
    token: Token,
    apdu: Vec<u8>,
    to: EndpointApduDestination,
    confirmed: bool,
    priority: NetworkPriority,
}

impl CovHost {
    fn lock(&self) -> MutexGuard<'_, CovState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Now on the core's timeline. Read with the mutex held, so the
    /// `Reporter` sees time move forward only.
    fn now(&self) -> bacnet_time::Instant {
        self.clock.now()
    }

    /// Make the application's change to `oid` with `apply` and queue what
    /// it reports: a Life Safety object's values are captured first, so its
    /// notification carries exactly what changed. A change that wrote
    /// nothing reports nothing.
    pub(crate) fn local(
        &self,
        db: &mut ObjectDatabase,
        oid: ObjectIdentifier,
        apply: impl FnOnce(&mut ObjectDatabase) -> Result<Applied, Error>,
    ) -> Result<Applied, Error> {
        let mut state = self.lock();
        state.reporter.capture(db, &[oid]);
        let applied = apply(db);
        let written: &[ObjectIdentifier] = match applied {
            Ok(Applied::Written) => &[oid],
            // Reporting no object drops the capture.
            _ => &[],
        };
        let now = self.now();
        state.reporter.changed(db, written, now);
        drop(state);
        self.wake.notify_one();
        applied
    }

    /// The answer `apdu` the dispatch task admitted: true when it answers a
    /// confirmed notification of this host, which then ends.
    pub(crate) fn answer(&self, admission: &Admission, apdu: &[u8]) -> bool {
        let token = admission.token();
        let mut state = self.lock();
        let Some(to) = state
            .leases
            .out
            .iter()
            .find(|lease| lease.token == token)
            .map(|lease| lease.to.clone())
        else {
            return false;
        };
        let now = self.now();
        let CovState {
            reporter, leases, ..
        } = &mut *state;
        reporter.reply(&to, apdu, leases, now);
        drop(state);
        self.wake.notify_one();
        true
    }

    /// The session stopped: send nothing more and give back every invoke ID
    /// a confirmed notification holds.
    fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        let leases = &mut state.leases;
        for lease in leases.out.drain(..) {
            let _ = leases.coordinator.cancel(lease.token);
        }
    }

    /// The task: wait for something queued or a deadline, then send.
    async fn run(self: Arc<Self>, db: HeldDatabase) {
        loop {
            // Made before `starved` is read, so a lease released in between
            // still wakes the task (Tokio delivers `notify_waiters` to every
            // `Notified` already made, polled or not).
            let released = self.coordinator.released();
            let (deadline, pool) = self.next_wake();
            let due = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            let released = async {
                if pool {
                    released.await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            // No arm takes or waits for a lock (#1498).
            tokio::select! {
                () = self.wake.notified() => {}
                () = due => {}
                () = released => {}
            }
            self.step(&db).await;
        }
    }

    /// When the task should look again, and whether it also waits for an
    /// invoke ID to come back to the session's pool. A held reply waits for
    /// its `replied`, so no deadline then.
    fn next_wake(&self) -> (Option<tokio::time::Instant>, bool) {
        let state = self.lock();
        if state.held > 0 || state.closed {
            return (None, false);
        }
        let leases = &state.leases;
        let bound = leases.starved == Starved::Bound && leases.out.len() >= COV_CONFIRMED_IN_FLIGHT;
        let deadline = if bound {
            // Only one of this host's own notifications ending frees an ID:
            // an answer wakes the task, and its timeout is one of these
            // deadlines. A report owed meanwhile, due again at once, would
            // only be refused again, so it waits for one of them. Not
            // served here until the next wake (#2512): a report owed for
            // outbox room, which shares the `Reporter`'s owed flag with
            // those owed for an ID; the COV-multiple Max_Notification_Delay
            // backstop; and a report's cutoff, which can come up to just
            // under a timeout before its attempt deadline when it waited in
            // the outbox.
            bacnet_time::earliest([state.reporter.table().next_expiry(), leases.next_due()])
        } else {
            state.reporter.next_deadline()
        };
        let deadline = deadline
            .and_then(|at| self.clock.to_std(at))
            .map(tokio::time::Instant::from_std);
        // With the pool empty, a report owed for want of an ID is due again
        // at once; it waits for one to come back instead, or a short while.
        let pool = leases.starved == Starved::Pool;
        let deadline = match deadline {
            Some(at) if pool => Some(at.max(tokio::time::Instant::now() + STARVED_RETRY)),
            deadline => deadline,
        };
        (deadline, pool)
    }

    /// Send what is queued, one notification at a time, telling the
    /// `Reporter` how each send ended, and run each deadline as it comes
    /// due, before the next send: a steady stream of changes can't hold
    /// back a retry or an expiry.
    async fn step(&self, db: &RwLock<ObjectDatabase>) {
        #[cfg(test)]
        self.passes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        loop {
            let due = {
                let state = self.lock();
                let now = self.now();
                !state.closed && state.reporter.next_deadline().is_some_and(|at| at <= now)
            };
            if due {
                let db = db.read().await;
                let mut state = self.lock();
                let now = self.now();
                let CovState {
                    reporter, leases, ..
                } = &mut *state;
                reporter.on_deadline(&db, leases, now);
            }
            let Some(outgoing) = self.pop() else {
                return;
            };
            let delivery = if self.send(&outgoing).await {
                Delivery::Sent
            } else {
                Delivery::NotSent
            };
            let mut state = self.lock();
            let now = self.now();
            let CovState {
                reporter, leases, ..
            } = &mut *state;
            reporter.delivered(outgoing.token, delivery, leases, now);
        }
    }

    /// The next notification to send, unless a reply holds them back.
    fn pop(&self) -> Option<Outgoing> {
        let mut state = self.lock();
        if state.held > 0 || state.closed {
            return None;
        }
        let now = self.now();
        let CovState {
            reporter, leases, ..
        } = &mut *state;
        loop {
            let mut apdu = BytesMut::new();
            let notification = reporter.pop(&NoBindings, leases, now, &mut apdu)?;
            if let Some(id) = notification.invoke_id {
                // Its answer is overdue a timeout from now, as the
                // `Reporter` reckons it, on a first send and a resend alike.
                leases.sent(id, now.saturating_add(leases.timeout));
            }
            let to = match &notification.to {
                Destination::Subscriber(to) => destination(to),
                // Device recipients come with event notifications (#2303).
                _ => {
                    reporter.delivered(notification.token, Delivery::NotSent, leases, now);
                    continue;
                }
            };
            return Some(Outgoing {
                token: notification.token,
                apdu: apdu.to_vec(),
                to,
                confirmed: notification.invoke_id.is_some(),
                priority: notification.priority,
            });
        }
    }

    /// Send `outgoing`, waiting for room in the egress: whether the
    /// transport took it.
    async fn send(&self, outgoing: &Outgoing) -> bool {
        loop {
            let admitted = self.egress.admit_apdu(
                outgoing.apdu.clone(),
                outgoing.to.clone(),
                outgoing.confirmed,
                outgoing.priority,
                Vec::new(),
                None,
            );
            match admitted {
                Ok(send) => return send.complete().await.result.is_ok(),
                Err(EndpointEgressAdmissionError::QueueFull) => {
                    if self.egress.wait_for_capacity().await.is_err() {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
    }
}

/// The route a subscription arrived on: its link MAC, and through a router
/// its routed source.
fn destination(to: &SubscriberEndpoint) -> EndpointApduDestination {
    match &to.network {
        Some(source) => EndpointApduDestination::Routed {
            destination_network: source.network,
            destination_mac: source.mac_address.clone(),
            router_mac: to.mac.clone(),
        },
        None => EndpointApduDestination::Direct {
            destination_mac: to.mac.clone(),
        },
    }
}

impl EndpointCov for CovHost {
    fn subscribe(&self, db: &mut ObjectDatabase, received: &ReceivedApdu) -> Option<Apdu> {
        let peer = SubscriberEndpoint::new(&received.source_mac, received.source_network.as_ref());
        // A SimpleACK, an Error or a Reject: no reply limit applies.
        let request = Inbound::new(received.apdu.clone(), u16::MAX);
        let mut out = BytesMut::new();
        let mut state = self.lock();
        let now = self.now();
        let handled = state.reporter.handle(db, &peer, request, now, &mut out);
        state.held += 1;
        drop(state);
        (handled.outcome == Outcome::Reply)
            .then(|| decode_apdu(out.freeze()).ok())
            .flatten()
    }

    fn replied(&self) {
        let mut state = self.lock();
        state.held = state.held.saturating_sub(1);
        drop(state);
        self.wake.notify_one();
    }

    fn live(&self, db: &ObjectDatabase, selection: LiveLists) -> LiveDeviceCov {
        let entries = {
            let state = self.lock();
            state
                .reporter
                .table()
                .live_cov_entries(selection, self.now())
        };
        LiveDeviceCov::project(db, selection, entries)
    }
}

/// The invoke IDs the `Reporter`'s confirmed notifications hold: leases of
/// the session's coordinator, each kept with the peer it was taken for, at
/// most [`COV_CONFIRMED_IN_FLIGHT`] of them.
struct Leases {
    coordinator: Arc<OutboundTransactionCoordinator>,
    out: Vec<Lease>,
    /// Why the last `take` found no ID.
    starved: Starved,
    /// How long a confirmed notification waits for its answer.
    timeout: bacnet_time::Duration,
}

/// One invoke ID a confirmed notification holds.
struct Lease {
    to: BACnetAddress,
    token: LeaseToken,
    /// When its current attempt's answer is overdue, once it has gone.
    due: Option<bacnet_time::Instant>,
}

/// Why a confirmed report found no invoke ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Starved {
    /// It found one.
    No,
    /// The session's pool was empty: the client role may give one back
    /// at any time.
    Pool,
    /// This host held [`COV_CONFIRMED_IN_FLIGHT`]: only one of its own
    /// notifications ending frees one.
    Bound,
}

impl Leases {
    /// The notification holding `id` went, its answer overdue at `due`.
    fn sent(&mut self, id: u8, due: bacnet_time::Instant) {
        if let Some(lease) = self
            .out
            .iter_mut()
            .find(|lease| lease.token.invoke_id() == id)
        {
            lease.due = Some(due);
        }
    }

    /// When the first answer still awaited is overdue.
    fn next_due(&self) -> Option<bacnet_time::Instant> {
        self.out.iter().filter_map(|lease| lease.due).min()
    }
}

impl InvokeIds for Leases {
    fn take(&mut self, to: &BACnetAddress, service: ConfirmedServiceChoice) -> Option<u8> {
        // At the bound a report waits, as it does for an empty pool.
        if self.out.len() >= COV_CONFIRMED_IN_FLIGHT {
            self.starved = Starved::Bound;
            return None;
        }
        let peer = match to.network_number {
            0 => CanonicalPeer::direct(&to.mac_address),
            network => CanonicalPeer::routed(network, &to.mac_address),
        };
        let reserved = self
            .coordinator
            .reserve(LeaseMetadata::notification(peer, service));
        let Ok(token) = reserved else {
            self.starved = Starved::Pool;
            return None;
        };
        self.starved = Starved::No;
        self.out.push(Lease {
            to: to.clone(),
            token,
            due: None,
        });
        Some(token.invoke_id())
    }

    fn release(&mut self, to: &BACnetAddress, id: u8) {
        let held = |lease: &Lease| lease.to == *to && lease.token.invoke_id() == id;
        if let Some(at) = self.out.iter().position(held) {
            let lease = self.out.swap_remove(at);
            let _ = self.coordinator.release(lease.token);
        }
    }
}

/// No Device recipients: a COV notification goes to its subscriber's route.
struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}

#[cfg(test)]
#[path = "cov_tests.rs"]
mod tests;
