//! The report steps and what they tell the histories: the bound a
//! deferred part comes back past, a value too large for any notification,
//! untimestamped values left out, and a lifetime that has ended.
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::primitives::ObjectIdentifier;

use super::tests::{DELAY, Host, Outcome, ctx, rows, seconds};
use super::*;
use crate::cov::multiple_fixture::{PV, T0, at, frame, peer, reference, request, set};
use crate::cov::timed::Settlement;
use crate::cov::timed_capture::TimedCapture;
use crate::cov::{Delivery, subscribe_cov_property_multiple};

#[test]
fn deferred_parts_come_back_whole_even_past_the_history_bound() {
    // One change per notification to this subscriber.
    let mut host = Host::new(1);
    let subs = host.subscribe(1, true, 1, &[reference(PV, true)], Some(80));
    let acknowledged = |host: &mut Host, subs: &[CovSubscriptionSnapshot], now| {
        let (report, _) = host.fan_out(subs, false, now).ready();
        let first = report.into_first().unwrap();
        let begun = begin_confirmed(
            &mut host.table,
            &mut host.histories,
            first.completion,
            &first.observations,
            Some(first.claim),
            first.deferred,
            now,
        )
        .unwrap();
        host.histories
            .settle(begun.claim.unwrap(), Settlement::Delivered, now);
        host.table
            .delivered(begun.token, Delivery::Acknowledged, now);
    };
    acknowledged(&mut host, &subs, T0);

    for change in 1..=4u8 {
        host.change(1, f32::from(change), 10 + change, at(u64::from(change)));
    }
    let all = host.all(&ctx(1, true));
    let (report, _) = host.fan_out(&all, false, at(4)).ready();
    assert_eq!(report.parts.len(), 4);
    let first = report.into_first().unwrap();
    assert_eq!(first.deferred.len(), 3);
    // More changes arrive while the report's are drained.
    for change in 5..=8u8 {
        host.change(1, f32::from(change), 10 + change, at(u64::from(change)));
    }
    assert_eq!(host.histories.take_counts().changes_dropped, 0);

    // The report takes the context: its three deferred changes queue
    // again beside the four newer ones, and the bound drops none of what
    // the report planned to send.
    let begun = begin_confirmed(
        &mut host.table,
        &mut host.histories,
        first.completion,
        &first.observations,
        Some(first.claim),
        first.deferred,
        at(8),
    )
    .unwrap();
    assert_eq!(host.histories.take_counts().changes_dropped, 0);
    // That is more than the context may hold: the next capture, which
    // does apply the bound, evicts.
    host.histories
        .settle(begun.claim.unwrap(), Settlement::Delivered, at(9));
    host.change(1, 9.0, 19, at(9));
    assert!(host.histories.take_counts().changes_dropped > 0);
}

/// A CharacterString Value beside the Analog Values, and one context of
/// process 1 on its Present_Value, for a subscriber of 206 octets.
fn text_host(timestamped: bool) -> (Host, Vec<CovSubscriptionSnapshot>) {
    use bacnet_objects::value_types::CharacterStringValueObject;
    use bacnet_services::cov_multiple::COVSubscriptionSpecification;
    let mut host = Host::new(1);
    host.db
        .add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
    let mut request = request(1, Some(300), Some(DELAY), &[]);
    request
        .list_of_cov_subscription_specifications
        .push(COVSubscriptionSpecification {
            monitored_object_identifier: csv(),
            list_of_cov_references: vec![reference(PV, timestamped)],
        });
    let subs = subscribe_cov_property_multiple(
        &mut host.table,
        &mut host.histories,
        &host.db,
        &peer(),
        Some(206),
        request,
        T0,
    )
    .unwrap();
    host.initial(&subs);
    (host, subs)
}

fn csv() -> ObjectIdentifier {
    ObjectIdentifier::new(bacnet_types::enums::ObjectType::CHARACTERSTRING_VALUE, 1).unwrap()
}

/// Write `characters` characters to the CharacterString Value.
fn set_text(db: &mut ObjectDatabase, characters: usize) {
    use bacnet_types::primitives::PropertyValue;
    let target = crate::write::WriteTarget {
        oid: csv(),
        property: PV,
        array_index: None,
        priority: Some(8),
        value: &[],
    };
    let text = "x".repeat(characters);
    let value = PropertyValue::CharacterString(text.as_str().into());
    crate::write::commit_attempt(db, None, target, value, None, None).unwrap();
}

#[test]
fn the_split_tells_the_histories_of_a_value_too_large_for_any_notification() {
    use crate::cov::timed::DropReason;
    let (mut host, subs) = text_host(true);
    assert_eq!(host.histories.take_counts().changes_dropped, 0);
    // Too large for a 206-octet notification even alone.
    set_text(&mut host.db, 300);
    host.clock.set(Some(frame(10)));
    TimedCapture::object(&host.table, csv(), at(1)).run(&host.db, &mut host.histories, at(1));
    let (report, _) = host.fan_out(&subs, false, at(1)).ready();
    // The change's other value, its Status_Flags, still goes out.
    assert_eq!(report.parts.len(), 1);
    assert_eq!(host.histories.take_counts().changes_dropped, 1);
    let drops = host.histories.take_drops();
    assert_eq!(drops.len(), 1);
    assert_eq!(
        (drops[0].reason, drops[0].first),
        (DropReason::TooLarge, true)
    );
    assert_eq!(&drops[0].key, subs[0].key());
}

#[test]
fn untimestamped_values_too_large_alone_are_left_out_counted_and_not_owed() {
    let (mut host, subs) = text_host(false);
    set_text(&mut host.db, 300);
    let (report, turn) = host.fan_out(&subs, false, at(1)).ready();
    assert!(report.parts.is_empty());
    assert_eq!(report.left_out, [subs[0].key().clone()]);
    assert_eq!(report.limit, 204);
    assert_eq!(host.histories.take_counts().untimed_oversized, 1);
    assert!(turn.is_some() && !report.needs_turn);
    assert!(!host.histories.end_send(&ctx(1, false)));
    assert_eq!(host.histories.next_deadline(), None, "not owed");
}

#[test]
fn a_reference_past_its_lifetime_reports_nothing() {
    let mut host = Host::new(1);
    let subs = host.subscribe(1, false, 1, &[reference(PV, false)], None);
    host.initial(&subs);
    set(&mut host.db, 1, 4.0);
    // Snapshots taken while the subscription lived, fanned out as its
    // 300 s end: a change it would report, and no lifetime to report in.
    let ended = T0 + seconds(300);
    assert!(matches!(
        host.fan_out(&subs, false, ended),
        Outcome::Nothing(0)
    ));
    // One second earlier it reports.
    let (report, _) = host.fan_out(&subs, false, ended - seconds(1)).ready();
    assert_eq!(rows(&report.parts[0].notification), [(1, 4.0, None)]);
    assert_eq!(report.parts[0].notification.time_remaining, 1);
}

/// `contexts` unconfirmed contexts, processes 1 to `contexts`, each with an
/// untimestamped Present_Value reference on Analog Values 1 and 2.
fn sharing(contexts: u32) -> Host {
    let mut host = Host::new(2);
    for process in (1..=contexts).rev() {
        host.subscribe(process, false, 2, &[reference(PV, false)], None);
    }
    host
}

/// The processes of one fanout over Analog Value `object`'s references,
/// in the order they take their turns.
fn turns(host: &mut Host, object: u32) -> Vec<u32> {
    let grouped = by_context(host.on(object, T0));
    in_turn(&mut host.histories, grouped)
        .iter()
        .map(|subs| subs[0].subscriber_process_identifier)
        .collect()
}

#[test]
fn turns_are_key_order_from_a_start_that_moves_per_fanout() {
    let mut host = sharing(3);
    let firsts: Vec<_> = (0..12)
        .map(|_| {
            let order = turns(&mut host, 1);
            // Always a rotation of the key order.
            let from = order[0];
            let rotation: Vec<_> = (0..3).map(|at| (from - 1 + at) % 3 + 1).collect();
            assert_eq!(order, rotation);
            from
        })
        .collect();
    assert_eq!(firsts, [1, 3, 2, 1, 3, 3, 3, 3, 2, 3, 1, 3]);

    // A fanout with one context has no order to choose and leaves the
    // sequence where it was: a second host that saw none of them takes
    // the same next turn.
    let mut other = sharing(3);
    for _ in 0..12 {
        turns(&mut other, 1);
    }
    let mut single = sharing(1);
    for _ in 0..5 {
        assert_eq!(turns(&mut single, 1), [1]);
        let lone = by_context(host.on(1, T0).into_iter().take(1));
        assert_eq!(in_turn(&mut host.histories, lone).len(), 1);
    }
    assert_eq!(turns(&mut host, 1), turns(&mut other, 1));
}

/// Run `changes` fanouts, Analog Values 1 and 2 changing in alternation
/// (or only the first, when not `alternating`), each serving the first
/// `budget` contexts in turn: how often each process heard of each object.
fn heard(contexts: u32, budget: usize, changes: u32, alternating: bool) -> Vec<((u32, u32), u32)> {
    let mut host = sharing(contexts);
    let mut heard = alloc::collections::BTreeMap::new();
    for change in 0..changes {
        let object = if alternating { change % 2 + 1 } else { 1 };
        for process in turns(&mut host, object).into_iter().take(budget) {
            *heard.entry((process, object)).or_insert(0) += 1;
        }
    }
    heard.into_iter().collect()
}

#[test]
fn a_spent_budget_spreads_its_losses_over_contexts_and_objects() {
    // Three contexts, two served a change, two objects changing in turn:
    // every context hears of both, about as often.
    let three = heard(3, 2, 60, true);
    let expected = [
        ((1, 1), 23),
        ((1, 2), 19),
        ((2, 1), 17),
        ((2, 2), 19),
        ((3, 1), 20),
        ((3, 2), 22),
    ];
    assert_eq!(three, expected);
    // Two contexts, one served a change, two objects in turn: a start that
    // moved by one each fanout would give each context one object only.
    let two = heard(2, 1, 40, true);
    assert_eq!(two, [((1, 1), 9), ((1, 2), 9), ((2, 1), 11), ((2, 2), 11)]);
    // One object, one served a change: every context in a run.
    let one = heard(3, 1, 60, false);
    assert_eq!(one, [((1, 1), 18), ((2, 1), 18), ((3, 1), 24)]);
    // An ample budget serves everyone every time, whatever the start.
    let all = heard(3, 3, 10, true);
    assert!(all.iter().all(|(_, count)| *count == 5), "{all:?}");
}
