//! The report steps on hand-set instants: which contexts report and in what
//! order, which references a report takes, what a busy context defers, what
//! is owed, when a send turn begins and ends, how a confirmed report begins
//! and what a refused or failed one gives back.
use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::cov_multiple::{COVNotificationMultipleRequest, COVReference};
use bacnet_time::{Duration, Instant};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;
use crate::cov::multiple_fixture::{
    HandClock, PV, T0, at, av, context, database, frame, peer, reference, request, set,
};
use crate::cov::timed::Settlement;
use crate::cov::timed_capture::TimedCapture;
use crate::cov::{Delivery, subscribe_cov_property_multiple};

/// Max_Notification_Delay of every context here, in seconds.
pub(super) const DELAY: u32 = 5;
/// The local maximum APDU.
const LOCAL: u32 = 1476;

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 7).unwrap()
}

/// A device's reporting state, held outright as a firmware host holds it.
pub(super) struct Host {
    pub(super) table: CovTable,
    pub(super) histories: TimedHistories,
    pub(super) db: ObjectDatabase,
    pub(super) clock: Arc<HandClock>,
}

impl Host {
    /// Analog Values 1 to `objects`, the Device clock at second 1.
    pub(super) fn new(objects: u32) -> Self {
        let clock = HandClock::showing(1);
        Self {
            table: CovTable::default(),
            histories: TimedHistories::new(LOCAL as usize),
            db: database(Some(&clock), objects),
            clock,
        }
    }

    /// Subscribe `process`'s context at [`T0`] for 300 s: `references` on
    /// each of Analog Values 1 to `objects`, for a subscriber accepting
    /// `max_apdu` octets.
    pub(super) fn subscribe(
        &mut self,
        process: u32,
        confirmed: bool,
        objects: u32,
        references: &[COVReference],
        max_apdu: Option<u16>,
    ) -> Vec<CovSubscriptionSnapshot> {
        let named: Vec<_> = (1..=objects)
            .map(|instance| (instance, references))
            .collect();
        let mut request = request(process, Some(300), Some(DELAY), &named);
        request.issue_confirmed_notifications = confirmed;
        subscribe_cov_property_multiple(
            &mut self.table,
            &mut self.histories,
            &self.db,
            &peer(),
            max_apdu,
            request,
            T0,
        )
        .unwrap()
    }

    /// The live references on Analog Value `instance`.
    pub(super) fn on(&mut self, instance: u32, now: Instant) -> Vec<CovSubscriptionSnapshot> {
        self.table
            .subscriptions_for(&av(instance), now)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Every reference of `context`, as the table holds it now.
    pub(super) fn all(&self, context: &MultipleContextKey) -> Vec<CovSubscriptionSnapshot> {
        self.table
            .multiple_context_references(context)
            .cloned()
            .collect()
    }

    /// Change Analog Value `instance` at Device second `second` and capture
    /// it at `now`, as a producer does.
    pub(super) fn change(&mut self, instance: u32, value: f32, second: u8, now: Instant) {
        set(&mut self.db, instance, value);
        self.clock.set(Some(frame(second)));
        TimedCapture::object(&self.table, av(instance), now).run(
            &self.db,
            &mut self.histories,
            now,
        );
    }

    /// The three steps for `subs`, one context's references, at `now`.
    pub(super) fn fan_out(
        &mut self,
        subs: &[CovSubscriptionSnapshot],
        force: bool,
        now: Instant,
    ) -> Outcome {
        let reading = Reading::read(Source::Database(&self.db), subs, force);
        match reading.gather(&mut self.table, &mut self.histories, now) {
            Gathering::Busy => Outcome::Busy,
            Gathering::FollowUp(keys) => Outcome::FollowUp(keys),
            Gathering::StoodBack => Outcome::StoodBack,
            Gathering::Nothing { unclocked } => Outcome::Nothing(unclocked),
            Gathering::Ready(mut gathered) => {
                let turn = gathered.take_turn();
                let report = gathered.split(&mut self.histories, device(), LOCAL);
                Outcome::Ready(Box::new(report), turn)
            }
        }
    }

    /// Send every part of an unconfirmed `report`, each send succeeding,
    /// and end its turn: true when a fanout stood back meanwhile.
    fn send_all(
        &mut self,
        report: Report,
        turn: Option<Vec<CovSubscriptionKey>>,
        now: Instant,
    ) -> (Vec<COVNotificationMultipleRequest>, bool) {
        let context = report.route.key().multiple_context().unwrap().clone();
        let mut sent = Vec::new();
        for part in report.parts {
            for (sub, observation, completion) in
                carried(&part.finishes, &part.claim, &report.observations, false)
            {
                self.table
                    .complete_observation(&sub, completion, observation, now);
            }
            self.histories.commit(part.claim);
            sent.push(part.notification);
        }
        let owed = turn.is_some() && self.histories.end_send(&context);
        (sent, owed)
    }

    /// Fan `subs` out as an initial report and send it.
    pub(super) fn initial(
        &mut self,
        subs: &[CovSubscriptionSnapshot],
    ) -> Vec<COVNotificationMultipleRequest> {
        let (report, turn) = self.fan_out(subs, true, T0).ready();
        self.send_all(report, turn, T0).0
    }
}

pub(super) enum Outcome {
    Busy,
    FollowUp(Vec<CovSubscriptionKey>),
    StoodBack,
    Nothing(usize),
    Ready(Box<Report>, Option<Vec<CovSubscriptionKey>>),
}

impl Outcome {
    fn name(&self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::FollowUp(_) => "follow-up",
            Self::StoodBack => "stood back",
            Self::Nothing(_) => "nothing",
            Self::Ready(..) => "ready",
        }
    }

    pub(super) fn ready(self) -> (Report, Option<Vec<CovSubscriptionKey>>) {
        match self {
            Self::Ready(report, turn) => (*report, turn),
            other => panic!("expected a report, got {}", other.name()),
        }
    }
}

/// `process`'s context of [`peer`], in either form.
pub(super) fn ctx(process: u32, confirmed: bool) -> MultipleContextKey {
    MultipleContextKey {
        confirmed,
        ..context(process)
    }
}

/// Each Present_Value a notification carries: the Analog Value's instance,
/// the REAL, and the Device second of its Time_Of_Change.
pub(super) fn rows(notification: &COVNotificationMultipleRequest) -> Vec<(u32, f32, Option<u8>)> {
    let mut rows = Vec::new();
    for item in &notification.list_of_cov_notifications {
        for value in &item.list_of_values {
            if value.property_identifier == PV {
                // An application-tagged REAL: its tag, then four octets.
                let octets: [u8; 4] = value.value[1..5].try_into().unwrap();
                rows.push((
                    item.monitored_object_identifier.instance_number(),
                    f32::from_be_bytes(octets),
                    value.time_of_change.map(|time| time.second),
                ));
            }
        }
    }
    rows
}

pub(super) fn seconds(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn contexts_report_in_the_order_of_their_keys() {
    let mut host = Host::new(1);
    let timed = [reference(PV, true)];
    // Subscribed out of order, and process 2 in both forms.
    for (process, confirmed) in [(3, false), (2, true), (1, false), (2, false)] {
        host.subscribe(process, confirmed, 1, &timed, None);
    }
    let grouped = by_context(host.on(1, T0));
    let order: Vec<_> = grouped
        .keys()
        .map(|context| (context.process_id, context.confirmed))
        .collect();
    assert_eq!(order, [(1, false), (2, false), (2, true), (3, false)]);
    assert!(grouped.values().all(|subs| subs.len() == 1));
}

#[test]
fn an_untimestamped_reference_reports_a_change_from_its_baseline_and_needs_no_turn() {
    let mut host = Host::new(2);
    let subs = host.subscribe(1, false, 2, &[reference(PV, false)], None);
    let initial = host.initial(&subs);
    assert_eq!(rows(&initial[0]), [(1, 0.0, None), (2, 0.0, None)]);

    // Unchanged: nothing to report, and the turn taken on the way ended.
    let on = host.on(1, at(1));
    assert!(matches!(
        host.fan_out(&on, false, at(1)),
        Outcome::Nothing(0)
    ));
    assert!(host.histories.begin_send(&ctx(1, false)), "no turn is held");
    host.histories.end_send(&ctx(1, false));

    set(&mut host.db, 1, 4.0);
    let on = host.on(1, at(2));
    let (report, turn) = host.fan_out(&on, false, at(2)).ready();
    assert_eq!(report.parts.len(), 1);
    assert_eq!(rows(&report.parts[0].notification), [(1, 4.0, None)]);
    assert_eq!(report.limit, LOCAL as usize - 2);
    // Untimestamped values in one notification need no turn: the host ends
    // the one it took at once.
    assert!(turn.is_some() && !report.needs_turn);
    host.send_all(report, turn, at(2));
    // Delivered: the same value reports nothing again.
    let on = host.on(1, at(3));
    assert!(matches!(
        host.fan_out(&on, false, at(3)),
        Outcome::Nothing(0)
    ));
}

#[test]
fn a_report_takes_every_pending_change_of_its_context() {
    let mut host = Host::new(3);
    let subs = host.subscribe(1, false, 3, &[reference(PV, true)], None);
    host.initial(&subs);

    host.change(2, 20.0, 10, at(1));
    host.change(1, 10.0, 11, at(2));
    // Only Analog Value 1 fans out, yet the report conveys the change of
    // Analog Value 2 as well; Analog Value 3 has none.
    let on = host.on(1, at(2));
    let (report, turn) = host.fan_out(&on, false, at(2)).ready();
    assert_eq!(report.parts.len(), 1);
    let mut carried_rows = rows(&report.parts[0].notification);
    carried_rows.sort_by_key(|row| row.0);
    assert_eq!(carried_rows, [(1, 10.0, Some(11)), (2, 20.0, Some(10))]);
    let retained: Vec<_> = report
        .observations
        .iter()
        .map(|(sub, _, _)| sub.monitored_object_identifier.instance_number())
        .collect();
    assert_eq!(retained, [1, 2]);
    // A context with timestamped references keeps its turn for the sends.
    assert!(report.needs_turn && turn.is_some());
    host.send_all(report, turn, at(2));
    assert_eq!(host.histories.next_deadline(), None, "all delivered");
}

#[test]
fn an_uncaptured_change_takes_the_device_clock_and_none_without_one() {
    let mut host = Host::new(1);
    let subs = host.subscribe(1, false, 1, &[reference(PV, true)], None);
    host.initial(&subs);

    // A raw database mutation, which no producer captured.
    set(&mut host.db, 1, 5.0);
    host.clock.set(Some(frame(7)));
    let on = host.on(1, at(1));
    let (report, turn) = host.fan_out(&on, false, at(1)).ready();
    assert_eq!(rows(&report.parts[0].notification), [(1, 5.0, Some(7))]);
    host.send_all(report, turn, at(1));

    // Without a valid Device clock the change has no time to carry: it is
    // skipped and counted, and the turn taken on the way has ended.
    set(&mut host.db, 1, 6.0);
    host.clock.set(None);
    let on = host.on(1, at(2));
    assert!(matches!(
        host.fan_out(&on, false, at(2)),
        Outcome::Nothing(1)
    ));

    // A producer's snapshot takes no clock at all, valid or not.
    host.clock.set(Some(frame(9)));
    let object = host.db.get(&av(1)).unwrap();
    let reading = Reading::read(Source::Snapshot(object), &on, false);
    let gathering = reading.gather(&mut host.table, &mut host.histories, at(3));
    assert!(matches!(gathering, Gathering::Nothing { unclocked: 0 }));

    // The database path adopts it once the clock is back.
    let (report, _) = host.fan_out(&on, false, at(4)).ready();
    assert_eq!(rows(&report.parts[0].notification), [(1, 6.0, Some(9))]);
}

#[test]
fn an_unconfirmed_context_sends_one_report_at_a_time() {
    let mut host = Host::new(2);
    let subs = host.subscribe(1, false, 2, &[reference(PV, true)], None);
    host.initial(&subs);

    host.change(1, 1.0, 10, at(1));
    let on = host.on(1, at(1));
    let (report, turn) = host.fan_out(&on, false, at(1)).ready();
    // The turn names the context's references, for the follow-up.
    let mut keys = turn.clone().expect("an unconfirmed context's turn");
    keys.sort();
    let mut expected: Vec<_> = subs.iter().map(|sub| sub.key().clone()).collect();
    expected.sort();
    assert_eq!(keys, expected);

    // A second fanout while the report is going out stands back, and
    // leaves its change queued.
    host.change(2, 2.0, 11, at(2));
    let other = host.on(2, at(2));
    assert!(matches!(
        host.fan_out(&other, false, at(2)),
        Outcome::StoodBack
    ));
    assert_eq!(
        host.histories.next_deadline(),
        Some(at(2) + seconds(u64::from(DELAY))),
        "the change is still pending"
    );

    // The report's end owes the follow-up, which then carries the change.
    let (sent, owed) = host.send_all(report, turn, at(2));
    assert_eq!(rows(&sent[0]), [(1, 1.0, Some(10))]);
    assert!(owed, "a fanout stood back");
    let (report, turn) = host.fan_out(&other, false, at(3)).ready();
    assert_eq!(rows(&report.parts[0].notification), [(2, 2.0, Some(11))]);
    let (_, owed) = host.send_all(report, turn, at(3));
    assert!(!owed);
}

/// A confirmed context with a timestamped Present_Value reference on each
/// of two Analog Values, its initial report begun at [`T0`].
fn confirmed_with_a_report_out() -> (Host, Vec<CovSubscriptionSnapshot>, Begun) {
    let mut host = Host::new(2);
    let subs = host.subscribe(1, true, 2, &[reference(PV, true)], None);
    let (report, turn) = host.fan_out(&subs, true, T0).ready();
    assert!(
        turn.is_none() && !report.needs_turn,
        "no turn when confirmed"
    );
    let first = report.into_first().expect("a part");
    assert!(first.deferred.is_empty());
    let begun = begin_confirmed(
        &mut host.table,
        &mut host.histories,
        first.completion,
        &first.observations,
        Some(first.claim),
        first.deferred,
        T0,
    )
    .expect("an idle context");
    (host, subs, begun)
}

#[test]
fn a_confirmed_report_lodges_its_claim_until_the_reports_own_deadline() {
    let (mut host, _, begun) = confirmed_with_a_report_out();
    let deadline = host.table.report_deadline(begun.token);
    assert_eq!(deadline, Some(T0 + host.table.flight_timing().deadline()));
    assert_eq!(host.histories.next_deadline(), deadline, "the lodged claim");
    // The Ack retires what the report conveyed.
    let claim = begun.claim.expect("a lodged claim");
    assert!(host.histories.settle(claim, Settlement::Delivered, at(1)));
    assert_eq!(host.histories.next_deadline(), None);
}

#[test]
fn a_busy_confirmed_context_drains_nothing() {
    let (mut host, _, begun) = confirmed_with_a_report_out();
    host.change(1, 1.0, 10, at(1));
    let on = host.on(1, at(1));
    assert!(matches!(host.fan_out(&on, false, at(1)), Outcome::Busy));

    // The Ack owes every reference of the context a fresh look, and that
    // report carries the change held meanwhile.
    host.histories
        .settle(begun.claim.unwrap(), Settlement::Delivered, at(2));
    let owed = host
        .table
        .delivered(begun.token, Delivery::Acknowledged, at(2));
    assert_eq!(owed.len(), 2, "the whole context");
    let all = host.all(&ctx(1, true));
    let (report, _) = host.fan_out(&all, false, at(2)).ready();
    assert_eq!(rows(&report.parts[0].notification), [(1, 1.0, Some(10))]);
}

#[test]
fn a_failed_report_holds_the_backstop_off_and_then_owes_one_follow_up() {
    let (mut host, _, begun) = confirmed_with_a_report_out();
    let hold_off_time = host.table.flight_timing().hold_off();
    assert!(seconds(u64::from(DELAY)) < hold_off_time);
    // The report fails at second 2: its claim goes back before the mark
    // clears, and the backstop waits one hold-off.
    let failed = at(2);
    host.histories
        .settle(begun.claim.unwrap(), Settlement::Returned, failed);
    host.table.delivered(begun.token, Delivery::Failed, failed);
    hold_off(&host.table, &mut host.histories, &ctx(1, true), failed);
    let until = failed + hold_off_time;

    // The returned changes are overdue by their delay well before the
    // hold-off ends, yet the backstop does not hand the context out.
    let before = until - Duration::from_millis(1);
    assert_eq!(host.histories.take_due(before), (Vec::new(), Some(until)));
    // A fanout in the hold-off finds the context busy and drains nothing.
    host.change(2, 2.0, 12, at(3));
    let on = host.on(2, at(3));
    assert!(matches!(host.fan_out(&on, false, at(3)), Outcome::Busy));
    assert_eq!(host.histories.take_due(before), (Vec::new(), Some(until)));

    // At its end the backstop hands out the references with changes, and
    // the first fanout hands the whole context to one follow-up.
    let (due, _) = host.histories.take_due(until);
    assert_eq!(due.len(), 2);
    assert!(matches!(host.fan_out(&on, false, until), Outcome::FollowUp(keys) if keys.len() == 2));
    // Owed once: the follow-up itself reports, initial values and change.
    let all = host.all(&ctx(1, true));
    let (report, _) = host.fan_out(&all, false, until).ready();
    let mut reported = rows(&report.parts[0].notification);
    reported.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        reported,
        [(1, 0.0, Some(1)), (2, 0.0, Some(1)), (2, 2.0, Some(12))]
    );
}

#[test]
fn a_busy_context_in_its_hold_off_moves_the_backstop_to_the_hold_offs_end() {
    let (mut host, _, begun) = confirmed_with_a_report_out();
    let failed = at(2);
    host.histories
        .settle(begun.claim.unwrap(), Settlement::Returned, failed);
    host.table.delivered(begun.token, Delivery::Failed, failed);
    // The host did not move the backstop itself: the returned changes are
    // due by their delay alone.
    let overdue = T0 + seconds(u64::from(DELAY));
    assert_eq!(host.histories.next_deadline(), Some(overdue));
    // The fanout that finds the context holding off moves it.
    let all = host.all(&ctx(1, true));
    assert!(matches!(host.fan_out(&all, false, overdue), Outcome::Busy));
    let until = failed + host.table.flight_timing().hold_off();
    assert_eq!(host.histories.next_deadline(), Some(until));
}

#[test]
fn a_refused_confirmed_report_gives_everything_back() {
    let mut host = Host::new(1);
    let subs = host.subscribe(1, true, 1, &[reference(PV, true)], None);
    let (report, _) = host.fan_out(&subs, true, T0).ready();
    let first = report.into_first().unwrap();
    assert_eq!(host.histories.next_deadline(), None, "drained");
    // A renewal replaces the reference before the report begins.
    host.subscribe(1, true, 1, &[reference(PV, true)], None);
    let refusal = begin_confirmed(
        &mut host.table,
        &mut host.histories,
        first.completion,
        &first.observations,
        Some(first.claim),
        first.deferred,
        at(1),
    )
    .err();
    assert_eq!(refusal, Some(BeginRefusal::NotCurrent));
    // Nothing is lodged, and the drained change is pending again beside
    // the renewal's own initial capture.
    let renewed = host.on(1, at(1));
    let (_, changes) = host
        .histories
        .drain(renewed[0].key(), renewed[0].generation());
    assert_eq!(changes.len(), 2);
}

/// Twenty untimestamped references, one per Analog Value, for a subscriber
/// whose 206-octet APDU carries eight items: the initial report takes three
/// notifications.
fn twenty(confirmed: bool) -> (Host, Vec<CovSubscriptionSnapshot>) {
    let mut host = Host::new(20);
    let subs = host.subscribe(1, confirmed, 20, &[reference(PV, false)], Some(206));
    (host, subs)
}

fn instances(notification: &COVNotificationMultipleRequest) -> Vec<u32> {
    rows(notification).into_iter().map(|row| row.0).collect()
}

#[test]
fn parts_not_sent_after_one_went_are_owed_and_go_first_in_the_next_report() {
    let (mut host, subs) = twenty(false);
    let (report, turn) = host.fan_out(&subs, true, T0).ready();
    assert_eq!(report.parts.len(), 3);
    assert!(report.needs_turn, "several parts keep the turn");
    assert_eq!(report.limit, 204);

    // The first part goes; the second fails, which stops the rest.
    let mut parts = report.parts.into_iter();
    let first = parts.next().unwrap();
    assert_eq!(instances(&first.notification), (1..=8).collect::<Vec<_>>());
    for (sub, observation, completion) in
        carried(&first.finishes, &first.claim, &report.observations, false)
    {
        host.table
            .complete_observation(&sub, completion, observation, T0);
    }
    host.histories.commit(first.claim);
    for part in parts {
        host.histories.give_back(part.claim.owing(), at(1));
    }
    assert!(!host.histories.end_send(&ctx(1, false)));
    assert!(turn.is_some());
    // The owed references are the backstop's, from when they were owed.
    let deadline = at(1) + seconds(u64::from(DELAY));
    assert_eq!(host.histories.next_deadline(), Some(deadline));

    // A delivered reference changes; the next report still leads with the
    // owed ones, so a split cannot keep deferring them.
    set(&mut host.db, 1, 9.0);
    let all = host.all(&ctx(1, false));
    let (report, _) = host.fan_out(&all, false, at(2)).ready();
    assert_eq!(report.parts.len(), 2);
    assert_eq!(
        instances(&report.parts[0].notification),
        (9..=16).collect::<Vec<_>>()
    );
    assert_eq!(
        instances(&report.parts[1].notification),
        [17, 18, 19, 20, 1]
    );
    // The report took the marks; handed back undelivered, with no part
    // gone, its references are owed from when they first were.
    assert_eq!(host.histories.next_deadline(), None);
    for part in report.parts {
        host.histories.give_back(part.claim, at(3));
    }
    assert_eq!(host.histories.next_deadline(), Some(deadline));
}

#[test]
fn a_confirmed_report_sends_its_first_part_and_owes_the_deferred_rest() {
    let (mut host, subs) = twenty(true);
    let (report, _) = host.fan_out(&subs, true, T0).ready();
    assert_eq!(report.parts.len(), 3);
    let whole = report.parts[0].notification.clone();
    let tickets: Vec<_> = report.observations[..8]
        .iter()
        .map(|(_, _, completion)| completion.ticket())
        .collect();

    let first = report.into_first().unwrap();
    assert_eq!(first.notification, whole, "only the oldest part goes now");
    assert_eq!(first.deferred.len(), 2);
    // The Ack completes the eight references the part carries, under the
    // newest of their tickets.
    let carried_instances: Vec<_> = first
        .observations
        .iter()
        .map(|(sub, _)| sub.monitored_object_identifier.instance_number())
        .collect();
    assert_eq!(carried_instances, (1..=8).collect::<Vec<_>>());
    assert_eq!(Some(first.completion.ticket()), tickets.into_iter().max());

    let begun = begin_confirmed(
        &mut host.table,
        &mut host.histories,
        first.completion,
        &first.observations,
        Some(first.claim),
        first.deferred,
        at(1),
    )
    .unwrap();
    // The deferred references are owed from the instant the report took
    // the context, and its claim is lodged until the report's deadline.
    let owed_until = at(1) + seconds(u64::from(DELAY));
    assert_eq!(host.histories.next_deadline(), Some(owed_until));
    assert!(host.table.report_deadline(begun.token) > Some(owed_until));

    // The Ack's follow-up reads the deferred values afresh and sends the
    // next part.
    host.histories
        .settle(begun.claim.unwrap(), Settlement::Delivered, at(2));
    host.table
        .delivered(begun.token, Delivery::Acknowledged, at(2));
    let all = host.all(&ctx(1, true));
    let (report, _) = host.fan_out(&all, false, at(2)).ready();
    assert_eq!(report.parts.len(), 2);
    assert_eq!(
        instances(&report.parts[0].notification),
        (9..=16).collect::<Vec<_>>()
    );
}

#[test]
fn a_part_completes_what_it_finishes_and_an_ack_all_it_carries() {
    // One timestamped change fits a notification to this subscriber, two
    // do not, so three changes go out in three parts.
    let mut host = Host::new(2);
    let subs = host.subscribe(1, false, 2, &[reference(PV, true)], Some(80));
    assert_eq!(host.initial(&subs).len(), 2);
    host.change(1, 1.0, 10, at(1));
    host.change(1, 2.0, 11, at(2));
    host.change(2, 3.0, 12, at(3));
    let all = host.all(&ctx(1, false));
    let (report, _) = host.fan_out(&all, false, at(3)).ready();
    let sent: Vec<_> = report
        .parts
        .iter()
        .map(|part| rows(&part.notification))
        .collect();
    assert_eq!(
        sent,
        [
            [(1, 1.0, Some(10))],
            [(1, 2.0, Some(11))],
            [(2, 3.0, Some(12))]
        ]
    );

    let completes = |part: &Part, every| -> Vec<u32> {
        carried(&part.finishes, &part.claim, &report.observations, every)
            .iter()
            .map(|(sub, _, _)| sub.monitored_object_identifier.instance_number())
            .collect()
    };
    // The first part carries an older change of Analog Value 1: sent
    // unconfirmed it completes nothing, since the reference's latest
    // change is still to come; acknowledged, it completes the reference at
    // the change it carries.
    assert_eq!(completes(&report.parts[0], false), [0u32; 0]);
    assert_eq!(completes(&report.parts[0], true), [1]);
    let (_, observation, _) = &carried(
        &report.parts[0].finishes,
        &report.parts[0].claim,
        &report.observations,
        true,
    )[0];
    let (_, older) = report.parts[0].claim.last_changes().next().unwrap();
    assert_eq!(observation, older.observation());
    // The parts with each reference's latest change finish it either way.
    assert_eq!(completes(&report.parts[1], false), [1]);
    assert_eq!(completes(&report.parts[2], false), [2]);
    assert_eq!(completes(&report.parts[2], true), [2]);
}
