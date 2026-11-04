//! [`Reporter`]: the `Responder` with COV reporting, for a host with nothing
//! to await between steps.

use alloc::vec::Vec;

use bacnet_encoding::apdu::{
    Apdu, ConfirmedRequest, SimpleAck, UnconfirmedRequest, decode_apdu, encode_apdu,
    max_apdu_header_at_or_below,
};
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_objects::database::ObjectDatabase;
use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

use super::confirmed::{ConfirmedTable, Outstanding};
use super::outbox::Outbox;
use super::{
    Bindings, Destination, InvokeIds, Notification, NotifyService, ReportConfig, ReporterCounters,
    peer_address,
};
use crate::cov::active::ActiveCovOverlay;
use crate::cov::life_safety::{LifeSafetyCovSnapshots, is_life_safety_object};
use crate::cov::{
    CovCounters, CovNotificationKind, CovSubscriptionKey, CovSubscriptionSnapshot, CovTable,
    Delivery, Evaluation, EventBudget, FlightTiming, SubscriberEndpoint, Token, evaluate,
};
use crate::reply::{self, SEGMENTED, emit};
use crate::responder::{Access, Inbound, Outcome, Responder, Written};
use crate::write::{WriteCommitObserver, WriteTarget};

#[path = "reporter_tests.rs"]
#[cfg(test)]
mod tests;

/// The [`Responder`] with COV reporting: SubscribeCOV and
/// SubscribeCOVProperty, the notifications what a request or the host
/// changes calls for, confirmed delivery with its retries, and the Device's
/// subscription lists. It strings together the staged steps
/// `bacnet-server` runs under its guards; the parity tests hold the two to
/// the same notifications.
///
/// The host hands it each request ([`handle`](Self::handle)), each answer to
/// a confirmed notification ([`reply`](Self::reply)), the objects it changes
/// itself ([`changed`](Self::changed)), and the time whenever
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
    /// References whose report found no room, looked at again once room
    /// frees.
    owed: Vec<CovSubscriptionKey>,
    /// An entry left the outbox since the owed references were last looked
    /// at.
    room_freed: bool,
    /// Invoke IDs of fenced notifications, given back at the next call
    /// that has the host's [`InvokeIds`].
    releases: Vec<(BACnetAddress, u8)>,
    /// Life Safety values the host captured before changing them.
    snapshots: LifeSafetyCovSnapshots,
    counters: ReporterCounters,
    /// The latest `now` the host gave.
    last: Instant,
}

impl Reporter {
    /// A reporter answering requests with `responder`, under `config`, at
    /// first holding no subscription. Its outbox is allocated here.
    pub fn new(responder: Responder, config: ReportConfig) -> Result<Self, Error> {
        let identity = responder.identity();
        let device = identity.device_oid();
        let max_apdu = max_apdu_header_at_or_below(u32::from(identity.max_apdu_length()))?;
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
            releases: Vec::new(),
            snapshots: LifeSafetyCovSnapshots::default(),
            counters: ReporterCounters::default(),
            last: Instant::ZERO,
        })
    }

    /// The subscription table.
    pub fn table(&self) -> &CovTable {
        &self.table
    }

    /// Run `request` from `peer` against `db`, writing any answer to `out`
    /// for the host to send before anything [`pop`](Self::pop) hands out.
    ///
    /// SubscribeCOV and SubscribeCOVProperty are answered here, and an
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
        if let Some(service) = subscribe_service(&apdu) {
            return self.subscribe(db, peer, service, request, now, out);
        }
        let handled = if Responder::classify(&apdu) == Access::Write {
            let mut capture = Capture::default();
            let written = self.responder.write(db, Some(&mut capture), request, out);
            self.report_changes(db, &written.objects, &capture.0, now);
            written
        } else {
            let overlay = ActiveCovOverlay::new(&self.table, db, self.device, now);
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
    /// computes): queue what that reports.
    pub fn changed(&mut self, db: &ObjectDatabase, objects: &[ObjectIdentifier], now: Instant) {
        self.last = now;
        let snapshots = core::mem::take(&mut self.snapshots);
        self.report_changes(db, objects, &snapshots, now);
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
        let service = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
        let (invoke_id, outcome) = match decode_apdu(Bytes::copy_from_slice(apdu)) {
            Ok(Apdu::SimpleAck(ack)) if ack.service_choice == service => {
                (ack.invoke_id, Delivery::Acknowledged)
            }
            Ok(Apdu::Error(error)) if error.service_choice == service => {
                (error.invoke_id, Delivery::Failed)
            }
            Ok(Apdu::Reject(reject)) => (reject.invoke_id, Delivery::Failed),
            Ok(Apdu::Abort(abort)) if abort.sent_by_server => (abort.invoke_id, Delivery::Failed),
            _ => return false,
        };
        let Some(out) = self.confirmed.answered(from, invoke_id) else {
            return false;
        };
        ids.release(&out.peer, out.invoke_id);
        self.end_confirmed(out.token, outcome, now);
        true
    }

    /// When the host should call [`on_deadline`](Self::on_deadline): the
    /// earliest lifetime, report deadline or overdue answer, or the latest
    /// `now` it gave when references are owed a look.
    pub fn next_deadline(&self) -> Option<Instant> {
        let looks = !self.looks.is_empty() || (!self.owed.is_empty() && self.room_freed);
        let resend = self.confirmed.resend_due();
        bacnet_time::earliest([
            self.table.next_expiry(),
            self.confirmed.next_deadline(),
            (looks || resend).then_some(self.last),
        ])
    }

    /// The time `now` has come: expire lifetimes and abandoned reports,
    /// time out unanswered notifications (to resend, or failed), and look
    /// again at the references owed one.
    pub fn on_deadline(&mut self, db: &ObjectDatabase, ids: &mut dyn InvokeIds, now: Instant) {
        self.last = now;
        self.release(ids);
        self.table.expire(now);
        self.settle();
        for out in self.confirmed.time_out(now) {
            ids.release(&out.peer, out.invoke_id);
            self.end_confirmed(out.token, Delivery::Failed, now);
        }
        let mut keys = core::mem::take(&mut self.looks);
        if self.room_freed {
            keys.append(&mut self.owed);
            self.room_freed = false;
        }
        self.look(db, keys, now);
    }

    /// The next notification to send, its APDU appended to `out`: a
    /// confirmed one due again first, then the outbox in order. A confirmed
    /// one takes an invoke ID from `ids`; one that gets none ends unsent,
    /// and its subscription is owed a later look. `bindings` resolves the
    /// Device recipients of event notifications, from #2303's third part.
    pub fn pop(
        &mut self,
        bindings: &dyn Bindings,
        ids: &mut dyn InvokeIds,
        now: Instant,
        out: &mut BytesMut,
    ) -> Option<Notification> {
        let _ = bindings;
        self.last = now;
        self.release(ids);
        if let Some(resend) = self.confirmed.resend(now, self.timing.timeout) {
            let notification = Notification {
                token: resend.token,
                to: Destination::Subscriber(resend.to.clone()),
                service: NotifyService::Cov,
                invoke_id: Some(resend.invoke_id),
                priority: NetworkPriority::Normal,
            };
            let (invoke_id, params) = (resend.invoke_id, resend.params.clone());
            self.encode_confirmed(invoke_id, &params, out);
            self.counters.resent += 1;
            return Some(notification);
        }
        loop {
            let (token, key, to, params, confirmed, begun) = self.outbox.pop(|entry, params| {
                (
                    entry.token,
                    entry.key.clone(),
                    entry.to.clone(),
                    params.to_vec(),
                    entry.confirmed,
                    entry.begun,
                )
            })?;
            self.room_freed = true;
            let start = out.len();
            let invoke_id = if confirmed {
                // A report popped so late that its answers would be cut off
                // ends as the server's worker cut off at its first poll
                // does: failed, held off.
                let cutoff = begun.saturating_add(self.timing.cutoff());
                if now >= cutoff {
                    self.end_confirmed(token, Delivery::Failed, now);
                    continue;
                }
                let peer = peer_address(&to);
                let Some(invoke_id) = ids.take(&peer) else {
                    // No invoke ID to send it under: it ends unsent, and the
                    // change is looked at again later.
                    self.table.delivered(token, Delivery::NotSent, now);
                    self.counters.owed += 1;
                    self.owe(key);
                    continue;
                };
                self.encode_confirmed(invoke_id, &params, out);
                let deadline = now.saturating_add(self.timing.timeout);
                let retries = self.timing.retries;
                let (to, params) = (to.clone(), params);
                self.confirmed.insert(Outstanding::new(
                    token,
                    peer,
                    to,
                    invoke_id,
                    params,
                    (deadline, cutoff),
                    retries,
                ));
                Some(invoke_id)
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: Bytes::from(params),
                });
                encode_apdu(out, &pdu).expect("an UnconfirmedRequest always encodes");
                None
            };
            let counters = &mut self.counters.cov;
            counters.notifications_sent += 1;
            if confirmed {
                counters.notifications_confirmed += 1;
            } else {
                counters.notifications_unconfirmed += 1;
            }
            counters.notification_bytes_sent += (out.len() - start) as u64;
            return Some(Notification {
                token,
                to: Destination::Subscriber(to),
                service: NotifyService::Cov,
                invoke_id,
                priority: NetworkPriority::Normal,
            });
        }
    }

    /// How the send of the notification `token` names ended. An
    /// unconfirmed one that was [`Sent`](Delivery::Sent) becomes its
    /// subscription's baseline. A confirmed one ends through
    /// [`reply`](Self::reply) or its timeout; the host reports one here
    /// only when it couldn't send it ([`NotSent`](Delivery::NotSent)), and
    /// it then ends with no hold-off.
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
            ids.release(&out.peer, out.invoke_id);
            self.end_confirmed(token, outcome, now);
            return;
        }
        self.table.delivered(token, outcome, now);
        self.settle();
    }

    /// The Device's subscription lists in `db`, live at `now`, for a read
    /// the host runs itself.
    pub fn overlay<'a>(&'a self, db: &'a ObjectDatabase, now: Instant) -> ActiveCovOverlay<'a> {
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

    /// Look again at the live references among `keys`, object by object,
    /// each object under a budget of its own.
    fn look(&mut self, db: &ObjectDatabase, mut keys: Vec<CovSubscriptionKey>, now: Instant) {
        keys.sort_unstable();
        keys.dedup();
        let mut subs: Vec<_> = keys
            .iter()
            .filter_map(|key| self.table.get_subscription(key))
            .filter(|sub| self.table.is_current(sub, now))
            .cloned()
            .collect();
        while let Some(first) = subs.first().map(|sub| sub.monitored_object_identifier) {
            let (this, rest): (Vec<_>, Vec<_>) = subs
                .into_iter()
                .partition(|sub| sub.monitored_object_identifier == first);
            subs = rest;
            if let Some(object) = db.get(&first) {
                self.report(object, &this, false, now);
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
        let mut room = self.outbox.room();
        let limits = self.outbox.limits();
        let outbox = &self.outbox;
        let mut refused = Vec::new();
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
                if room.take(octets, outbox.queued(sub.key())) {
                    return Some(());
                }
                // Too large for the whole outbox: owing it would only find
                // no room again.
                let hopeless = limits.entries == 0 || octets > limits.octets;
                refused.push((sub.key().clone(), hopeless));
                None
            },
        );
        self.settle();
        for report in reports {
            let sub = &subs[report.index];
            let pushed = self.outbox.push(
                report.token,
                sub.key().clone(),
                sub.endpoint(),
                sub.issue_confirmed_notifications,
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
            self.counters.owed += 1;
            if !hopeless {
                self.owe(key);
            }
        }
    }

    fn owe(&mut self, key: CovSubscriptionKey) {
        if !self.owed.contains(&key) {
            self.owed.push(key);
        }
    }

    /// End the confirmed report `token` names as `outcome` says.
    fn end_confirmed(&mut self, token: Token, outcome: Delivery, now: Instant) {
        if outcome == Delivery::Failed {
            self.counters.failed += 1;
        }
        let owed = self.table.delivered(token, outcome, now);
        self.looks.extend(owed);
        self.settle();
    }

    /// Take the table's counts and what it fenced or removed: a fenced
    /// report's queued notification dies and its invoke ID goes back, a
    /// removed reference's queued notifications die, and owed references
    /// get their look.
    fn settle(&mut self) {
        let counts = self.table.take_counts();
        add(&mut self.counters.cov, &counts);
        let fenced = self.table.take_fenced();
        if fenced.is_empty() {
            return;
        }
        let before = self.outbox.room();
        self.outbox.kill(|entry| {
            fenced.tokens.contains(&entry.token) || fenced.removed.contains(&entry.key)
        });
        self.room_freed |= self.outbox.room().octets_free() > before.octets_free();
        for token in &fenced.tokens {
            if let Some(out) = self.confirmed.take(*token) {
                self.releases.push((out.peer, out.invoke_id));
            }
        }
        self.owed.retain(|key| !fenced.removed.contains(key));
        self.looks.extend(fenced.revisit);
    }

    /// Give back the invoke IDs of fenced notifications.
    fn release(&mut self, ids: &mut dyn InvokeIds) {
        for (peer, id) in self.releases.drain(..) {
            ids.release(&peer, id);
        }
    }

    fn encode_confirmed(&self, invoke_id: u8, params: &[u8], out: &mut BytesMut) {
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: self.max_apdu,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: Bytes::copy_from_slice(params),
        });
        encode_apdu(out, &pdu).expect("a validated Max-APDU always encodes");
    }
}

/// SubscribeCOV or SubscribeCOVProperty, read from an unsegmented confirmed
/// request's header.
fn subscribe_service(apdu: &[u8]) -> Option<ConfirmedServiceChoice> {
    let (&first, service) = (apdu.first()?, apdu.get(3)?);
    if first >> 4 != 0 || first & SEGMENTED != 0 {
        return None;
    }
    let service = ConfirmedServiceChoice::from_raw(*service);
    matches!(
        service,
        ConfirmedServiceChoice::SUBSCRIBE_COV | ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY
    )
    .then_some(service)
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
