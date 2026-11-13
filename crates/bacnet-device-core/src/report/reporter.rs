//! [`Reporter`]: the `Responder` with COV reporting, for a host with nothing
//! to await between steps.

use alloc::vec::Vec;

use bacnet_encoding::apdu::{
    Apdu, ConfirmedRequest, SimpleAck, decode_apdu, encode_apdu, max_apdu_header_at_or_below,
};
use bacnet_objects::database::ObjectDatabase;
use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::{Bytes, BytesMut};

use super::confirmed::ConfirmedTable;
use super::outbox::Outbox;
use super::{InvokeIds, NotifyService, ReportConfig, ReporterCounters};
use crate::clock::{Basis, DeviceClock, Zone};
use crate::cov::active::ActiveCovOverlay;
use crate::cov::life_safety::{LifeSafetyCovSnapshots, is_life_safety_object};
use crate::cov::{
    CovCounters, CovNotificationKind, CovReport, CovSubscriptionKey, CovSubscriptionSnapshot,
    CovTable, Delivery, Evaluation, EventBudget, FlightTiming, SubscriberEndpoint, Token, evaluate,
};
use crate::reply::{self, SEGMENTED, emit};
use crate::responder::{Access, Inbound, Outcome, Responder, Written};
use crate::write::{WriteCommitObserver, WriteTarget};

#[cfg(feature = "event-reporting")]
#[path = "reporter_events.rs"]
mod events;
#[path = "reporter_ids.rs"]
mod ids;
#[path = "reporter_local.rs"]
mod local;
#[cfg(feature = "cov-multiple")]
#[path = "reporter_multiple.rs"]
mod multiple;
#[path = "reporter_pop.rs"]
mod pop;
#[path = "reporter_tests.rs"]
#[cfg(test)]
mod tests;

pub use local::LocalValue;

/// The [`Responder`] with COV reporting: SubscribeCOV, SubscribeCOVProperty
/// and, with the `cov-multiple` feature, SubscribeCOVPropertyMultiple, the
/// notifications what a request or the host changes calls for, confirmed
/// delivery with its retries, and the Device's subscription lists. It
/// strings together the staged steps `bacnet-server` runs under its
/// guards; the parity tests hold the two to the same notifications.
///
/// The host hands it each request ([`handle`](Self::handle)), each answer to
/// a confirmed notification ([`reply`](Self::reply)), the application's own
/// changes, which it makes and reports ([`apply_local`](Self::apply_local),
/// [`update`](Self::update), [`set_reliability`](Self::set_reliability),
/// [`command`](Self::command), [`relinquish`](Self::relinquish)), the
/// objects the host changes by other means ([`changed`](Self::changed)) or
/// deletes ([`remove_for_object`](Self::remove_for_object)), and the time whenever
/// [`next_deadline`](Self::next_deadline) comes ([`on_deadline`](Self::on_deadline)).
/// It sends what [`pop`](Self::pop) hands out, and reports how each send
/// ended ([`delivered`](Self::delivered)).
#[derive(Debug)]
pub struct Reporter {
    responder: Responder,
    device: ObjectIdentifier,
    /// The Max-APDU-Length-Accepted a confirmed notification states.
    max_apdu: u16,
    table: CovTable,
    timing: FlightTiming,
    outbox: Outbox,
    confirmed: ConfirmedTable,
    /// References owed a fresh look now: an Ack held their changes back,
    /// or a fence left a report's outcome unknown.
    looks: Vec<CovSubscriptionKey>,
    /// References whose report found no room, oldest first, looked at
    /// again in that order as entries free. One refused again keeps its
    /// place.
    owed: Vec<CovSubscriptionKey>,
    /// An entry left the outbox since the owed references were last looked
    /// at.
    room_freed: bool,
    /// References whose confirmed report found no invoke ID, oldest first,
    /// looked at again only as IDs come back (#2512).
    owed_ids: Vec<CovSubscriptionKey>,
    /// Invoke IDs given back since those were last looked at: this
    /// reporter's own, and those the host reports
    /// ([`invoke_ids_released`](Self::invoke_ids_released)).
    ids_freed: usize,
    /// References [`look`](Self::look) evaluated, for the tests.
    #[cfg(test)]
    looked: usize,
    /// Invoke IDs of fenced notifications, given back at the next call
    /// that has the host's [`InvokeIds`].
    releases: Vec<(BACnetAddress, u8)>,
    /// Life Safety values the host captured before changing them.
    snapshots: LifeSafetyCovSnapshots,
    counters: ReporterCounters,
    /// The latest `now` the host gave.
    last: Instant,
    /// The COV-multiple path: its timestamped histories, and the reports
    /// going out.
    #[cfg(feature = "cov-multiple")]
    multiple: multiple::Multiple,
    /// The event path's schedule, when it is on.
    #[cfg(feature = "event-reporting")]
    events: Option<events::Events>,
}

impl Reporter {
    /// A reporter answering requests with `responder`, under `config`, at
    /// first holding no subscription. Its outbox is allocated here. Refused
    /// when the identity's Max_APDU_Length_Accepted encodes no header, or
    /// when `config` asks for a fault pass without a sweep.
    pub fn new(responder: Responder, config: ReportConfig) -> Result<Self, Error> {
        let identity = responder.identity();
        let device = identity.device_oid();
        let max_apdu = max_apdu_header_at_or_below(u32::from(identity.max_apdu_length()))?;
        #[cfg(feature = "event-reporting")]
        if config.fault_detection.is_some() && config.sweep.is_none() {
            // The pass would report its changes to COV, and the FAULT
            // transitions would wait for a sweep that never runs.
            return Err(Error::Encoding(
                "a fault pass needs the event path: set a sweep too".into(),
            ));
        }
        #[cfg(feature = "cov-multiple")]
        let multiple = multiple::Multiple::new(usize::from(identity.max_apdu_length()));
        #[cfg(feature = "event-reporting")]
        let events = events::Events::new(&config);
        Ok(Self {
            responder,
            device,
            max_apdu,
            table: CovTable::new(config.cov).with_flight_timing(config.confirmed),
            timing: config.confirmed,
            outbox: Outbox::new(config.outbox),
            confirmed: ConfirmedTable::default(),
            looks: Vec::new(),
            owed: Vec::new(),
            room_freed: false,
            owed_ids: Vec::new(),
            ids_freed: 0,
            #[cfg(test)]
            looked: 0,
            releases: Vec::new(),
            snapshots: LifeSafetyCovSnapshots::default(),
            counters: ReporterCounters::default(),
            last: Instant::ZERO,
            #[cfg(feature = "cov-multiple")]
            multiple,
            #[cfg(feature = "event-reporting")]
            events,
        })
    }

    /// The subscription table.
    pub fn table(&self) -> &CovTable {
        &self.table
    }

    /// The Device's clock, which the `Responder` this reporter was built
    /// from was given ([`Responder::with_clock`]), if any. Its
    /// [`window`](crate::clock::SyncedClock::window) is the one to judge
    /// recipient windows by: the database's clock frame reads a valid date
    /// before the clock is synchronized.
    pub fn clock(&self) -> Option<&DeviceClock> {
        self.responder.clock()
    }

    /// Set the Device's clock from the host's own real-time clock, as
    /// [`Responder::set_time`] does: lent to `db` again, and counted
    /// synchronized. Refused, changing nothing, when the reporter's
    /// `Responder` has no clock or the value isn't a specific date and time.
    pub fn set_time(
        &mut self,
        db: &mut ObjectDatabase,
        basis: Basis,
        date: Date,
        time: Time,
    ) -> Result<(), Error> {
        self.responder.set_time(db, basis, date, time)
    }

    /// Move the Device's clock to `zone`, as [`Responder::set_zone`] does:
    /// lent to `db` again, and still synchronized if it was, so
    /// [`clock`](Self::clock)'s window follows the new local time. Refused,
    /// changing nothing, when the reporter's `Responder` has no clock or
    /// the clock refuses the zone.
    pub fn set_zone(&mut self, db: &mut ObjectDatabase, zone: Zone) -> Result<(), Error> {
        self.responder.set_zone(db, zone)
    }

    /// Run `request` from `peer` against `db`, writing any answer to `out`
    /// for the host to send before anything [`pop`](Self::pop) hands out.
    ///
    /// SubscribeCOV, SubscribeCOVProperty and, with the `cov-multiple`
    /// feature, SubscribeCOVPropertyMultiple are answered here, and an
    /// accepted subscription queues its first notification. Every other
    /// request is the `Responder`'s, as [`Responder::handle`] runs it, with
    /// its reads answering the Device's subscription lists from the table
    /// ([`overlay`](Self::overlay)); a write queues the notifications its
    /// changes call for, a Life Safety object's from the values it changed.
    /// `peer` is the route the request came on: its link MAC, and the NPDU
    /// source when it came through a router.
    pub fn handle(
        &mut self,
        db: &mut ObjectDatabase,
        peer: &SubscriberEndpoint,
        request: Inbound,
        now: Instant,
        out: &mut BytesMut,
    ) -> Written {
        self.last = now;
        let apdu = request.apdu.clone();
        #[cfg(feature = "event-reporting")]
        if let Some(written) = self.event_service(db, &request, now, out) {
            return written;
        }
        if let Some(service) = subscribe_service(&apdu) {
            #[cfg(feature = "cov-multiple")]
            if service == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE {
                return self.subscribe_multiple(db, peer, request, now, out);
            }
            return self.subscribe(db, peer, service, request, now, out);
        }
        let handled = if Responder::classify(&apdu) == Access::Write {
            let mut capture = Capture::default();
            #[cfg(not(feature = "cov-multiple"))]
            let written = self.responder.write(db, Some(&mut capture), request, out);
            // Each attempt's timestamped changes are captured as it
            // commits, around the Life Safety capture.
            #[cfg(feature = "cov-multiple")]
            let written = {
                use crate::cov::timed_capture::{TimedCapture, TimedWriteCapture};
                let queue = (&mut self.multiple.histories, now);
                let all = TimedCapture::all(&self.table, now);
                let mut timed = TimedWriteCapture::new(all, queue, Some(&mut capture));
                let written = self.responder.write(db, Some(&mut timed), request, out);
                // Changes the request's exact fanout may not select are
                // looked at again, ahead of the backstop.
                self.looks.extend_from_slice(timed.life_safety_queued());
                written
            };
            self.after_change(db, &written.objects, &capture.0, Tail::Request, now);
            written
        } else {
            let overlay = self.overlay(db, now);
            self.responder.read(db, Some(&overlay), request, out).into()
        };
        match handled.outcome {
            Outcome::Unhandled => reply::refuse(&apdu, out).into(),
            _ => handled,
        }
    }

    /// Before the host changes `objects` itself, capture the Life Safety
    /// ones' COV values, so [`changed`](Self::changed) reports exactly what
    /// the change did, as a write through [`handle`](Self::handle) does.
    pub fn capture(&mut self, db: &ObjectDatabase, objects: &[ObjectIdentifier]) {
        for &object in objects {
            self.snapshots.capture(db, object);
        }
    }

    /// The host changed `objects` itself (a sensor's sample, a value it
    /// computes): queue what that reports. With the event path on, each
    /// object's intrinsic reporting is evaluated first, as after a write,
    /// and its event notifications queued ahead of the COV ones.
    pub fn changed(&mut self, db: &mut ObjectDatabase, objects: &[ObjectIdentifier], now: Instant) {
        self.last = now;
        let snapshots = core::mem::take(&mut self.snapshots);
        self.after_change(db, objects, &snapshots, Tail::Host, now);
    }

    /// The tail every change runs, whichever entry it came through: a
    /// request's write ([`handle`](Self::handle)), the host's change
    /// ([`changed`](Self::changed), and so the local writes), or a
    /// background pass of the event path. Out of line, so the entries
    /// share one copy (#2524).
    ///
    /// With the event path on, `objects`' intrinsic reporting is evaluated
    /// first and the notifications it fires queued, unless a pass committed
    /// them already (`Tail::Committed`). With `cov-multiple` the
    /// timestamped references then take the changes. Last, `objects` are
    /// reported to COV, a Life Safety one held in `snapshots` from the
    /// values it changed, so a committed transition's Status_Flags change
    /// follows its notifications (#889).
    #[inline(never)]
    fn after_change(
        &mut self,
        db: &mut ObjectDatabase,
        objects: &[ObjectIdentifier],
        snapshots: &LifeSafetyCovSnapshots,
        tail: Tail,
        now: Instant,
    ) {
        #[cfg(feature = "event-reporting")]
        let committed = if tail == Tail::Committed {
            Vec::new()
        } else {
            self.object_events(db, objects, now)
        };
        #[cfg(not(feature = "event-reporting"))]
        let committed: [ObjectIdentifier; 0] = [];
        #[cfg(feature = "cov-multiple")]
        if tail == Tail::Request {
            // A request's attempts were captured as each committed; a
            // transition committed since changes Status_Flags at its own
            // time, for the timestamped references, as the server captures it.
            self.capture_changed(db, &committed, &LifeSafetyCovSnapshots::default(), now);
        } else {
            self.capture_changed(db, objects, snapshots, now);
        }
        #[cfg(not(feature = "cov-multiple"))]
        let _ = (committed, tail);
        self.report_changes(db, objects, snapshots, now);
    }

    /// The host deleted `object` from its database: drop its subscriptions,
    /// as the server does at DeleteObject, with their queued notifications,
    /// the looks they were owed and any [`capture`](Self::capture) of the
    /// object, and end each confirmed notification reporting it that is
    /// still out, giving its invoke ID back to `ids`. A later answer to one
    /// of those matches nothing, as long as `ids` hands its invoke ID out
    /// again only in turn ([`InvokeIds::take`]). [`pop`](Self::pop) hands
    /// out nothing more for `object`, though one it already handed out is
    /// still the host's to send, and an object created later under its
    /// identifier starts with no subscription.
    ///
    /// [`handle`](Self::handle) rejects DeleteObject as an unrecognized
    /// service, as the [`Responder`] runs none. A host that serves it
    /// takes the request before `handle`, deletes the object, calls this,
    /// and lists DeleteObject in its own Protocol_Services_Supported
    /// ([`SERVICES`](super::SERVICES) doesn't). A host that removes an
    /// object on its own calls this too. Removing an object with no
    /// subscription, or removing one twice, does nothing.
    pub fn remove_for_object(&mut self, object: ObjectIdentifier, ids: &mut dyn InvokeIds) {
        while let Some(out) = self.confirmed.take_object(object) {
            self.releases.push((out.peer, out.invoke_id));
        }
        self.table.remove_for_object(object);
        self.settle();
        self.looks.retain(|key| key.object() != object);
        self.snapshots.forget(object);
        self.release(ids);
    }

    /// An answer `from` a peer: true when it answers a confirmed
    /// notification this reporter sent, which then ends. A SimpleACK
    /// commits what the notification carried and owes its references a
    /// fresh look at the next [`on_deadline`](Self::on_deadline); an Error,
    /// Reject or Abort holds its subscription off for one retry cycle.
    pub fn reply(
        &mut self,
        from: &BACnetAddress,
        apdu: &[u8],
        ids: &mut dyn InvokeIds,
        now: Instant,
    ) -> bool {
        self.last = now;
        self.release(ids);
        let ours = |service| {
            service == NotifyService::Cov.confirmed()
                || service == NotifyService::CovMultiple.confirmed()
                || service == NotifyService::Event.confirmed()
        };
        let (invoke_id, outcome) = match decode_apdu(Bytes::copy_from_slice(apdu)) {
            Ok(Apdu::SimpleAck(ack)) if ours(ack.service_choice) => {
                (ack.invoke_id, Delivery::Acknowledged)
            }
            Ok(Apdu::Error(error)) if ours(error.service_choice) => {
                (error.invoke_id, Delivery::Failed)
            }
            Ok(Apdu::Reject(reject)) => (reject.invoke_id, Delivery::Failed),
            Ok(Apdu::Abort(abort)) if abort.sent_by_server => (abort.invoke_id, Delivery::Failed),
            _ => return false,
        };
        let Some(out) = self.confirmed.answered(from, invoke_id) else {
            return false;
        };
        self.give_back(ids, &out.peer, out.invoke_id);
        self.end_confirmed(out.token, outcome, now);
        true
    }

    /// When the host should call [`on_deadline`](Self::on_deadline): the
    /// earliest lifetime, report deadline or overdue answer, the earliest
    /// Max_Notification_Delay deadline of a COV-multiple context with
    /// changes pending, the next sweep or fault pass of the event path, or
    /// the latest `now` it gave when references are owed a look.
    pub fn next_deadline(&self) -> Option<Instant> {
        let looks = !self.looks.is_empty()
            || (!self.owed.is_empty() && self.room_freed)
            || (self.owes_invoke_id() && self.ids_freed > 0);
        let resend = self.confirmed.resend_due();
        bacnet_time::earliest([
            self.table.next_expiry(),
            self.confirmed.next_deadline(),
            (looks || resend).then_some(self.last),
            #[cfg(feature = "cov-multiple")]
            self.multiple_deadline(),
            #[cfg(feature = "event-reporting")]
            self.events.as_ref().and_then(events::Events::deadline),
        ])
    }

    /// The time `now` has come: expire lifetimes and abandoned reports,
    /// time out unanswered notifications (to resend, or failed), run the
    /// event path's sweep and fault pass when due, and look again at the
    /// references owed one.
    pub fn on_deadline(&mut self, db: &mut ObjectDatabase, ids: &mut dyn InvokeIds, now: Instant) {
        self.expire_and_time_out(ids, now);
        #[cfg(feature = "event-reporting")]
        self.event_deadline(db, now);
        self.look_owed(db, now);
    }

    /// [`on_deadline`](Self::on_deadline) over a shared database, for a
    /// reporter with no event path (no [`ReportConfig`] sweep), whose
    /// deadlines commit nothing: a host that serves reads meanwhile, as the
    /// endpoint does, needn't take the database exclusively. A reporter
    /// with the event path on refuses it, doing nothing: its sweep and
    /// fault pass need [`on_deadline`](Self::on_deadline), and a host that
    /// went on calling this would find the deadline still due.
    pub fn on_cov_deadline(
        &mut self,
        db: &ObjectDatabase,
        ids: &mut dyn InvokeIds,
        now: Instant,
    ) -> Result<(), Error> {
        #[cfg(feature = "event-reporting")]
        if self.events.is_some() {
            return Err(Error::Encoding(
                "the event path's deadlines need on_deadline".into(),
            ));
        }
        self.expire_and_time_out(ids, now);
        self.look_owed(db, now);
        Ok(())
    }

    /// Expire lifetimes and abandoned reports, and time out unanswered
    /// notifications, at `now`.
    fn expire_and_time_out(&mut self, ids: &mut dyn InvokeIds, now: Instant) {
        self.last = now;
        self.release(ids);
        self.table.expire(now);
        self.settle();
        for out in self.confirmed.time_out(now) {
            self.give_back(ids, &out.peer, out.invoke_id);
            self.end_confirmed(out.token, Delivery::Failed, now);
        }
    }

    /// Look again at the references owed a look at `now`.
    fn look_owed(&mut self, db: &ObjectDatabase, now: Instant) {
        let mut keys = core::mem::take(&mut self.looks);
        // Contexts whose pending changes reached their deadline.
        #[cfg(feature = "cov-multiple")]
        keys.extend(self.multiple_due(now));
        // As many references owed an ID as IDs came back, ahead of those
        // owed room; one refused again waits for the next.
        // Held event notifications need none of these: every `pop` tries
        // them first, and with IDs counted per peer, reserving for them
        // would hold a reference to one peer back behind another's.
        let freed = core::mem::take(&mut self.ids_freed).min(self.owed_ids.len());
        keys.extend_from_slice(&self.owed_ids[..freed]);
        let mut owed = 0;
        if core::mem::take(&mut self.room_freed) {
            // The oldest owed references, as many as there are free
            // entries: more would only be refused again. They stay owed
            // until a look reaches them.
            owed = self.outbox.room().entries_free().min(self.owed.len());
            keys.extend_from_slice(&self.owed[..owed]);
        }
        self.look(db, &keys, now);
        // A look that settled an owed reference, with an entry still free,
        // gives the next ones their turn now rather than at the next pop.
        // One that settled none waits for a pop, so it can't spin.
        let settled = keys[keys.len() - owed..]
            .iter()
            .any(|key| !self.owed.contains(key));
        // `|=`: room a fence freed during the look stays freed.
        self.room_freed |=
            settled && !self.owed.is_empty() && self.outbox.room().entries_free() > 0;
    }

    /// How the send of the notification `token` names ended. An
    /// unconfirmed one that was [`Sent`](Delivery::Sent) becomes its
    /// subscription's baseline. A confirmed one ends through
    /// [`reply`](Self::reply) or its timeout; the host reports one here
    /// only when it couldn't send it ([`NotSent`](Delivery::NotSent)), and
    /// it then ends with no hold-off.
    ///
    /// With `cov-multiple`, call this for every unconfirmed notification
    /// [`pop`](Self::pop) hands out, whichever way its send ended. A
    /// report to a COV-multiple context holds the context's send turn
    /// until the last of its notifications is reported here: one reported
    /// sent retires the changes it carried, and one that was not sent
    /// stops the report, so the notifications queued after it are not
    /// handed out and their changes wait for the context's
    /// Max_Notification_Delay. A notification the host never reports on
    /// keeps the turn, and every later change of that context from being
    /// queued, until its reference leaves the table: no deadline ends it,
    /// as a confirmed notification's does.
    pub fn delivered(
        &mut self,
        token: Token,
        outcome: Delivery,
        ids: &mut dyn InvokeIds,
        now: Instant,
    ) {
        self.last = now;
        self.release(ids);
        if self.confirmed.holds(token) {
            if outcome == Delivery::Sent {
                return;
            }
            let out = self.confirmed.take(token).expect("held above");
            self.give_back(ids, &out.peer, out.invoke_id);
            self.end_confirmed(token, outcome, now);
            return;
        }
        // An unconfirmed event notification leaves no baseline.
        if token.is_event() {
            return;
        }
        // One part of an unconfirmed COV-multiple report.
        #[cfg(feature = "cov-multiple")]
        if self.part_delivered(token, outcome, now) {
            self.settle();
            return;
        }
        // One not sent that a change was measured from owes a look (#2508).
        let owed = self.table.delivered(token, outcome, now);
        self.looks.extend(owed);
        self.settle();
    }

    /// The Device's subscription lists in `db`, live at `now`, for a read
    /// the host runs itself. Without the `cov-multiple` feature
    /// Active_COV_Multiple_Subscriptions is the Device's own: the reporter
    /// then runs no SubscribeCOVPropertyMultiple, so its table holds no
    /// reference that list would show.
    pub fn overlay<'a>(
        &'a self,
        db: &'a ObjectDatabase,
        now: Instant,
    ) -> ActiveCovOverlay<'a, { cfg!(feature = "cov-multiple") }> {
        ActiveCovOverlay::new(&self.table, db, self.device, now)
    }

    /// What this reporter has counted.
    pub fn counters(&self) -> ReporterCounters {
        let mut counters = self.counters.clone();
        counters.cov.subscriptions_active = self.table.len() as u64;
        counters
    }

    /// SubscribeCOV or SubscribeCOVProperty: the reply, then the first
    /// notification of an accepted subscription.
    fn subscribe(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        service: ConfirmedServiceChoice,
        request: Inbound,
        now: Instant,
        out: &mut BytesMut,
    ) -> Written {
        let Ok(Apdu::ConfirmedRequest(confirmed)) = decode_apdu(request.apdu) else {
            return Outcome::Silent.into();
        };
        let accepted = if service == ConfirmedServiceChoice::SUBSCRIBE_COV {
            self.table
                .subscribe_cov(db, peer, &confirmed.service_request, now)
        } else {
            self.table
                .subscribe_cov_property(db, peer, &confirmed.service_request, now)
        };
        let answer = match &accepted {
            Ok(_) => Apdu::SimpleAck(SimpleAck {
                invoke_id: confirmed.invoke_id,
                service_choice: service,
            }),
            Err(error) => reply::error_apdu(confirmed.invoke_id, service, error),
        };
        let outcome = emit(out, &answer, Outcome::Reply);
        self.settle();
        if let Ok(Some(subscription)) = accepted
            && let Some(object) = db.get(&subscription.monitored_object_identifier)
        {
            self.report(object, &[subscription], true, now);
        }
        outcome.into()
    }

    /// Queue what a change of `objects` reports: a Life Safety object held
    /// in `snapshots` from the values it changed, the rest by comparison.
    fn report_changes(
        &mut self,
        db: &ObjectDatabase,
        objects: &[ObjectIdentifier],
        snapshots: &LifeSafetyCovSnapshots,
        now: Instant,
    ) {
        for &object in objects.iter().filter(|&&oid| !snapshots.holds(oid)) {
            let subs = self.singles(object, now);
            if let Some(stored) = db.get(&object).filter(|_| !subs.is_empty()) {
                self.report(stored, &subs, false, now);
            }
            #[cfg(feature = "cov-multiple")]
            self.report_multiples(db, object, None, now);
        }
        for change in snapshots.changes(db, objects) {
            let subs: Vec<_> = self
                .singles(change.object_identifier, now)
                .into_iter()
                .filter(|sub| change.notifies(sub))
                .collect();
            if let Some(stored) = db
                .get(&change.object_identifier)
                .filter(|_| !subs.is_empty())
            {
                self.report(stored, &subs, change.status_changed(), now);
            }
            #[cfg(feature = "cov-multiple")]
            self.report_multiples(db, change.object_identifier, Some(&change), now);
        }
    }

    /// The live ordinary and Single subscriptions to `object`, after the
    /// expiry pass.
    fn singles(&mut self, object: ObjectIdentifier, now: Instant) -> Vec<CovSubscriptionSnapshot> {
        let subs = self
            .table
            .subscriptions_for(&object, now)
            .into_iter()
            .filter(|sub| sub.notification_kind == CovNotificationKind::Single)
            .cloned()
            .collect();
        self.settle();
        subs
    }

    /// Look again at the live references `keys` names, in its order, object
    /// by object, each object under a budget of its own: an object comes
    /// up with the first reference to it, and its later ones come along.
    /// A reference whose subscription or object is gone can't be looked
    /// at, so it is owed nothing more.
    fn look(&mut self, db: &ObjectDatabase, keys: &[CovSubscriptionKey], now: Instant) {
        // COV-multiple references are looked at by context.
        #[cfg(feature = "cov-multiple")]
        self.look_multiple(db, keys, now);
        let single = |key: &CovSubscriptionKey| key.multiple_context().is_none();
        for (at, key) in keys.iter().enumerate().filter(|(_, key)| single(key)) {
            let object = key.object();
            let seen = |earlier: &CovSubscriptionKey| single(earlier) && earlier.object() == object;
            if keys[..at].iter().any(seen) {
                continue;
            }
            let mut subs = Vec::new();
            for (later, key) in keys.iter().enumerate().skip(at) {
                if !single(key) || key.object() != object || keys[at..later].contains(key) {
                    continue;
                }
                let sub = self.table.get_subscription(key);
                match sub.filter(|sub| self.table.is_current(sub, now)) {
                    Some(sub) => subs.push(sub.clone()),
                    None => self.forgive(|owed| owed == key),
                }
            }
            #[cfg(test)]
            {
                self.looked += subs.len();
            }
            match db.get(&object) {
                Some(stored) if !subs.is_empty() => self.report(stored, &subs, false, now),
                Some(_) => {}
                None => self.forgive(|owed| owed.object() == object),
            }
        }
    }

    /// Evaluate a change of `object` for `subs` and queue what it reports.
    /// An unconfirmed report replaces the one queued for its subscription,
    /// which ends unsent; a report the outbox has no room for leaves its
    /// subscription owed.
    fn report(
        &mut self,
        object: &dyn bacnet_objects::traits::BACnetObject,
        subs: &[CovSubscriptionSnapshot],
        force: bool,
        now: Instant,
    ) {
        let mut budget = EventBudget::new(self.table.policy());
        let max_octets = self.table.policy().max_notification_bytes_per_event;
        let mut room = self.outbox.room();
        let limits = self.outbox.limits();
        let outbox = &self.outbox;
        let mut refused = Vec::new();
        // Owed references admitted: the budget may still drop their reports.
        let (owed, owed_ids) = (&self.owed, &self.owed_ids);
        let mut admitted = Vec::new();
        let evaluation = Evaluation {
            device: self.device,
            force,
        };
        let reports = evaluate(
            &mut self.table,
            object,
            subs,
            evaluation,
            &mut budget,
            now,
            |sub, octets| {
                // Too large for the whole outbox, or for one input's budget
                // of octets: owing it would only see it refused again.
                let hopeless = limits.entries == 0 || octets > limits.octets || octets > max_octets;
                if !hopeless && room.take(octets, outbox.queued(sub.key())) {
                    if owed.contains(sub.key()) || owed_ids.contains(sub.key()) {
                        admitted.push(sub.key().clone());
                    }
                    return Some(());
                }
                refused.push((sub.key().clone(), hopeless));
                None
            },
        );
        self.settle();
        if !self.owed.is_empty() || !self.owed_ids.is_empty() {
            self.keep_owed(subs, &reports, &refused, &admitted, &budget);
        }
        for report in reports {
            let sub = &subs[report.index];
            let pushed = self.outbox.push(
                report.token,
                sub.key().clone(),
                sub.endpoint(),
                (sub.issue_confirmed_notifications, NotifyService::Cov),
                now,
                &report.params,
            );
            if let Some(replaced) = pushed.replaced {
                // Superseded before it was sent: its baseline never was.
                self.table.delivered(replaced, Delivery::NotSent, now);
            }
            if !pushed.queued {
                debug_assert!(false, "admission reserved the room");
                self.table.delivered(report.token, Delivery::NotSent, now);
                self.owe(sub.key().clone());
            }
        }
        for (key, hopeless) in refused {
            if hopeless {
                self.counters.owed += 1;
            } else {
                self.owe(key);
            }
        }
    }

    /// After an evaluation of `subs`: an owed reference it reported, or
    /// found nothing to report for, is owed no more. One refused again, or
    /// whose report the budget dropped, keeps its place.
    fn keep_owed(
        &mut self,
        subs: &[CovSubscriptionSnapshot],
        reports: &[CovReport<()>],
        refused: &[(CovSubscriptionKey, bool)],
        admitted: &[CovSubscriptionKey],
        budget: &EventBudget,
    ) {
        // The budget runs out at a report, and past it evaluate drops a
        // report before asking admission: a reference there may have had one.
        let spent = budget
            .is_exhausted()
            .then(|| reports.last().map_or(0, |report| report.index + 1));
        let keep = |key: &CovSubscriptionKey| {
            let Some(at) = subs.iter().position(|sub| sub.key() == key) else {
                return true;
            };
            if reports.iter().any(|report| report.index == at) {
                return false;
            }
            if let Some((_, hopeless)) = refused.iter().find(|(again, _)| again == key) {
                return !hopeless;
            }
            admitted.contains(key) || spent.is_some_and(|from| at >= from)
        };
        self.owed.retain(keep);
        // One refused room now is owed room, not an ID.
        let refused_room = |key: &CovSubscriptionKey| refused.iter().any(|(again, _)| again == key);
        self.owed_ids.retain(|key| keep(key) && !refused_room(key));
    }

    /// Owe `key` a later look, after those owed already; counted once,
    /// however often it is refused until an evaluation reports it or finds
    /// nothing to report.
    fn owe(&mut self, key: CovSubscriptionKey) {
        if !self.owed.contains(&key) {
            self.counters.owed += 1;
            self.owed.push(key);
        }
    }

    /// End the confirmed report `token` names as `outcome` says. An event
    /// notification's end is only counted: nothing waits on it.
    fn end_confirmed(&mut self, token: Token, outcome: Delivery, now: Instant) {
        if outcome == Delivery::Failed {
            self.counters.failed += 1;
        }
        if token.is_event() {
            return;
        }
        // A COV-multiple report's claim is settled before the table ends
        // the report, so what it returns is back before the mark clears.
        #[cfg(feature = "cov-multiple")]
        let context = self.settle_lodged(token, outcome, now);
        let owed = self.table.delivered(token, outcome, now);
        #[cfg(feature = "cov-multiple")]
        if let Some(context) = context.filter(|_| outcome == Delivery::Failed) {
            self.hold_off_multiple(&context, token);
        }
        self.looks.extend(owed);
        self.settle();
    }

    /// Take the table's counts and what it fenced or removed: a fenced
    /// report's queued notification dies and its invoke ID goes back, a
    /// removed reference's queued notifications die, and owed references
    /// get their look.
    fn settle(&mut self) {
        self.settle_after(true);
    }

    /// [`settle`](Self::settle), with the timestamped histories brought in
    /// line with the table when `follow`: every mutation but the
    /// SubscribeCOVPropertyMultiple service, which does that itself.
    fn settle_after(&mut self, follow: bool) {
        let counts = self.table.take_counts();
        add(&mut self.counters.cov, &counts);
        let fenced = self.table.take_fenced();
        #[cfg(feature = "cov-multiple")]
        self.settle_multiple(&fenced, follow);
        #[cfg(not(feature = "cov-multiple"))]
        let _ = follow;
        if fenced.is_empty() {
            return;
        }
        let before = self.outbox.room();
        self.outbox.kill(|entry| {
            let removed = entry.key().is_some_and(|key| fenced.removed.contains(key));
            fenced.tokens.contains(&entry.token) || removed
        });
        self.room_freed |= self.outbox.room().octets_free() > before.octets_free();
        for token in &fenced.tokens {
            if let Some(out) = self.confirmed.take(*token) {
                self.releases.push((out.peer, out.invoke_id));
            }
        }
        self.forgive(|key| fenced.removed.contains(key));
        self.looks.extend(fenced.revisit);
    }

    /// Give back the invoke IDs of fenced notifications.
    fn release(&mut self, ids: &mut dyn InvokeIds) {
        self.count_freed(self.releases.len());
        for (peer, id) in self.releases.drain(..) {
            ids.release(&peer, id);
        }
    }

    fn encode_confirmed(
        &self,
        invoke_id: u8,
        service: NotifyService,
        params: &[u8],
        out: &mut BytesMut,
    ) {
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: self.max_apdu,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service.confirmed(),
            service_request: Bytes::copy_from_slice(params),
        });
        encode_apdu(out, &pdu).expect("a validated Max-APDU always encodes");
    }
}

/// A subscribe service the reporter runs, read from an unsegmented
/// confirmed request's header.
fn subscribe_service(apdu: &[u8]) -> Option<ConfirmedServiceChoice> {
    let (&first, service) = (apdu.first()?, apdu.get(3)?);
    if first >> 4 != 0 || first & SEGMENTED != 0 {
        return None;
    }
    let service = ConfirmedServiceChoice::from_raw(*service);
    let multiple = cfg!(feature = "cov-multiple")
        && service == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
    let single = matches!(
        service,
        ConfirmedServiceChoice::SUBSCRIBE_COV | ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY
    );
    (single || multiple).then_some(service)
}

/// Which entry a change came through, for [`Reporter::after_change`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// A request's write, its timestamped changes captured as each attempt
    /// committed.
    Request,
    /// A change the host made or tells of.
    Host,
    /// A background pass's changes, its transitions committed and
    /// distributed already.
    #[cfg(feature = "event-reporting")]
    Committed,
}

/// Add `counts`, one table mutation's, to the running `total`.
fn add(total: &mut CovCounters, counts: &CovCounters) {
    total.subscriptions_created += counts.subscriptions_created;
    total.subscriptions_cancelled += counts.subscriptions_cancelled;
    total.subscriptions_purged += counts.subscriptions_purged;
    total.subscriptions_rejected_quota += counts.subscriptions_rejected_quota;
    total.subscriptions_rejected_capacity += counts.subscriptions_rejected_capacity;
    total.subscriptions_rejected_indefinite += counts.subscriptions_rejected_indefinite;
    total.notifications_throttled_fanout += counts.notifications_throttled_fanout;
}

/// Captures each Life Safety object a write is about to change.
#[derive(Default)]
struct Capture(LifeSafetyCovSnapshots);

impl WriteCommitObserver for Capture {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        if is_life_safety_object(write.oid) {
            self.0.capture_before_write(db, write.oid);
        }
    }
    fn committed(&mut self, _: &mut ObjectDatabase) {}
    fn failed(&mut self, _: &mut ObjectDatabase, _: &Error) {}
}
