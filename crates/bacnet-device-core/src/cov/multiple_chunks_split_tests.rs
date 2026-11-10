//! The split of a report to the subscriber's APDU size, on plain claims:
//! where the boundaries fall, that every part fits, the order of the items,
//! and what the split hands back for the histories.
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::*;
use crate::cov::multiple_fixture::{
    FLAGS, HandClock, PV, T0, at, database, frame, key, reference, request, set, subscribe,
};
use crate::cov::multiple_items::FieldStamp;
use crate::cov::timed::TimedHistories;
use crate::cov::timed_capture::TimedCapture;
use crate::cov::{CovSubscriptionSnapshot, CovTable};

/// Octets no report here comes near.
const ROOMY: usize = 1474;

/// A context's references with what a report of them needs: the changes
/// captured for each, and the values read for the untimestamped ones.
struct Report {
    accepted: Vec<CovSubscriptionSnapshot>,
    /// Each reference's drained changes with their history's incarnation.
    drained: Vec<(u64, Vec<TimedChange>)>,
    reads: MultipleReads,
    /// Each untimestamped reference's prepared values; empty for the rest.
    values: Vec<Vec<COVNotificationValue>>,
}

/// Process 1 subscribes Present_Value of Analog Values 1 to `objects`, with
/// timestamps or without. Then `writes` changes are captured, each at its
/// own Device second from 1 on, going round the objects.
fn report(objects: u32, timestamped: bool, writes: u8) -> Report {
    let clock = HandClock::showing(0);
    let mut db: ObjectDatabase = database(Some(&clock), objects);
    let mut table = CovTable::default();
    let mut histories = TimedHistories::new(1476);
    let one = [reference(PV, timestamped)];
    let all: Vec<_> = (1..=objects).map(|instance| (instance, &one[..])).collect();
    let accepted = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &all),
        T0,
    )
    .unwrap();
    // The initial reports are not part of what these tests split.
    for snapshot in &accepted {
        histories.drain(snapshot.key(), snapshot.generation());
    }
    for second in 1..=writes {
        let instance = u32::from(second - 1) % objects + 1;
        set(&mut db, instance, f32::from(second));
        clock.set(Some(frame(second)));
        let now = at(u64::from(second));
        TimedCapture::all(&table, now).run(&db, &mut histories, now);
    }
    let drained = accepted
        .iter()
        .map(|snapshot| histories.drain(snapshot.key(), snapshot.generation()))
        .collect();
    let mut reads = MultipleReads::default();
    let values = accepted
        .iter()
        .map(|snapshot| {
            let object = db.get(&snapshot.monitored_object_identifier).unwrap();
            match snapshot.timestamped {
                true => Vec::new(),
                false => reads.prepare(object, snapshot, None, true).unwrap().values,
            }
        })
        .collect();
    Report {
        accepted,
        drained,
        reads,
        values,
    }
}

impl Report {
    /// The claim a fanout would hold for this report.
    fn claim(&self) -> TimedClaim {
        let mut claim = TimedClaim::new();
        for (snapshot, (incarnation, changes)) in self.accepted.iter().zip(&self.drained) {
            if snapshot.timestamped {
                claim.add(snapshot.key().clone(), *incarnation, changes.clone());
            } else {
                claim.add_untimed(snapshot.key().clone(), snapshot.generation(), None);
            }
        }
        claim
    }

    /// Split the report's claim, or `claim`, at `limit`, retaining every
    /// reference when `retain`.
    fn split_with(&self, claim: TimedClaim, retain: bool, limit: usize) -> Split {
        let envelope = Envelope {
            subscriber_process_identifier: 1,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap(),
            time_remaining: 300,
        };
        let retained: Vec<_> = self
            .accepted
            .iter()
            .zip(&self.values)
            .filter(|_| retain)
            .map(|(snapshot, values)| (snapshot, values.as_slice()))
            .collect();
        let untimed = BTreeSet::new();
        let stamp = |_: &Coordinate, _: &[u8]| FieldStamp::Unowned;
        split(
            &ReportContent {
                envelope: &envelope,
                retained: &retained,
                reads: &self.reads,
                untimed: &untimed,
                stamp: &stamp,
            },
            claim,
            limit,
        )
    }

    fn split(&self, limit: usize) -> Split {
        self.split_with(self.claim(), true, limit)
    }
}

fn len(part: &Part) -> usize {
    encoded_len(&part.notification).expect("it encodes")
}

/// Each value a part conveys, in order: its object's instance, its
/// property, and the Device second of its Time_Of_Change if it has one.
fn rows(part: &Part) -> Vec<(u32, PropertyIdentifier, Option<u8>)> {
    part.notification
        .list_of_cov_notifications
        .iter()
        .flat_map(|item| {
            item.list_of_values.iter().map(|value| {
                (
                    item.monitored_object_identifier.instance_number(),
                    value.property_identifier,
                    value.time_of_change.map(|time| time.second),
                )
            })
        })
        .collect()
}

/// The Device seconds of the Present_Value rows of every part, in the order
/// the parts go out.
fn seconds(parts: &[Part]) -> Vec<u8> {
    parts
        .iter()
        .flat_map(rows)
        .filter(|(_, property, _)| *property == PV)
        .filter_map(|(_, _, second)| second)
        .collect()
}

fn finished(part: &Part) -> Vec<CovSubscriptionKey> {
    part.finishes.iter().cloned().collect()
}

fn nothing_given_up(split: &Split) {
    assert!(split.lost.is_empty());
    assert!(split.forgone.is_empty());
    assert!(split.rest.is_empty());
}

#[test]
fn a_report_that_fits_is_one_notification_history_first_then_each_reference_once() {
    // Analog Value 1 at seconds 1 and 3, Analog Value 2 at second 2.
    let report = report(2, true, 3);
    let split = report.split(ROOMY);
    nothing_given_up(&split);
    let [part] = &split.parts[..] else {
        panic!("one part, not {}", split.parts.len());
    };
    assert_eq!(
        rows(part),
        [
            // The older change of Analog Value 1, as history.
            (1, PV, Some(1)),
            (1, FLAGS, Some(1)),
            // Then the current state of each reference, in the order the
            // report retains them.
            (1, PV, Some(3)),
            (1, FLAGS, Some(3)),
            (2, PV, Some(2)),
            (2, FLAGS, Some(2)),
        ]
    );
    // The notification is named after the newest change it carries.
    assert_eq!(
        part.notification.timestamp,
        Some(cov_multiple_datetime(frame(3)))
    );
    assert_eq!(finished(part), [key(1, 1, PV), key(1, 2, PV)]);
}

#[test]
fn every_part_fits_and_none_could_carry_one_change_more() {
    const CHANGES: u8 = 12;
    let whole = |changes: u8| len(&report(1, true, changes).split(ROOMY).parts[0]);
    // One change alone, and what each further change adds to its item.
    let (single, each) = (whole(1), whole(2) - whole(1));
    let report = report(1, true, CHANGES);
    assert_eq!(whole(CHANGES), single + usize::from(CHANGES - 1) * each);

    for limit in single..whole(CHANGES) {
        let split = report.split(limit);
        nothing_given_up(&split);
        let parts = &split.parts;
        assert!(parts.len() > 1, "limit {limit}: more than one notification");
        // No part is past the subscriber's size.
        for part in parts {
            assert!(len(part) <= limit, "limit {limit}: a part of {}", len(part));
        }
        // The last part takes as many of the newest changes as fit, and the
        // older ones fill the parts before it from the oldest on: one change
        // more would fit none of them. Only the part before the last, which
        // takes what is left, may have room.
        let remainder = parts.len() - 2;
        for (at, part) in parts.iter().enumerate() {
            assert!(
                at == remainder || len(part) + each > limit,
                "limit {limit}: part {at} of {} had room for a change",
                len(part)
            );
        }
        // Every change goes out once, oldest first, and only the last part,
        // which carries the newest, finishes the reference.
        assert_eq!(
            seconds(parts),
            (1..=CHANGES).collect::<Vec<_>>(),
            "limit {limit}"
        );
        let (last, earlier) = parts.split_last().unwrap();
        assert!(earlier.iter().all(|part| part.finishes.is_empty()));
        assert_eq!(finished(last), [key(1, 1, PV)]);
    }
}

#[test]
fn a_change_too_large_for_one_notification_goes_out_value_by_value() {
    // What a notification of one whole change takes.
    let single = len(&report(1, true, 1).split(ROOMY).parts[0]);
    let report = report(1, true, 2);
    // One octet short of a whole change: each value still fits alone.
    let split = report.split(single - 1);
    nothing_given_up(&split);
    let conveyed: Vec<_> = split.parts.iter().map(rows).collect();
    assert_eq!(
        conveyed,
        [
            [(1, PV, Some(1))],
            [(1, FLAGS, Some(1))],
            [(1, PV, Some(2))],
            [(1, FLAGS, Some(2))],
        ]
    );
    for part in &split.parts {
        assert!(len(part) < single);
    }
    // Only the part with the last value of the newest change finishes it.
    let finishing: Vec<_> = split.parts.iter().map(|part| part.finishes.len()).collect();
    assert_eq!(finishing, [0, 0, 0, 1]);
}

#[test]
fn a_value_too_large_for_any_notification_is_given_up_and_its_change_named() {
    let report = report(2, true, 2);
    // Fewer octets than the envelope alone takes.
    let split = report.split(10);
    assert!(split.parts.is_empty());
    // Once for each change, in capture order, however many values it lost.
    assert_eq!(split.lost, [key(1, 1, PV), key(1, 2, PV)]);
    assert!(split.forgone.is_empty());
    assert!(split.rest.is_empty(), "the changes are gone, not returned");
}

#[test]
fn untimestamped_values_split_by_object_and_those_that_fit_nowhere_are_handed_back() {
    let report = report(2, false, 2);
    let whole = report.split(ROOMY);
    nothing_given_up(&whole);
    let [both] = &whole.parts[..] else {
        panic!("one part, not {}", whole.parts.len());
    };
    assert_eq!(
        rows(both),
        [
            (1, PV, None),
            (1, FLAGS, None),
            (2, PV, None),
            (2, FLAGS, None)
        ]
    );
    assert_eq!(finished(both), [key(1, 1, PV), key(1, 2, PV)]);

    // One octet short of both: an item each, and each part finishes the
    // reference whose values it carries.
    let apart = report.split(len(both) - 1);
    nothing_given_up(&apart);
    let conveyed: Vec<_> = apart.parts.iter().map(rows).collect();
    assert_eq!(
        conveyed,
        [
            [(1, PV, None), (1, FLAGS, None)],
            [(2, PV, None), (2, FLAGS, None)]
        ]
    );
    for (part, instance) in apart.parts.iter().zip([1, 2]) {
        assert!(len(part) < len(both));
        assert_eq!(finished(part), [key(1, instance, PV)]);
        let carried: Vec<_> = part.claim.untimed().cloned().collect();
        assert_eq!(carried, [key(1, instance, PV)]);
    }

    // Room for one object's item and not the other's: the first goes out,
    // and the one left out is handed back, for the host to count.
    let one = len(&apart.parts[0]);
    let short = report.split(one);
    assert_eq!(short.parts.len(), 2, "both items are the same size");
    let none = report.split(one - 1);
    assert!(none.parts.is_empty());
    let forgone: Vec<_> = none.forgone.untimed().cloned().collect();
    assert_eq!(forgone, [key(1, 1, PV), key(1, 2, PV)]);
    assert!(none.lost.is_empty() && none.rest.is_empty());
}

#[test]
fn changes_a_report_cannot_convey_are_left_over_for_their_references() {
    let report = report(2, true, 2);
    // The report retains no reference, so nothing conveys each one's latest
    // change and its one notification would carry nothing: it is not sent,
    // and the changes are not lost.
    let split = report.split_with(report.claim(), false, ROOMY);
    assert!(split.parts.is_empty());
    assert!(split.lost.is_empty() && split.forgone.is_empty());
    let left: Vec<_> = split
        .rest
        .in_order()
        .iter()
        .map(|(key, change)| ((*key).clone(), change.frame().local_time.second))
        .collect();
    assert_eq!(left, [(key(1, 1, PV), 1), (key(1, 2, PV), 2)]);
}
