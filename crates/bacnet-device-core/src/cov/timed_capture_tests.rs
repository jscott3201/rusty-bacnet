//! The capture on hand-set instants: what it queues, with which Device time
//! and from which instant its context's Max_Notification_Delay runs.
use bacnet_time::Duration;
use bacnet_types::primitives::PropertyValue;

use super::*;
use crate::cov::multiple_fixture::{
    FLAGS, HandClock, PV, T0, at, av, database, deliver, frame, key, reference, request,
    seconds_and_values, set, subscribe,
};
use crate::write::commit_attempt;

const DELAY: u32 = 30;

/// A table with process 1's timestamped Present_Value references on Analog
/// Values 1 to `count`, admitted at [`T0`] and their initial reports
/// delivered, so nothing is pending.
fn admitted(
    db: &ObjectDatabase,
    count: u32,
) -> (CovTable, TimedHistories, Vec<CovSubscriptionSnapshot>) {
    let mut table = CovTable::default();
    let mut histories = TimedHistories::new(1476);
    let timed = [reference(PV, true)];
    let objects: Vec<_> = (1..=count).map(|instance| (instance, &timed[..])).collect();
    let accepted = subscribe(
        &mut table,
        &mut histories,
        db,
        request(1, Some(300), Some(DELAY), &objects),
        T0,
    )
    .unwrap();
    for snapshot in &accepted {
        deliver(&mut histories, snapshot);
    }
    assert_eq!(histories.next_deadline(), None, "nothing pending");
    (table, histories, accepted)
}

#[test]
fn a_capture_stamps_the_device_time_and_counts_the_delay_from_its_one_instant() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 2);
    let (table, mut histories, accepted) = admitted(&db, 2);

    set(&mut db, 1, 10.0);
    set(&mut db, 2, 20.0);
    clock.set(Some(frame(5)));
    let queued = TimedCapture::all(&table, at(10)).run(&db, &mut histories, at(10));
    assert_eq!(queued, [key(1, 1, PV), key(1, 2, PV)]);

    // The context's Max_Notification_Delay runs from the capture's instant,
    // not from the admission's or any later one.
    assert_eq!(
        histories.next_deadline(),
        Some(at(10) + Duration::from_secs(u64::from(DELAY)))
    );
    // What a subscriber sees is the Device time of the capture, on every
    // value, and the object's Status_Flags beside the value.
    for (snapshot, value) in accepted.iter().zip([10.0, 20.0]) {
        let changes = deliver(&mut histories, snapshot);
        assert_eq!(seconds_and_values(&changes), [(5, value)]);
        let properties: Vec<_> = changes[0]
            .values()
            .iter()
            .map(|value| (value.property_identifier, value.time_of_change))
            .collect();
        let time = Some(frame(5).local_time);
        assert_eq!(properties, [(PV, time), (FLAGS, time)]);
    }
}

#[test]
fn a_capture_queues_what_changed_and_nothing_else() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 2);
    let (table, mut histories, accepted) = admitted(&db, 2);

    // Nothing changed: nothing is queued, however often it is captured.
    let none = TimedCapture::all(&table, at(1)).run(&db, &mut histories, at(1));
    assert!(none.is_empty());
    assert_eq!(histories.next_deadline(), None);

    // Only the object that changed queues a change.
    set(&mut db, 2, 7.0);
    let queued = TimedCapture::all(&table, at(2)).run(&db, &mut histories, at(2));
    assert_eq!(queued, [key(1, 2, PV)]);
    assert!(deliver(&mut histories, &accepted[0]).is_empty());
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[1])),
        [(1, 7.0)]
    );

    // A capture of one object looks at that object alone.
    set(&mut db, 1, 3.0);
    set(&mut db, 2, 8.0);
    let queued = TimedCapture::object(&table, av(1), at(3)).run(&db, &mut histories, at(3));
    assert_eq!(queued, [key(1, 1, PV)]);
    assert!(deliver(&mut histories, &accepted[1]).is_empty());
}

#[test]
fn each_change_of_a_reference_is_kept_in_capture_order() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (table, mut histories, accepted) = admitted(&db, 1);

    // Out, on and back: three changes, the last equal to the first.
    for (second, value) in [(2, 10.0), (3, 20.0), (4, 10.0)] {
        set(&mut db, 1, value);
        clock.set(Some(frame(second)));
        let now = at(u64::from(second));
        TimedCapture::object(&table, av(1), now).run(&db, &mut histories, now);
    }
    // The delay still runs from the earliest of them.
    assert_eq!(
        histories.next_deadline(),
        Some(at(2) + Duration::from_secs(u64::from(DELAY)))
    );
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[0])),
        [(2, 10.0), (3, 20.0), (4, 10.0)]
    );
}

#[test]
fn the_read_fixes_the_values_and_the_device_time_and_the_queueing_the_instant() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (table, mut histories, accepted) = admitted(&db, 1);

    set(&mut db, 1, 4.0);
    clock.set(Some(frame(6)));
    let read = TimedCapture::object(&table, av(1), at(1))
        .read(&db)
        .expect("a reference and a valid clock");
    // What happens between the two steps is not in this capture.
    set(&mut db, 1, 99.0);
    clock.set(Some(frame(50)));
    assert_eq!(read.queue(&mut histories, at(7)), [key(1, 1, PV)]);

    assert_eq!(
        histories.next_deadline(),
        Some(at(7) + Duration::from_secs(u64::from(DELAY)))
    );
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[0])),
        [(6, 4.0)]
    );
}

#[test]
fn without_a_valid_device_clock_nothing_is_read() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (table, mut histories, _) = admitted(&db, 1);
    set(&mut db, 1, 4.0);

    clock.set(None);
    assert!(TimedCapture::all(&table, at(1)).read(&db).is_none());
    // A date whose weekday is wrong is no actual date.
    let mut wrong = frame(2);
    wrong.local_date.day_of_week = 5;
    clock.set(Some(wrong));
    assert!(TimedCapture::all(&table, at(1)).read(&db).is_none());
    assert!(
        TimedCapture::all(&table, at(1))
            .run(&db, &mut histories, at(1))
            .is_empty()
    );
    assert_eq!(histories.next_deadline(), None);

    // The clock back, the change is captured then, at that Device time.
    clock.set(Some(frame(9)));
    assert_eq!(
        TimedCapture::all(&table, at(2)).run(&db, &mut histories, at(2)),
        [key(1, 1, PV)]
    );
}

#[test]
fn only_live_timestamped_references_are_captured() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 2);
    let mut table = CovTable::default();
    let mut histories = TimedHistories::new(1476);
    // Analog Value 1 with timestamps, 2 without, for 300 seconds.
    let accepted = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(
            1,
            Some(300),
            Some(DELAY),
            &[(1, &[reference(PV, true)]), (2, &[reference(PV, false)])],
        ),
        T0,
    )
    .unwrap();
    deliver(&mut histories, &accepted[0]);
    set(&mut db, 1, 1.0);
    set(&mut db, 2, 2.0);

    // The untimestamped reference has no history to capture into, and a
    // capture of its object alone has nothing to read.
    let queued = TimedCapture::all(&table, at(1)).run(&db, &mut histories, at(1));
    assert_eq!(queued, [key(1, 1, PV)]);
    assert!(
        TimedCapture::object(&table, av(2), at(1))
            .read(&db)
            .is_none()
    );

    // Once the lifetime is over, the reference is not captured any more.
    set(&mut db, 1, 5.0);
    assert!(TimedCapture::all(&table, at(300)).read(&db).is_none());
    assert!(
        TimedCapture::object(&table, av(1), at(300))
            .read(&db)
            .is_none()
    );
}

#[test]
fn a_write_request_captures_each_attempt_as_it_commits() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 2);
    let (table, mut histories, accepted) = admitted(&db, 1);
    clock.set(Some(frame(8)));

    let mut observer = TimedWriteCapture::new(
        TimedCapture::all(&table, at(4)),
        (&mut histories, at(4)),
        None,
    );
    // Analog Value 1 goes out and back in one request; Analog Value 2 is
    // written too, and nobody subscribes to it.
    for (instance, value) in [(1, 10.0), (2, 5.0), (1, 20.0), (1, 10.0)] {
        let target = WriteTarget {
            oid: av(instance),
            property: PV,
            array_index: None,
            priority: None,
            value: &[],
        };
        commit_attempt(
            &mut db,
            Some(&mut observer),
            target,
            PropertyValue::Real(value),
            None,
            None,
        )
        .unwrap();
    }
    assert!(observer.life_safety_queued().is_empty());
    drop(observer);

    assert_eq!(
        histories.next_deadline(),
        Some(at(4) + Duration::from_secs(u64::from(DELAY))),
        "every attempt at the request's instant"
    );
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[0])),
        [(8, 10.0), (8, 20.0), (8, 10.0)]
    );
}

#[test]
fn an_initial_capture_reports_the_current_value_at_the_frame_it_is_given() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (table, mut histories, accepted) = admitted(&db, 1);
    set(&mut db, 1, 4.0);
    TimedCapture::all(&table, at(1)).run(&db, &mut histories, at(1));
    deliver(&mut histories, &accepted[0]);

    // Nothing changed since, and the Device clock is gone: an initial
    // capture still reports, with the frame its caller validated.
    clock.set(None);
    let queued =
        TimedCapture::initial(&table, &accepted, frame(40), at(2)).run(&db, &mut histories, at(2));
    assert_eq!(queued, [key(1, 1, PV)]);
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[0])),
        [(40, 4.0)]
    );
}
