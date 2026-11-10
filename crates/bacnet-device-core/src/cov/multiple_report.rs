//! What one fanout reports to a COV-multiple context (135-2020 §13.16 to
//! §13.18): which of its references a report takes, what it drains from
//! their histories, how it is split, and what its sends owe afterwards.
//!
//! A host fans a change out context by context ([`by_context`]), each in
//! three steps, because it reads its objects under one guard and its table
//! under another:
//!
//! 1. [`Reading::read`] reads the fanned-out references' current values
//!    from a [`Source`], with the objects borrowed and no table.
//! 2. [`Reading::gather`] takes the table and the histories at one instant.
//!    A confirmed context with a report outstanding, or holding off after a
//!    failed one, reports nothing and drains nothing ([`Gathering::Busy`]);
//!    one whose hold-off has passed hands every reference to one follow-up
//!    ([`Gathering::FollowUp`]); an unconfirmed context sends one report at
//!    a time, and a fanout that finds one going out stands back
//!    ([`Gathering::StoodBack`]). Otherwise it issues the tickets, drains
//!    the pending timestamped changes of the whole context into one claim
//!    and takes the owed marks ([`Gathered`]).
//! 3. [`Gathered::split`] takes the histories again, with the objects still
//!    borrowed, and splits the report to the subscriber's APDU size
//!    ([`Report`]), telling the histories what the split gave up.
//!
//! The host then sends. An unconfirmed report's parts all go out, oldest
//! changes first; each sent part's claim is committed and what it completes
//! ([`carried`]) recorded, and a part that is not sent stops the rest,
//! whose claims go back, owing once a part went. A confirmed report sends
//! its first part only ([`Report::into_first`]) and begins it in the table
//! ([`begin_confirmed`]); its later parts go back to their queue for the
//! follow-up its Ack owes.
//!
//! Nothing here locks, logs, sends or reads a monotonic clock: every claim
//! handed out is plain data its holder must settle, and the send turn of an
//! unconfirmed context ([`Gathered::take_turn`]) is the host's to end.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::cell::RefCell;

use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov_multiple::{COVNotificationMultipleRequest, COVNotificationValue};
use bacnet_time::Instant;
use bacnet_types::primitives::ObjectIdentifier;

use super::multiple_chunks::{
    Envelope, Part, ReportContent, Split, request_limit, split, untimed_octets,
};
use super::multiple_items::{Coordinate, FieldStamp};
use super::multiple_reads::{MultipleReads, PreparedReference};
use super::timed::{ClaimId, FieldTiming, TimedChange, TimedClaim, TimedHistories};
use super::{
    BeginRefusal, CovObservation, CovSubscriptionKey, CovSubscriptionSnapshot, CovTable,
    CovTimeRemaining, MultipleContextKey, PreparedCovCompletion, Token,
};

/// The COV-multiple references of one fanout by context, the contexts in
/// the order they report: that of their keys, so by recipient (direct
/// before routed, then by address), then by subscriber process identifier,
/// the unconfirmed form before the confirmed one. Each context keeps its
/// references in the order given.
pub fn by_context(
    subscriptions: impl IntoIterator<Item = CovSubscriptionSnapshot>,
) -> BTreeMap<MultipleContextKey, Vec<CovSubscriptionSnapshot>> {
    let mut grouped: BTreeMap<MultipleContextKey, Vec<CovSubscriptionSnapshot>> = BTreeMap::new();
    for sub in subscriptions {
        grouped
            .entry(
                sub.key()
                    .multiple_context()
                    .expect("Multiple snapshot")
                    .clone(),
            )
            .or_default()
            .push(sub);
    }
    grouped
}

/// Where a report reads its values.
#[derive(Clone, Copy)]
pub enum Source<'a> {
    /// The database, borrowed under the host's guard. Producers capture
    /// under its write guard, so while it is borrowed no capture can move a
    /// field record past the values read here. It supplies every value, and
    /// the Device clock for a change no capture recorded.
    Database(&'a ObjectDatabase),
    /// A producer's snapshot of one object. Its changes were captured at
    /// the producer, so this path takes no clock: it neither adopts a
    /// current-state change nor times an uncaptured field (#987).
    Snapshot(&'a dyn BACnetObject),
}

impl<'a> Source<'a> {
    fn object(self, oid: ObjectIdentifier) -> Option<&'a dyn BACnetObject> {
        match self {
            Self::Database(db) => db.get(&oid),
            Self::Snapshot(object) => (object.object_identifier() == oid).then_some(object),
        }
    }

    /// The Device clock now, if it is valid; only the database has one.
    fn clock(self) -> Option<ClockFrame> {
        match self {
            Self::Database(db) => db
                .clock_frame()
                .filter(|frame| frame.is_valid_actual_datetime()),
            Self::Snapshot(_) => None,
        }
    }
}

/// What a fanned-out reference read.
enum Candidate {
    /// An untimestamped reference that qualified: its values and the
    /// observation they establish.
    Untimed(Vec<COVNotificationValue>, CovObservation),
    /// A timestamped reference, decided against its captured history under
    /// the table; its current state when the database was read.
    Timed(Option<PreparedReference>),
}

/// Step one: the current values of one context's fanned-out references.
pub struct Reading<'a> {
    source: Source<'a>,
    subscriptions: &'a [CovSubscriptionSnapshot],
    force: bool,
    /// The clock sample for any current-state fallback.
    clock_frame: Option<ClockFrame>,
    /// One read per object in this context; all selected values and
    /// companions share this borrow, never a cross-context cache.
    reads: MultipleReads,
    candidates: Vec<(&'a CovSubscriptionSnapshot, Candidate)>,
    /// Untimestamped references read now, qualifying or not: this fanout
    /// takes any owed mark they hold (#1038).
    evaluated: Vec<&'a CovSubscriptionSnapshot>,
}

impl<'a> Reading<'a> {
    /// Read `subscriptions`, the references of one context a change fans
    /// out to, from `source`. Untimestamped references qualify now, against
    /// the observation their snapshot last reported (all of them when
    /// `force`, for an initial report); timestamped ones are decided in
    /// [`gather`](Self::gather), against their captured history.
    pub fn read(
        source: Source<'a>,
        subscriptions: &'a [CovSubscriptionSnapshot],
        force: bool,
    ) -> Self {
        let clock_frame = subscriptions
            .iter()
            .any(|sub| sub.timestamped)
            .then(|| source.clock())
            .flatten();
        let mut reads = MultipleReads::default();
        for sub in subscriptions {
            if let Some(object) = source.object(sub.monitored_object_identifier) {
                reads.capture_source(object, sub);
            }
        }
        let mut candidates = Vec::new();
        let mut evaluated = Vec::new();
        for sub in subscriptions {
            let Some(object) = source.object(sub.monitored_object_identifier) else {
                continue;
            };
            if sub.timestamped {
                // A producer snapshot's changes were captured at the
                // producer; only a database preparation adds current state.
                let current = matches!(source, Source::Database(_))
                    .then(|| reads.read(object, sub))
                    .flatten();
                candidates.push((sub, Candidate::Timed(current)));
                continue;
            }
            evaluated.push(sub);
            let Some(prepared) =
                reads.prepare(object, sub, sub.last_notified_observation.as_ref(), force)
            else {
                continue;
            };
            candidates.push((
                sub,
                Candidate::Untimed(prepared.values, prepared.observation),
            ));
        }
        Self {
            source,
            subscriptions,
            force,
            clock_frame,
            reads,
            candidates,
            evaluated,
        }
    }

    /// Step two, at `now`, with the objects still borrowed: decide whether
    /// the context reports, and gather what its report conveys.
    ///
    /// Call it under the guard the values were read under, so the tickets
    /// issued here follow the order of the observations (#826). No object
    /// is read. Each prepared value owns its own check; a live sibling with
    /// a failed read cannot authorize a stale value.
    pub fn gather(
        self,
        table: &mut CovTable,
        histories: &mut TimedHistories,
        now: Instant,
    ) -> Gathering<'a> {
        let Self {
            source,
            subscriptions,
            force,
            clock_frame,
            reads,
            candidates,
            evaluated,
        } = self;
        let Some(first) = subscriptions.first() else {
            return Gathering::Nothing { unclocked: 0 };
        };
        let context = first
            .key()
            .multiple_context()
            .expect("Multiple snapshot")
            .clone();
        // A confirmed context has at most one outstanding report, and the
        // next one has to batch everything held meanwhile (#896). That
        // report's Ack, or the first fanout after a hold-off, sends it;
        // nothing is drained until then.
        if !table.context_idle(&context, subscriptions, now) {
            // Pending timestamped changes are retried when a hold-off ends,
            // not after the backstop's usual wait.
            if let Some(until) = table.context_hold_until(&context, now) {
                histories.hold_until(&context, until);
            }
            return Gathering::Busy;
        }
        // A failed report's changes can sit on any object of the context.
        // The first fanout after its hold-off hands the whole context to
        // one follow-up instead of reporting only its own object.
        if table.take_owed_context(&context, now) {
            return Gathering::FollowUp(context_keys(table, &context));
        }
        // An unconfirmed context sends one report at a time, so no later
        // report overtakes the parts of this one (#986, #1038); a confirmed
        // context is held by its outstanding report instead. A fanout that
        // finds a report going out leaves its changes queued, and that
        // report hands the context to a follow-up when done. A context
        // without timestamped references keeps the turn only for a report
        // of several parts ([`Report::needs_turn`]).
        let any_timestamped = table
            .multiple_context_references(&context)
            .any(|sub| sub.timestamped);
        let mut turn = None;
        if !context.confirmed {
            let keys = context_keys(table, &context);
            if !histories.begin_send(&context) {
                return Gathering::StoodBack;
            }
            turn = Some(keys);
        }
        // Timestamped changes drained for this report, and the
        // untimestamped references whose values it carries.
        let mut claim = TimedClaim::new();
        // Since when each evaluated untimestamped reference is owed. A mark
        // taken here without a report to carry it is settled: the reference
        // had nothing left to report.
        let owed: BTreeMap<_, _> = evaluated
            .iter()
            .filter_map(|sub| Some((sub.key(), histories.take_owed(sub.key(), sub.generation())?)))
            .collect();
        let live = |table: &CovTable, sub: &CovSubscriptionSnapshot| {
            table
                .remaining_lifetime(sub, now)
                .and_then(CovTimeRemaining::wire_seconds)
        };
        let mut unclocked = 0;
        let mut retained = Vec::new();
        for (sub, candidate) in candidates {
            let Some(remaining) = live(table, sub) else {
                continue;
            };
            let current = match candidate {
                Candidate::Untimed(values, baseline) => {
                    let Some(completion) = table.prepare_completion(sub) else {
                        continue;
                    };
                    claim.add_untimed(
                        sub.key().clone(),
                        sub.generation(),
                        owed.get(sub.key()).copied(),
                    );
                    retained.push((sub.clone(), values, baseline, completion, remaining));
                    continue;
                }
                Candidate::Timed(current) => current,
            };
            let Some(completion) = table.prepare_completion(sub) else {
                continue;
            };
            // Captured changes carry their own commit times. The current
            // state is conveyed as well only when it differs from the last
            // captured or conveyed observation (a change no producer
            // captured, such as a raw database mutation), stamped with this
            // preparation's clock.
            let (incarnation, mut changes) = histories.drain(sub.key(), sub.generation());
            // The histories' baseline is the newest captured or conveyed
            // state; a returned older change can sit at the tail of
            // `changes`.
            let baseline = histories
                .baseline(sub.key(), sub.generation())
                .cloned()
                .or_else(|| changes.last().map(|change| change.observation().clone()))
                .or_else(|| sub.last_notified_observation.clone());
            // An admission capture already supplied the initial report.
            let force = force
                && changes.is_empty()
                && histories.baseline(sub.key(), sub.generation()).is_none();
            let current =
                current.filter(|current| force || reads.reports(current, baseline.as_ref()));
            match (current, clock_frame) {
                (Some(prepared), Some(frame)) => {
                    let values = reads
                        .with_flags_companion(&sub.monitored_object_identifier, prepared.values);
                    changes.push(histories.adopt(
                        sub.key(),
                        sub.generation(),
                        TimedChange::new(frame, values, prepared.observation, now),
                    ));
                }
                // Without a valid Device clock the change has no time to
                // carry: skipped, and counted for the host.
                (Some(_), None) => unclocked += 1,
                (None, _) => {}
            }
            let Some(last) = changes.last().map(|change| change.observation().clone()) else {
                continue;
            };
            claim.add(sub.key().clone(), incarnation, changes);
            retained.push((sub.clone(), Vec::new(), last, completion, remaining));
        }
        // Every notification to a context conveys all of its pending
        // timestamped changes (§§13.17.1.1, 13.18.1.1), including those of
        // references on objects that did not change now. A confirmed
        // context starts none while a report is outstanding, so this holds
        // for it too. Captured values need no object read.
        let mut untimed = BTreeSet::new();
        let mut others = Vec::new();
        for other in table.multiple_context_references(&context) {
            if !other.timestamped {
                untimed.insert((
                    other.monitored_object_identifier,
                    other.monitored_property,
                    other.monitored_property_array_index,
                ));
                continue;
            }
            if subscriptions.iter().any(|sub| sub.key() == other.key()) {
                continue;
            }
            let Some(remaining) = live(table, other) else {
                continue;
            };
            others.push((other.clone(), remaining));
        }
        for (other, remaining) in others {
            let Some(completion) = table.prepare_completion(&other) else {
                continue;
            };
            let (incarnation, changes) = histories.drain(other.key(), other.generation());
            let Some(last) = changes.last().map(|change| change.observation().clone()) else {
                continue;
            };
            claim.add(other.key().clone(), incarnation, changes);
            retained.push((other, Vec::new(), last, completion, remaining));
        }
        let subscriber_max_apdu = table
            .multiple_context_references(&context)
            .find_map(|sub| sub.subscriber_max_apdu());
        // A live timestamped reference that conveys no change in the last
        // notification still times its field when a sibling carries it
        // (#987). Changes drained now can all go out in an earlier part
        // (#1008), so every live one is an owner; one whose latest change
        // the last notification carries times its field itself.
        let owners: BTreeMap<Coordinate, (CovSubscriptionKey, u64)> = table
            .multiple_context_references(&context)
            .filter(|other| other.timestamped && live(table, other).is_some())
            .map(|other| {
                (
                    (
                        other.monitored_object_identifier,
                        other.monitored_property,
                        other.monitored_property_array_index,
                    ),
                    (other.key().clone(), other.generation()),
                )
            })
            .collect();
        // References an earlier report left owed go first, so a split
        // report cannot keep deferring them (#1038).
        retained.sort_by_key(|(sub, ..)| !owed.contains_key(sub.key()));
        if retained.is_empty() {
            // Nothing drained and nothing prepared: the claim is empty. The
            // turn ends where it began, under the same borrow of the
            // histories, so no fanout stood back meanwhile.
            debug_assert!(claim.is_empty());
            if turn.is_some() {
                histories.end_send(&context);
            }
            return Gathering::Nothing { unclocked };
        }
        Gathering::Ready(Box::new(Gathered {
            source,
            context,
            turn,
            any_timestamped,
            claim,
            retained,
            untimed,
            subscriber_max_apdu,
            owners,
            reads,
            unclocked,
            now,
        }))
    }
}

/// The keys of every reference of `context`.
fn context_keys(table: &CovTable, context: &MultipleContextKey) -> Vec<CovSubscriptionKey> {
    table
        .multiple_context_references(context)
        .map(|sub| sub.key().clone())
        .collect()
}

/// What [`Reading::gather`] decided.
pub enum Gathering<'a> {
    /// A confirmed context whose report is outstanding, or which holds off
    /// after a failed one: nothing is drained. That report's Ack, or the
    /// first fanout after the hold-off, reports what is held; while it
    /// holds off, the histories' backstop waits for the hold-off's end.
    Busy,
    /// A confirmed context whose failed report's hold-off has passed: the
    /// host fans these references, all of the context's, out in one
    /// follow-up, since the failed report's changes may sit on objects
    /// other than the one fanned out now.
    FollowUp(Vec<CovSubscriptionKey>),
    /// An unconfirmed context with a report going out. The histories noted
    /// that this fanout stood back, and that report's host owes the
    /// follow-up when its turn ends.
    StoodBack,
    /// Nothing to report. A send turn taken on the way has ended.
    Nothing {
        /// Timestamped current-state changes skipped for want of a valid
        /// Device clock, for the host to log.
        unclocked: usize,
    },
    /// A report to split and send.
    Ready(Box<Gathered<'a>>),
}

/// One reference a report retains: its snapshot, its prepared current
/// values (none for a timestamped reference), the observation its delivery
/// establishes, its ticket and the seconds of lifetime it has left.
type RetainedReference = (
    CovSubscriptionSnapshot,
    Vec<COVNotificationValue>,
    CovObservation,
    PreparedCovCompletion,
    u32,
);

/// Step two's result: what one report to a context conveys, before it is
/// split. It holds a claim on the drained changes as plain data, and, for
/// an unconfirmed context, the context's send turn: until
/// [`split`](Self::split) returns and the host has taken the turn
/// ([`take_turn`](Self::take_turn)), nothing gives either back.
pub struct Gathered<'a> {
    source: Source<'a>,
    context: MultipleContextKey,
    /// The context's references, while this report holds its send turn.
    turn: Option<Vec<CovSubscriptionKey>>,
    any_timestamped: bool,
    claim: TimedClaim,
    retained: Vec<RetainedReference>,
    /// The coordinates the context subscribes without timestamps.
    untimed: BTreeSet<Coordinate>,
    subscriber_max_apdu: Option<u16>,
    owners: BTreeMap<Coordinate, (CovSubscriptionKey, u64)>,
    reads: MultipleReads,
    unclocked: usize,
    now: Instant,
}

impl Gathered<'_> {
    /// The context the report goes to.
    pub fn context(&self) -> &MultipleContextKey {
        &self.context
    }

    /// Timestamped current-state changes skipped for want of a valid Device
    /// clock, for the host to log.
    pub fn unclocked(&self) -> usize {
        self.unclocked
    }

    /// Take over the send turn of an unconfirmed context, which this report
    /// holds since [`Reading::gather`] ([`TimedHistories::begin_send`]): the
    /// context's references, for the follow-up. `None` for a confirmed
    /// context, which has no turn, or once taken.
    ///
    /// The host ends the turn ([`TimedHistories::end_send`]) after the
    /// report's last part, when the report stops early, or at once when
    /// the report turns out not to need it ([`Report::needs_turn`]); if a
    /// fanout stood back meanwhile, it fans these references out again.
    pub fn take_turn(&mut self) -> Option<Vec<CovSubscriptionKey>> {
        self.turn.take()
    }

    /// Step three: split the report to the subscriber's APDU size, as
    /// `device` sending with a local maximum APDU of `local_apdu` octets.
    ///
    /// The histories are told what the split gave up: one drop for each
    /// change that lost a value too large for any notification, the
    /// untimestamped references left out because their values fit none
    /// ([`Report::left_out`]), and the changes of a last notification that
    /// would convey nothing, which go back to their references. The fields
    /// a sibling carries for a timestamped reference that conveys no change
    /// are timed from the histories, and from the Device clock for a value
    /// no capture recorded (#987).
    pub fn split(
        self,
        histories: &mut TimedHistories,
        device: ObjectIdentifier,
        local_apdu: u32,
    ) -> Report {
        let Self {
            source,
            context,
            turn: _,
            any_timestamped,
            claim,
            retained,
            untimed,
            subscriber_max_apdu,
            owners,
            reads,
            unclocked: _,
            now,
        } = self;
        let (route, _, _, _, time_remaining) = retained.first().expect("a retained reference");
        let route = route.clone();
        let envelope = Envelope {
            subscriber_process_identifier: route.subscriber_process_identifier,
            initiating_device_identifier: device,
            time_remaining: *time_remaining,
        };
        let parts: Vec<_> = retained
            .iter()
            .map(|(sub, values, _, _, _)| (sub, values.as_slice()))
            .collect();
        // The untimestamped values travel with the last notification, so
        // the history bound keeps room for them (#986).
        histories.note_reserve(&context, untimed_octets(&parts));
        let limit = request_limit(
            local_apdu,
            subscriber_max_apdu,
            route.issue_confirmed_notifications,
        );
        // The split asks for field times while it sizes notifications, so
        // the histories are lent to it through a cell.
        let lent = RefCell::new(&mut *histories);
        // Times a field of a timestamped reference that conveys no change
        // now, when a sibling carries it (#987).
        let stamp = |coordinate: &Coordinate, value: &[u8]| {
            let Some((key, generation)) = owners.get(coordinate) else {
                return FieldStamp::Unowned;
            };
            // The clock is read only for a value no capture recorded, and
            // only on the database path.
            let timing = lent
                .borrow_mut()
                .field_time(key, *generation, value, || source.clock());
            match timing {
                FieldTiming::NotLive => FieldStamp::Unowned,
                FieldTiming::NoTime => FieldStamp::Withhold,
                FieldTiming::At(seq, frame) => FieldStamp::At((seq, frame)),
            }
        };
        let content = ReportContent {
            envelope: &envelope,
            retained: &parts,
            reads: &reads,
            untimed: &untimed,
            stamp: &stamp,
        };
        let Split {
            parts,
            lost,
            mut forgone,
            rest,
        } = split(&content, claim, limit);
        histories.lost_values(&lost);
        let left_out: Vec<_> = forgone.untimed().cloned().collect();
        if !left_out.is_empty() {
            histories.forgo_untimed(&mut forgone);
        }
        // A last notification that would convey nothing leaves its changes
        // over: they go back to their references.
        histories.give_back(rest, now);
        // Untimestamped values a single notification carries need no turn;
        // reports of them may go out side by side.
        let needs_turn = !context.confirmed && (any_timestamped || parts.len() >= 2);
        Report {
            route,
            parts,
            observations: retained
                .into_iter()
                .map(|(sub, _, baseline, completion, _)| (sub, baseline, completion))
                .collect(),
            needs_turn,
            left_out,
            limit,
        }
    }
}

/// A reference a report carries, with the observation its delivery
/// establishes and the ticket that completes it.
pub type Carried = (
    CovSubscriptionSnapshot,
    CovObservation,
    PreparedCovCompletion,
);

/// One report, split into the notifications that convey it. Each part's
/// claim is plain data: its holder commits it once the part is delivered
/// and gives it back otherwise.
pub struct Report {
    /// The reference whose recipient and current route receive the report.
    /// Its `issue_confirmed_notifications` is the context's form.
    pub route: CovSubscriptionSnapshot,
    /// The notifications, in the order they go out. None when every
    /// timestamped value was too large to send and nothing else changed.
    pub parts: Vec<Part>,
    /// Every reference the report retains; [`carried`] picks those a part
    /// completes.
    pub observations: Vec<Carried>,
    /// Whether an unconfirmed context's report has to keep the send turn
    /// until its last part is sent: one with timestamped references, or of
    /// several parts. Untimestamped values that one notification carries
    /// need none, and the host ends a turn it took at once. Always false
    /// for a confirmed context.
    pub needs_turn: bool,
    /// Untimestamped references left out because their values exceed the
    /// notification size on their own (#1066), for the host to log. The
    /// histories counted them; they are not owed.
    pub left_out: Vec<CovSubscriptionKey>,
    /// Service-request octets one notification of the report may take.
    pub limit: usize,
}

/// The references a part completes, each with its observation: those it
/// finishes (`finishes`), at their latest drained change or prepared state,
/// and, when `every` (a confirmed part's Ack completes all it carries),
/// each other timestamped reference at the last change of it in the part's
/// `claim`.
pub fn carried(
    finishes: &BTreeSet<CovSubscriptionKey>,
    claim: &TimedClaim,
    observations: &[Carried],
    every: bool,
) -> Vec<Carried> {
    observations
        .iter()
        .filter_map(|(sub, baseline, completion)| {
            if finishes.contains(sub.key()) {
                return Some((sub.clone(), baseline.clone(), *completion));
            }
            let (_, change) = claim
                .last_changes()
                .filter(|_| every)
                .find(|(key, _)| *key == sub.key())?;
            Some((sub.clone(), change.observation().clone(), *completion))
        })
        .collect()
}

/// The part of a confirmed report that goes out now, and the rest.
pub struct FirstPart {
    /// The reference whose recipient and current route receive it.
    pub route: CovSubscriptionSnapshot,
    /// The notification's service request.
    pub notification: COVNotificationMultipleRequest,
    /// The report's one ticket: the newest prepared one, which postdates
    /// every carried reference's baseline, so it completes them all.
    pub completion: PreparedCovCompletion,
    /// Each reference the part carries, with the observation its Ack
    /// completes.
    pub observations: Vec<(CovSubscriptionSnapshot, CovObservation)>,
    /// What the part's Ack retires.
    pub claim: TimedClaim,
    /// The later parts' claims. They return to their queue, uncounted,
    /// once the report holds the context ([`begin_confirmed`]), and the
    /// Ack's follow-up sends the next part (#986). Deferred, not dropped:
    /// requeueing them must not let the bound evict what this report
    /// planned to send. The values of a change sent one per notification
    /// rejoin there as one change (#1090). Untimestamped references of
    /// later parts are owed meanwhile, and the follow-up reads their values
    /// afresh, so a newer change to one goes in their place (#1038).
    pub deferred: Vec<TimedClaim>,
}

impl Report {
    /// For a confirmed context: only the oldest part goes now. `None` when
    /// the report has no part.
    pub fn into_first(self) -> Option<FirstPart> {
        let mut parts = self.parts.into_iter();
        let first = parts.next()?;
        let deferred = parts.map(|part| part.claim.without_eviction()).collect();
        let observations = carried(&first.finishes, &first.claim, &self.observations, true);
        Some(FirstPart {
            route: self.route,
            notification: first.notification,
            completion: observations
                .iter()
                .map(|(_, _, completion)| *completion)
                .max_by_key(|completion| completion.ticket())
                .expect("a carried reference"),
            observations: observations
                .into_iter()
                .map(|(sub, observation, _)| (sub, observation))
                .collect(),
            claim: first.claim,
            deferred,
        })
    }
}

/// A confirmed report the table began.
pub struct Begun {
    /// The report: its outcome goes back with it ([`CovTable::delivered`]).
    pub token: Token,
    /// The claim on what the report conveys, lodged with the histories
    /// until the report's own deadline: its holder settles it when the
    /// report ends ([`TimedHistories::settle`]), delivered on the Ack and
    /// returned otherwise, and the histories give it back at the deadline
    /// if nobody did.
    pub claim: Option<ClaimId>,
}

/// Begin one confirmed notification at `now`: mark its coordinate
/// outstanding under `completion`, keeping the observation each reference
/// carries for the Ack ([`CovTable::begin_confirmed`]), and settle what it
/// holds of the histories.
///
/// Once the report holds its coordinate, a change `claim` sends only some
/// values of keeps the rest queued until it is delivered (#1163), the
/// `deferred` parts go back to their queue with their untimestamped
/// references owed (#1038), and `claim` is lodged until the report's
/// deadline, read off the table, so both end at one instant.
///
/// A refusal gives everything back, before any follow-up can drain the same
/// references again, so the history stays in capture order.
/// [`BeginRefusal::NotCurrent`] means a fence moved a reference since it
/// was captured: the host looks at the references again. A busy coordinate
/// needs nothing: its outstanding report, Ack or hold-off owns the
/// follow-up.
pub fn begin_confirmed(
    table: &mut CovTable,
    histories: &mut TimedHistories,
    completion: PreparedCovCompletion,
    observations: &[(CovSubscriptionSnapshot, CovObservation)],
    claim: Option<TimedClaim>,
    deferred: Vec<TimedClaim>,
    now: Instant,
) -> Result<Begun, BeginRefusal> {
    let begun = table.begin_confirmed(
        completion,
        observations
            .iter()
            .map(|(sub, observation)| (sub, observation.clone())),
        now,
    );
    match begun {
        Ok(token) => {
            if let Some(claim) = &claim {
                histories.going_out(claim);
            }
            for deferred in deferred {
                histories.give_back(deferred.owing(), now);
            }
            // Were the report gone already, its claim ends now.
            let deadline = table.report_deadline(token).unwrap_or(now);
            Ok(Begun {
                token,
                claim: claim.map(|claim| histories.lodge(claim, deadline)),
            })
        }
        Err(refusal) => {
            if let Some(claim) = claim {
                histories.give_back(claim, now);
            }
            for deferred in deferred {
                histories.give_back(deferred, now);
            }
            Err(refusal)
        }
    }
}

/// A confirmed report to `context` failed at `now` (its retries ran out, or
/// the subscriber refused it): the histories' backstop waits one hold-off,
/// the table's, before it hands the context out again, so the timestamped
/// history the report returned goes out once the hold-off ends.
pub fn hold_off(
    table: &CovTable,
    histories: &mut TimedHistories,
    context: &MultipleContextKey,
    now: Instant,
) {
    histories.hold_until(
        context,
        now.saturating_add(table.flight_timing().hold_off()),
    );
}
