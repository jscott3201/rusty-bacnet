//! The hold-off after a failed confirmed COV-multiple report, and whose
//! failure it follows (#2419). A failure the table counted holds the
//! context's Max_Notification_Delay backstop off until the hold-off ends,
//! no shorter and no longer. A failure the table ignored, because the
//! report's subscription was replaced or cancelled meanwhile, leaves the
//! backstop of the subscription made since where it was.
//!
//! The reporter here retries once after 3 s, so a hold-off lasts 6 s and a
//! report nobody answers fails 6 s after it began.
use super::*;

/// The hold-off of this reporter's timing, in seconds.
const HOLD_OFF: u64 = 6;

/// One confirmed context on AI-1's timestamped Present_Value.
fn timed() -> [(ObjectIdentifier, bool); 1] {
    [(ai(), true)]
}

impl Device {
    /// Pop the next report and tell the reporter it could not be sent: its
    /// changes are pending again, with no hold-off, and wait for their
    /// Max_Notification_Delay.
    fn pop_unsent(&mut self, now: Instant) -> Vec<Row> {
        let (report, rows) = self.pop_multiple(now).expect("a report to leave unsent");
        self.reporter
            .delivered(report.token, Delivery::NotSent, &mut self.ids, now);
        rows
    }

    /// Run the reporter's deadlines in turn until one sends a report: when
    /// that is, and the report with its rows.
    fn next_report(&mut self) -> (Instant, Notification, Vec<Row>) {
        loop {
            let now = self.reporter.next_deadline().expect("something due");
            self.reporter.on_deadline(&self.db, &mut self.ids, now);
            if let Some((report, rows)) = self.pop_multiple(now) {
                return (now, report, rows);
            }
        }
    }
}

#[test]
fn a_replaced_subscriptions_refused_report_ends_nothing() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &timed(), 1476, T0);
    let (old, _) = device.pop_multiple(T0).expect("the initial report");

    // The subscriber lists the reference again while that report is out,
    // and the look the fence owes finds the new report begun. That report
    // can't be sent, so its changes wait for the backstop: 5 s after the
    // earliest of them, the old report's, which the fence gave back.
    clock.set(2);
    device.subscribe_multiple(1, true, &timed(), 1476, secs(1));
    device.deadline();
    assert_eq!(device.pop_unsent(secs(1)).len(), 2);

    // The old report's refusal names a report the fence already ended.
    assert!(!device.answer(old.invoke_id.unwrap(), true, secs(2)));
    assert_eq!(device.reporter.counters().failed, 0);
    let (at, _, sent) = device.next_report();
    assert_eq!(at, secs(DELAY), "the backstop, on time");
    assert_eq!(sent, [(1, 10.0, Some(1)), (1, 10.0, Some(2))]);
}

#[test]
fn a_cancelled_subscriptions_refused_report_leaves_the_new_backstop_alone() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &timed(), 1476, T0);
    let (old, _) = device.pop_multiple(T0).expect("the initial report");

    // The context is cancelled and made again at second 1, with that
    // report still out. The new subscription's report can't be sent, so
    // its change is due 5 s after its capture.
    let reply = device.subscribe_multiple(1, true, &[], 1476, secs(1));
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    clock.set(2);
    device.subscribe_multiple(1, true, &timed(), 1476, secs(1));
    assert_eq!(device.pop_unsent(secs(1)), [(1, 10.0, Some(2))]);

    // The cancelled subscription's report is refused at second 2. The
    // table knows no such report any more, so nothing holds off.
    assert!(device.answer(old.invoke_id.unwrap(), true, secs(2)));
    let (at, _, sent) = device.next_report();
    assert_eq!(
        at,
        secs(1 + DELAY),
        "the new subscription's backstop, not the old report's hold-off"
    );
    assert_eq!(sent, [(1, 10.0, Some(2))]);
}

#[test]
fn a_cancelled_subscriptions_unanswered_report_leaves_the_new_backstop_alone() {
    let (mut device, clock) = device();
    device.subscribe_multiple(1, true, &timed(), 1476, T0);
    device.pop_multiple(T0).expect("the initial report");

    // Cancelled and made again at second 1; the new subscription's initial
    // report is acknowledged.
    device.subscribe_multiple(1, true, &[], 1476, secs(1));
    clock.set(2);
    device.subscribe_multiple(1, true, &timed(), 1476, secs(1));
    let (new, _) = device
        .pop_multiple(secs(1))
        .expect("the new initial report");
    assert!(device.answer(new.invoke_id.unwrap(), false, secs(1)));
    // The old report is sent again at second 3, and nobody answers.
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, secs(3));
    assert!(device.pop(secs(3)).is_some(), "the old report, resent");

    // A change at second 4 whose report can't be sent is due at second 9.
    clock.set(4);
    device.sample(ai(), 11.0, secs(4));
    assert_eq!(device.pop_unsent(secs(4)), [(1, 11.0, Some(4))]);

    // The old report's retries run out at second 6.
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, secs(HOLD_OFF));
    assert_eq!(device.reporter.counters().failed, 1);
    assert!(device.pop(secs(HOLD_OFF)).is_none());
    let (at, _, sent) = device.next_report();
    assert_eq!(
        at,
        secs(4 + DELAY),
        "the new subscription's backstop, not the old report's hold-off"
    );
    assert_eq!(sent, [(1, 11.0, Some(4))]);
}

/// A real failure, where only the reporter's own step moves the backstop:
/// the backstop sent the report, so its next attempt would wait another
/// Max_Notification_Delay, here longer than the hold-off.
#[test]
fn a_refused_backstop_report_goes_out_again_when_its_hold_off_ends() {
    const SLOW: u64 = 20;
    let (mut device, clock) = device();
    device.subscribe_delayed(1, true, &timed(), (1476, SLOW as u32), T0);
    let (initial, _) = device.pop_multiple(T0).expect("the initial report");
    assert!(device.answer(initial.invoke_id.unwrap(), false, T0));
    device.deadline();

    // A change at second 1 whose report can't be sent waits for the
    // backstop, which sends it at second 21.
    clock.set(10);
    device.sample(ai(), 11.0, secs(1));
    assert_eq!(device.pop_unsent(secs(1)), [(1, 11.0, Some(10))]);
    let (backstop, report, _) = device.next_report();
    assert_eq!(backstop, secs(1 + SLOW));

    // Refused there: the context holds off for 6 s, and the backstop tries
    // again when that ends, not a whole delay later.
    assert!(device.answer(report.invoke_id.unwrap(), true, backstop));
    let until = secs(1 + SLOW + HOLD_OFF);
    let before = secs(SLOW + HOLD_OFF);
    device
        .reporter
        .on_deadline(&device.db, &mut device.ids, before);
    assert!(device.pop(before).is_none(), "still held off");
    let (at, again, sent) = device.next_report();
    assert_eq!(at, until, "when the hold-off ends");
    assert_eq!(sent, [(1, 11.0, Some(10))]);
    assert!(device.answer(again.invoke_id.unwrap(), false, until));
    device.deadline();
    assert!(device.pop(until).is_none(), "delivered");
}
