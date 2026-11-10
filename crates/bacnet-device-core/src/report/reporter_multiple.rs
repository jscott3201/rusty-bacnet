//! The [`Reporter`] running SubscribeCOVPropertyMultiple and its
//! notifications (Clauses 13.16 to 13.18), on the steps `bacnet-server`'s
//! fanout runs: [`multiple_report`](crate::cov::multiple_report) decides,
//! and what the server does around its sends with guards that settle when
//! dropped, the `Reporter` does explicitly, around its outbox.
//!
//! - A change is captured where it commits: each attempt of a write
//!   request as an observer of the write, a change the host makes itself
//!   when it reports it ([`Reporter::changed`]).
//! - A fanout takes its contexts in key order. An unconfirmed report's
//!   parts are queued together and leave one by one. While any is queued
//!   the report holds its context's send turn, so a later fanout stands
//!   back and is handed to a follow-up when the turn ends. Each part the
//!   host reports sent has its claim committed; one that was not sent stops
//!   the report, whose later parts leave the outbox and go back to their
//!   references, owing once a part went. A part that found no room in the
//!   outbox or no budget stops the report the same way.
//! - A confirmed report begins when its first part is queued, as a Single
//!   one does, with its claim lodged until the report's deadline. The claim
//!   is settled where the report ends: delivered on the Ack, returned on
//!   every other outcome and on a fence, and after a failure the backstop
//!   waits out the hold-off.
//! - The histories' deadline is one of the `Reporter`'s, and what is due
//!   then is fanned out again, as the references owed a look are.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use bacnet_encoding::apdu::{Apdu, SimpleAck, decode_apdu};
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::cov_multiple::{
    COVNotificationMultipleRequest, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_time::Instant;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::Reporter;
use crate::cov::life_safety::{LifeSafetyCovChange, LifeSafetyCovSnapshots};
use crate::cov::multiple_report::{
    Carried, Gathering, Reading, Report, Source, begin_confirmed, by_context, carried, hold_off,
    in_turn, part_token,
};
use crate::cov::timed::{ClaimId, Settlement, TimedClaim, TimedHistories, follow_table};
use crate::cov::timed_capture::TimedCapture;
use crate::cov::{
    BeginRefusal, CovNotificationKind, CovSubscriptionKey, CovSubscriptionSnapshot, Delivery,
    EventBudget, Fenced, MultipleContextKey, SubscriberEndpoint, Token,
    subscribe_cov_property_multiple,
};
use crate::reply::{self, emit};
use crate::report::NotifyService;
use crate::report::outbox::Room;
use crate::responder::{Inbound, Outcome, Written};

/// Octets an UnconfirmedCOVNotificationMultiple APDU adds to its
/// parameters.
const UNCONFIRMED_HEADER: usize = 2;
/// Octets an unsegmented ConfirmedCOVNotificationMultiple APDU adds to its
/// parameters.
const CONFIRMED_HEADER: usize = 4;

/// What the `Reporter` keeps for the COV-multiple path.
#[derive(Debug)]
pub(super) struct Multiple {
    /// The timestamped histories, held outright.
    pub(super) histories: TimedHistories,
    /// Unconfirmed reports with a part still in the outbox.
    sending: Vec<Sending>,
    /// Confirmed reports out, each with the claim it lodged.
    lodged: Vec<Lodged>,
}

/// One unconfirmed report going out.
#[derive(Debug)]
struct Sending {
    context: MultipleContextKey,
    /// The reference its outbox entries are queued under.
    route: CovSubscriptionKey,
    /// The context's references, while the report holds its send turn.
    turn: Option<Vec<CovSubscriptionKey>>,
    /// Every reference the report retains.
    observations: Vec<Carried>,
    /// The parts in the outbox, in the order they leave.
    queued: Vec<Queued>,
    /// The claims of the parts that were not queued or not sent, in the
    /// order they go back.
    unsent: Vec<TimedClaim>,
    /// Whether a part went.
    sent: bool,
    /// What the parts sent so far complete.
    delivered: Vec<Carried>,
}

/// One part of an unconfirmed report in the outbox.
#[derive(Debug)]
struct Queued {
    token: Token,
    claim: TimedClaim,
    finishes: BTreeSet<CovSubscriptionKey>,
}

/// A confirmed report out.
#[derive(Debug)]
struct Lodged {
    token: Token,
    claim: Option<ClaimId>,
    context: MultipleContextKey,
}

impl Multiple {
    /// Nothing subscribed, for a device whose maximum APDU is `local_apdu`
    /// octets.
    pub(super) fn new(local_apdu: usize) -> Self {
        Self {
            histories: TimedHistories::new(local_apdu),
            sending: Vec::new(),
            lodged: Vec::new(),
        }
    }
}

impl Reporter {
    /// SubscribeCOVPropertyMultiple: the reply, then the initial report of
    /// the references it accepted, or kept before the one it refused.
    pub(super) fn subscribe_multiple(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        request: Inbound,
        now: Instant,
        out: &mut BytesMut,
    ) -> Written {
        let Ok(Apdu::ConfirmedRequest(confirmed)) = decode_apdu(request.apdu) else {
            return Outcome::Silent.into();
        };
        let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
        let decoded = SubscribeCOVPropertyMultipleRequest::decode(&confirmed.service_request)
            .map_err(Error::into_request_reject);
        let (answer, accepted) = match decoded {
            Err(error) => (
                reply::error_apdu(confirmed.invoke_id, service, &error),
                Vec::new(),
            ),
            Ok(request) => {
                let admitted = subscribe_cov_property_multiple(
                    &mut self.table,
                    &mut self.multiple.histories,
                    db,
                    peer,
                    Some(confirmed.max_apdu_length),
                    request,
                    now,
                );
                match admitted {
                    Ok(accepted) => (
                        Apdu::SimpleAck(SimpleAck {
                            invoke_id: confirmed.invoke_id,
                            service_choice: service,
                        }),
                        accepted,
                    ),
                    Err(refusal) => (
                        reply::error_apdu(confirmed.invoke_id, service, &refusal.error),
                        refusal.committed,
                    ),
                }
            }
        };
        let outcome = emit(out, &answer, Outcome::Reply);
        // The service kept the histories in step with the table itself.
        self.settle_after(false);
        let accepted: Vec<_> = accepted
            .into_iter()
            .filter(|sub| self.table.is_current(sub, now))
            .collect();
        let mut budget = EventBudget::new(self.table.policy());
        self.fan_out_multiple(db, accepted, true, now, &mut budget);
        outcome.into()
    }

    /// Capture what the host changed on `objects` itself, as its producers
    /// would where the change committed: a Life Safety object held in
    /// `snapshots` from the values it changed, the rest by comparison.
    pub(super) fn capture_changed(
        &mut self,
        db: &ObjectDatabase,
        objects: &[ObjectIdentifier],
        snapshots: &LifeSafetyCovSnapshots,
        now: Instant,
    ) {
        let histories = &mut self.multiple.histories;
        for &object in objects.iter().filter(|&&oid| !snapshots.holds(oid)) {
            TimedCapture::object(&self.table, object, now).run(db, histories, now);
        }
        let changes = snapshots.changes(db, objects);
        if !changes.is_empty() {
            TimedCapture::exact(&self.table, &changes, now).run(db, histories, now);
        }
    }

    /// Report a change of `object` to its COV-multiple references: those
    /// `change` notifies, when it is a Life Safety object's exact change.
    pub(super) fn report_multiples(
        &mut self,
        db: &ObjectDatabase,
        object: ObjectIdentifier,
        change: Option<&LifeSafetyCovChange>,
        now: Instant,
    ) {
        let subs: Vec<_> = self
            .table
            .subscriptions_for(&object, now)
            .into_iter()
            .filter(|sub| {
                sub.notification_kind == CovNotificationKind::Multiple
                    && change.is_none_or(|change| change.notifies(sub))
            })
            .cloned()
            .collect();
        self.settle();
        let force = change.is_some_and(LifeSafetyCovChange::status_changed);
        let mut budget = EventBudget::new(self.table.policy());
        self.fan_out_multiple(db, subs, force, now, &mut budget);
    }

    /// Look again at the live COV-multiple references among `keys`: each
    /// context once, under a budget of its own, as the server's follow-up
    /// pass does.
    pub(super) fn look_multiple(
        &mut self,
        db: &ObjectDatabase,
        keys: &[CovSubscriptionKey],
        now: Instant,
    ) {
        let wanted: BTreeSet<_> = keys
            .iter()
            .filter(|key| key.multiple_context().is_some())
            .collect();
        if wanted.is_empty() {
            return;
        }
        let subs: Vec<_> = wanted
            .iter()
            .filter_map(|key| self.table.get_subscription(key))
            .filter(|sub| self.table.is_current(sub, now))
            .cloned()
            .collect();
        // A reference owed a look for want of room has it now; its report
        // owes it again if it is refused again.
        self.owed.retain(|owed| !wanted.contains(owed));
        for subs in by_context(subs).values() {
            let mut budget = EventBudget::new(self.table.policy());
            self.report_context(db, subs, false, now, &mut budget);
        }
        self.multiple_counts();
    }

    /// Report a change to `subs`, COV-multiple references, context by
    /// context in the order of their keys, under one budget.
    fn fan_out_multiple(
        &mut self,
        db: &ObjectDatabase,
        subs: Vec<CovSubscriptionSnapshot>,
        force: bool,
        now: Instant,
        budget: &mut EventBudget,
    ) {
        if subs.is_empty() || budget.is_exhausted() {
            return;
        }
        for subs in in_turn(&self.multiple.histories, by_context(subs)) {
            self.report_context(db, &subs, force, now, budget);
        }
        self.multiple_counts();
    }

    /// Decide what `subs`, one context's references, report, and queue it.
    fn report_context(
        &mut self,
        db: &ObjectDatabase,
        subs: &[CovSubscriptionSnapshot],
        force: bool,
        now: Instant,
        budget: &mut EventBudget,
    ) {
        let reading = Reading::read(Source::Database(db), subs, force);
        let histories = &mut self.multiple.histories;
        let mut gathered = match reading.gather(&mut self.table, histories, now) {
            Gathering::Ready(gathered) => gathered,
            // A failed report's hold-off has passed: the whole context is
            // fanned out once, now, under a budget of its own, as the
            // server's follow-up pass does. The step took the debt, so
            // this second look cannot come back here.
            Gathering::FollowUp(keys) => {
                let subs: Vec<_> = keys
                    .iter()
                    .filter_map(|key| self.table.get_subscription(key))
                    .filter(|sub| self.table.is_current(sub, now))
                    .cloned()
                    .collect();
                let mut budget = EventBudget::new(self.table.policy());
                self.report_context(db, &subs, false, now, &mut budget);
                return;
            }
            Gathering::Busy | Gathering::StoodBack | Gathering::Nothing { .. } => return,
        };
        let context = gathered.context().clone();
        let mut turn = gathered.take_turn();
        let local = u32::from(self.responder.identity().max_apdu_length());
        let report = gathered.split(histories, self.device, local);
        if !report.needs_turn {
            self.end_turn(&context, turn.take());
        }
        // A context the spent budget keeps back goes first at its next
        // fanout; one that has its turn waits no more.
        let spent = budget.is_exhausted();
        self.multiple.histories.note_turn(&context, spent);
        if spent {
            self.counters.cov.notifications_throttled_fanout += 1;
            for part in report.parts {
                self.multiple.histories.give_back(part.claim, now);
            }
            self.end_turn(&context, turn);
            return;
        }
        if report.route.issue_confirmed_notifications {
            self.queue_confirmed(report, now, budget);
        } else {
            self.queue_unconfirmed(report, context, turn, now, budget);
        }
    }

    /// End the send turn of `context`, if `turn` holds it; a fanout that
    /// stood back meanwhile gets its look.
    fn end_turn(&mut self, context: &MultipleContextKey, turn: Option<Vec<CovSubscriptionKey>>) {
        if let Some(keys) = turn
            && self.multiple.histories.end_send(context)
        {
            self.looks.extend(keys);
        }
    }

    /// Begin a confirmed report and queue its first part; the rest is
    /// deferred to the follow-up its Ack owes.
    fn queue_confirmed(&mut self, report: Report, now: Instant, budget: &mut EventBudget) {
        let Some(first) = report.into_first() else {
            return;
        };
        let key = first.route.key().clone();
        let context = key.multiple_context().expect("Multiple snapshot").clone();
        let mut params = BytesMut::new();
        let encoded = first.notification.encode(&mut params).is_ok();
        let octets = params.len() + CONFIRMED_HEADER;
        // Room in the outbox is this host's admission, ahead of the budget
        // as the server's in-flight slot and invoke ID are.
        let room = encoded && self.outbox.room().take(params.len(), None);
        if !room || !budget.try_consume(octets) {
            if room {
                self.counters.cov.notifications_throttled_fanout += 1;
            } else if encoded {
                // Looked at again as room frees.
                self.owe(key);
            }
            let histories = &mut self.multiple.histories;
            histories.give_back(first.claim, now);
            for deferred in first.deferred {
                histories.give_back(deferred, now);
            }
            return;
        }
        let begun = begin_confirmed(
            &mut self.table,
            &mut self.multiple.histories,
            first.completion,
            &first.observations,
            Some(first.claim),
            first.deferred,
            now,
        );
        match begun {
            Ok(begun) => {
                self.multiple.lodged.push(Lodged {
                    token: begun.token,
                    claim: begun.claim,
                    context,
                });
                let pushed = self.outbox.push(
                    begun.token,
                    key.clone(),
                    first.route.endpoint(),
                    (true, NotifyService::CovMultiple),
                    now,
                    &params,
                );
                if !pushed.queued {
                    debug_assert!(false, "admission reserved the room");
                    self.end_confirmed(begun.token, Delivery::NotSent, now);
                    self.owe(key);
                }
            }
            Err(refusal) => {
                budget.refund(octets);
                // A fence moved a reference since it was captured: look at
                // its live state again. A busy context needs nothing.
                if refusal == BeginRefusal::NotCurrent {
                    let keys = first.observations.iter().map(|(sub, _)| sub.key().clone());
                    self.looks.extend(keys);
                }
            }
        }
        self.settle();
    }

    /// Queue every part of an unconfirmed report, oldest changes first. A
    /// part that finds no room or no budget stops the rest, which goes back
    /// when the report ends.
    fn queue_unconfirmed(
        &mut self,
        report: Report,
        context: MultipleContextKey,
        turn: Option<Vec<CovSubscriptionKey>>,
        now: Instant,
        budget: &mut EventBudget,
    ) {
        let Report {
            route,
            parts,
            observations,
            ..
        } = report;
        let mut sending = Sending {
            context,
            route: route.key().clone(),
            turn,
            observations,
            queued: Vec::new(),
            unsent: Vec::new(),
            sent: false,
            delivered: Vec::new(),
        };
        let mut room = self.outbox.room();
        let (mut stopped, mut crowded) = (false, false);
        for part in parts {
            let token = if stopped {
                None
            } else {
                self.queue_part(
                    &route,
                    &part.notification,
                    &mut room,
                    &mut crowded,
                    budget,
                    now,
                )
            };
            match token {
                Some(token) => sending.queued.push(Queued {
                    token,
                    claim: part.claim,
                    finishes: part.finishes,
                }),
                None => {
                    stopped = true;
                    sending.unsent.push(part.claim);
                }
            }
        }
        if sending.queued.is_empty() {
            // Nothing went and nothing is owed in the histories: looked at
            // again as room frees.
            if crowded {
                self.owe(route.key().clone());
            }
            self.finish(sending, now);
        } else {
            self.multiple.sending.push(sending);
        }
    }

    /// Queue one part of an unconfirmed report to `route`; its token, or
    /// `None` when it does not encode, finds no room (`crowded`) or no
    /// budget.
    fn queue_part(
        &mut self,
        route: &CovSubscriptionSnapshot,
        notification: &COVNotificationMultipleRequest,
        room: &mut Room,
        crowded: &mut bool,
        budget: &mut EventBudget,
        now: Instant,
    ) -> Option<Token> {
        let mut params = BytesMut::new();
        notification.encode(&mut params).ok()?;
        if !room.take(params.len(), None) {
            *crowded = true;
            return None;
        }
        // The token before the budget, so a part that gets none has spent
        // nothing: there is no refund to forget.
        let token = part_token(&mut self.table, route)?;
        if !budget.try_consume(params.len() + UNCONFIRMED_HEADER) {
            self.counters.cov.notifications_throttled_fanout += 1;
            return None;
        }
        let pushed = self.outbox.push(
            token,
            route.key().clone(),
            route.endpoint(),
            (false, NotifyService::CovMultiple),
            now,
            &params,
        );
        // The room was reserved above, so the push queues it.
        debug_assert!(pushed.queued, "admission reserved the room");
        pushed.queued.then_some(token)
    }

    /// The send of a queued part of an unconfirmed report ended. A part
    /// that was sent retires what it carried, and the last one ends the
    /// report; one that was not sent stops the report, and the parts after
    /// it leave the outbox. False when `token` names no such part.
    pub(super) fn part_delivered(&mut self, token: Token, outcome: Delivery, now: Instant) -> bool {
        let holds = |sending: &Sending| sending.queued.iter().any(|part| part.token == token);
        let Some(at) = self.multiple.sending.iter().position(holds) else {
            return false;
        };
        let sending = &mut self.multiple.sending[at];
        let index = sending
            .queued
            .iter()
            .position(|part| part.token == token)
            .expect("found above");
        let part = sending.queued.remove(index);
        if outcome == Delivery::Sent {
            // A reference whose latest change, or untimestamped values,
            // this part carried is complete once it is sent, whatever
            // becomes of the parts after it (#1008, #1038).
            let completes = carried(&part.finishes, &part.claim, &sending.observations, false);
            sending.delivered.extend(completes);
            self.multiple.histories.commit(part.claim);
            sending.sent = true;
            if !sending.queued.is_empty() {
                return true;
            }
            let sending = self.multiple.sending.remove(at);
            self.finish(sending, now);
        } else {
            // The part that stopped the report goes back first, then the
            // parts queued after it, which leave the outbox, then those
            // never queued.
            let mut sending = self.multiple.sending.remove(at);
            let dead: Vec<_> = sending.queued.iter().map(|part| part.token).collect();
            let mut back = vec![part.claim];
            back.extend(sending.queued.drain(..).map(|part| part.claim));
            back.append(&mut sending.unsent);
            sending.unsent = back;
            self.outbox.kill(|entry| dead.contains(&entry.token));
            self.room_freed = true;
            self.finish(sending, now);
        }
        self.multiple_counts();
        true
    }

    /// End an unconfirmed report: what was not sent goes back to its
    /// references, owing once a part went (#1038), the references the sent
    /// parts carried complete, and the turn ends.
    fn finish(&mut self, sending: Sending, now: Instant) {
        let Sending {
            context,
            turn,
            queued,
            unsent,
            sent,
            delivered,
            ..
        } = sending;
        let claims = queued.into_iter().map(|part| part.claim).chain(unsent);
        for claim in claims {
            let claim = if sent { claim.owing() } else { claim };
            self.multiple.histories.give_back(claim, now);
        }
        for (sub, observation, completion) in delivered {
            self.table
                .complete_observation(&sub, completion, observation, now);
        }
        self.end_turn(&context, turn);
    }

    /// Settle the claim a confirmed COV-multiple report lodged, before the
    /// table ends the report: delivered on the Ack, returned on any other
    /// outcome. The report's context, when `token` names such a report.
    pub(super) fn settle_lodged(
        &mut self,
        token: Token,
        outcome: Delivery,
        now: Instant,
    ) -> Option<MultipleContextKey> {
        let lodged = &mut self.multiple.lodged;
        let at = lodged.iter().position(|lodged| lodged.token == token)?;
        let lodged = lodged.swap_remove(at);
        if let Some(claim) = lodged.claim {
            let settlement = if outcome == Delivery::Acknowledged {
                Settlement::Delivered
            } else {
                Settlement::Returned
            };
            // A claim past its deadline went back already.
            self.multiple.histories.settle(claim, settlement, now);
        }
        Some(lodged.context)
    }

    /// A confirmed report to `context` failed: its history goes out once
    /// the hold-off ends.
    pub(super) fn hold_off_multiple(&mut self, context: &MultipleContextKey, now: Instant) {
        hold_off(&self.table, &mut self.multiple.histories, context, now);
    }

    /// The histories' part of [`settle`](Self::settle), for what the table
    /// `fenced`: they follow the table unless the mutation kept them in
    /// step itself, a fenced report's claim returns, and an unconfirmed
    /// report whose route left the table ends, its queued parts dying with
    /// the reference.
    pub(super) fn settle_multiple(&mut self, fenced: &Fenced, follow: bool) {
        let now = self.last;
        if follow && !fenced.is_empty() {
            follow_table(&mut self.multiple.histories, &self.table, fenced, &[], now);
        }
        for token in &fenced.tokens {
            self.settle_lodged(*token, Delivery::NotSent, now);
        }
        while let Some(at) = self
            .multiple
            .sending
            .iter()
            .position(|sending| fenced.removed.contains(&sending.route))
        {
            let mut sending = self.multiple.sending.remove(at);
            let dead: Vec<_> = sending.queued.iter().map(|part| part.token).collect();
            self.outbox.kill(|entry| dead.contains(&entry.token));
            // A follow-up, if one is owed, looks at what is left.
            if let Some(keys) = &mut sending.turn {
                keys.retain(|key| !fenced.removed.contains(key));
            }
            self.finish(sending, now);
        }
        self.multiple_counts();
    }

    /// Take what the histories counted and dropped since the last call.
    fn multiple_counts(&mut self) {
        let histories = &mut self.multiple.histories;
        let counts = histories.take_counts();
        self.counters.cov.timed_changes_dropped += counts.changes_dropped;
        self.counters.cov.untimed_references_oversized += counts.untimed_oversized;
        histories.take_drops();
        histories.take_moved();
    }

    /// The histories' next deadline: a context's Max_Notification_Delay
    /// backstop, or a lodged claim's.
    pub(super) fn multiple_deadline(&self) -> Option<Instant> {
        self.multiple.histories.next_deadline()
    }

    /// The references due at `now`, to look at again.
    pub(super) fn multiple_due(&mut self, now: Instant) -> Vec<CovSubscriptionKey> {
        self.multiple.histories.take_due(now).0
    }
}
