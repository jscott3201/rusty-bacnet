//! Owed references: which evaluation ends the debt, and how a look hands
//! freed entries on without spinning.

use super::*;
use crate::cov::CovPolicy;

fn send(device: &mut Device, now: Instant) -> Option<u32> {
    let (sent, apdu) = device.pop(now)?;
    device
        .reporter
        .delivered(sent.token, Delivery::Sent, &mut device.ids, now);
    Some(reported(&apdu).0)
}

/// An owed reference whose object is gone is owed nothing more, and the
/// entry its look would have taken goes to the next owed one at once,
/// with no other change or pop.
#[test]
fn a_look_drops_a_stale_owed_reference_and_moves_on() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.subscribe_to(ai(), 1, false, T0);
    device.subscribe_to(ai2(), 2, false, T0);
    device.subscribe_to(ai(), 3, false, T0);
    assert_eq!(device.reporter.owed.len(), 2);
    device.db.remove(&ai2()).unwrap();
    assert_eq!(send(&mut device, T0), Some(1));
    for _ in 0..4 {
        if device.reporter.next_deadline() != Some(T0) {
            break;
        }
        device.deadline();
    }
    assert_eq!(send(&mut device, T0), Some(3), "3's initial notification");
    assert!(device.reporter.owed.is_empty());
    assert!(device.reporter.next_deadline().is_none_or(|due| due > T0));
}

/// A budget of one notification a change: a change that reports 1 drops
/// 2's owed initial report, which stays owed and goes out once an entry
/// frees.
#[test]
fn an_owed_reference_the_budget_drops_stays_owed() {
    let policy = CovPolicy {
        max_notifications_per_event: 1,
        ..CovPolicy::default()
    };
    let room = OutboxLimits {
        entries: 1,
        octets: 256,
    };
    let mut device = Device::with_policy(room, policy);
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    device.subscribe(1, false, T0);
    device.subscribe(2, false, T0);
    assert_eq!(send(&mut device, T0), Some(1));
    // Before a deadline looks at 2, a change reports 1 and spends the budget.
    device.write(P::PRESENT_VALUE, PropertyValue::Real(20.0), T0);
    assert_eq!(device.reporter.owed.len(), 1, "2 is still owed");
    let mut order = Vec::new();
    for _ in 0..3 {
        if device.reporter.next_deadline() == Some(T0) {
            device.deadline();
        }
        order.extend(send(&mut device, T0));
    }
    assert_eq!(order, [1, 2]);
    assert!(device.reporter.owed.is_empty());
}

/// An owed reference that an evaluation finds nothing to report for, its
/// value back at what it last reported, is owed nothing more.
#[test]
fn an_owed_reference_with_nothing_to_report_is_cleared() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    for process in [1, 2] {
        device.subscribe(process, false, T0);
        assert_eq!(send(&mut device, T0), Some(process));
    }
    // 1's report takes the entry; 2's is refused.
    device.write(P::PRESENT_VALUE, PropertyValue::Real(20.0), T0);
    assert_eq!(device.reporter.owed.len(), 1);
    device.write(P::PRESENT_VALUE, PropertyValue::Real(10.0), T0);
    assert!(
        device.reporter.owed.is_empty(),
        "back at its baseline: nothing owed"
    );
}

/// Three entries with octets for one notification: a look that queues an
/// owed reference gives the next its turn at once, and one that settles
/// none waits for a pop rather than coming due again.
#[test]
fn a_look_that_settles_nothing_waits_for_a_pop() {
    let mut device = Device::new(OutboxLimits {
        entries: 3,
        octets: 40,
    });
    for process in 1..=3 {
        device.subscribe(process, false, T0);
    }
    assert_eq!(device.reporter.owed.len(), 2);
    assert_eq!(send(&mut device, T0), Some(1));
    device.deadline();
    assert_eq!(device.reporter.owed.len(), 1, "2 queued, 3 refused again");
    assert_eq!(device.reporter.next_deadline(), Some(T0), "3's turn");
    device.deadline();
    assert_eq!(device.reporter.owed.len(), 1);
    assert!(
        device.reporter.next_deadline().is_none_or(|due| due > T0),
        "a look that settled nothing doesn't come due again"
    );
    assert_eq!(send(&mut device, T0), Some(2));
    device.deadline();
    assert_eq!(send(&mut device, T0), Some(3));
}

/// The octets of an UnconfirmedCOVNotification for one of these
/// subscriptions; a confirmed one is two more.
fn unconfirmed_octets() -> usize {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, false, T0);
    let mut out = BytesMut::new();
    device
        .reporter
        .pop(&NoBindings, &mut device.ids, T0, &mut out)
        .unwrap();
    out.len()
}

/// A budget of octets that fits an unconfirmed notification but not a
/// confirmed one: the confirmed subscription's report can never go, so
/// it is counted and not owed, and the owed one behind it goes once an
/// entry frees.
#[test]
fn a_report_over_the_byte_budget_is_not_owed() {
    let policy = CovPolicy {
        max_notification_bytes_per_event: unconfirmed_octets(),
        ..CovPolicy::default()
    };
    let room = OutboxLimits {
        entries: 1,
        octets: 256,
    };
    let mut device = Device::with_policy(room, policy);
    device.subscribe(3, false, T0);
    device.subscribe(1, true, T0);
    device.subscribe_to(ai2(), 2, false, T0);
    assert_eq!(device.reporter.owed.len(), 1, "2 owed; 1 can never go");
    assert_eq!(device.reporter.counters().owed, 2);
    assert_eq!(send(&mut device, T0), Some(3));
    let mut sent = Vec::new();
    for _ in 0..4 {
        if device.reporter.next_deadline() == Some(T0) {
            device.deadline();
        }
        sent.extend(send(&mut device, T0));
    }
    assert_eq!(sent, [2]);
    assert!(device.reporter.owed.is_empty());
}

/// A budget of octets for one and a half notifications: a look at two
/// owed references reports the first, and admits the second only for the
/// budget to drop it. That one stays owed and goes at the next look.
#[test]
fn an_owed_report_the_byte_budget_drops_after_admission_stays_owed() {
    let octets = unconfirmed_octets();
    let policy = CovPolicy {
        max_notification_bytes_per_event: octets + octets / 2,
        ..CovPolicy::default()
    };
    let room = OutboxLimits {
        entries: 2,
        octets: 256,
    };
    let mut device = Device::with_policy(room, policy);
    for process in 1..=4 {
        device.subscribe(process, false, T0);
    }
    assert_eq!(device.reporter.owed.len(), 2);
    assert_eq!(send(&mut device, T0), Some(1));
    assert_eq!(send(&mut device, T0), Some(2));
    let throttled = device
        .reporter
        .counters()
        .cov
        .notifications_throttled_fanout;
    let mut sent = Vec::new();
    for _ in 0..4 {
        if device.reporter.next_deadline() == Some(T0) {
            device.deadline();
        }
        sent.extend(send(&mut device, T0));
    }
    assert_eq!(sent, [3, 4]);
    let counters = device.reporter.counters();
    assert_eq!(
        counters.cov.notifications_throttled_fanout,
        throttled + 1,
        "4's report was admitted, then dropped by the budget"
    );
    assert!(device.reporter.owed.is_empty());
}

/// Reports owed for an invoke ID are kept apart from those owed room
/// (#2512): while one waits for an ID, nothing comes due for it, and the
/// reporter's own ID coming back, here with an Ack, gives it its turn.
#[test]
fn a_report_owed_an_id_waits_for_one_without_coming_due() {
    let mut device = Device::new(OutboxLimits::default());
    device.ids.limit = 1;
    device.subscribe(1, true, T0);
    let (out, _) = device.pop(T0).unwrap();
    device.subscribe(2, true, T0);
    assert!(device.pop(T0).is_none(), "no ID for 2's report");
    let answer = T0 + Duration::from_secs(3);
    assert_eq!(device.reporter.next_deadline(), Some(answer), "1's answer");
    assert!(device.ack(out.invoke_id.unwrap(), T0));
    assert_eq!(device.reporter.next_deadline(), Some(T0), "2's turn");
    device.deadline();
    let (second, apdu) = device.pop(T0).unwrap();
    assert!(second.invoke_id.is_some());
    assert_eq!(reported(&apdu).0, 2);
}

/// While a report waits for an ID, one owed room still goes as room frees:
/// the outbox's one entry passes from one unconfirmed subscriber to the
/// next with no other change, and nothing comes due for the ID.
#[test]
fn a_report_owed_room_goes_while_one_waits_for_an_id() {
    let mut device = Device::new(OutboxLimits {
        entries: 1,
        octets: 256,
    });
    device.ids.limit = 0;
    device.subscribe(1, true, T0);
    assert!(device.pop(T0).is_none());
    device.subscribe_to(ai2(), 2, false, T0);
    device.subscribe_to(ai2(), 3, false, T0);
    assert_eq!(send(&mut device, T0), Some(2));
    assert_eq!(device.reporter.next_deadline(), Some(T0), "room freed");
    device.deadline();
    assert_eq!(send(&mut device, T0), Some(3));
    assert!(
        device.reporter.next_deadline().is_none_or(|due| due > T0),
        "the ID wait comes due for nothing"
    );
}

/// A confirmed notification's own deadline: its answer's, then its cutoff
/// while a resend waits, then the resend's answer's; none once answered.
#[test]
fn each_notification_out_has_its_deadline() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (out, _) = device.pop(T0).unwrap();
    let after = |seconds| T0 + Duration::from_secs(seconds);
    let deadline = |device: &Device| device.reporter.notification_deadline(out.token);
    assert_eq!(deadline(&device), Some(after(3)));
    device.deadline();
    // Three seconds a try, one retry: the cutoff is two tries and a half.
    assert_eq!(deadline(&device), Some(T0 + Duration::from_millis(7_500)));
    device.pop(after(3)).unwrap();
    assert_eq!(deadline(&device), Some(after(6)));
    assert!(device.ack(out.invoke_id.unwrap(), after(4)));
    assert_eq!(deadline(&device), None);
}

/// An ID a fence frees counts for the reports owed one: with one ID, A's
/// confirmed report holds it and B's waits; A renewing for unconfirmed
/// notifications fences A's report, and B goes. (A cancellation fences
/// nothing: the cancelled report runs on, as on the server, and its ID
/// comes back with its answer or its timeout.)
#[test]
fn an_id_a_fence_frees_lets_an_owed_report_go() {
    let mut device = Device::new(OutboxLimits::default());
    device.ids.limit = 1;
    device.subscribe(1, true, T0);
    assert!(device.pop(T0).is_some(), "A's report out");
    device.subscribe(2, true, T0);
    assert!(device.pop(T0).is_none(), "B's waits for the ID");
    device.subscribe(1, false, T0);
    let (initial, _) = device.pop(T0).unwrap();
    assert_eq!(initial.invoke_id, None, "A renewed, unconfirmed");
    assert_eq!(device.reporter.next_deadline(), Some(T0), "B's turn");
    device.deadline();
    let (b, apdu) = device.pop(T0).expect("B goes");
    assert!(b.invoke_id.is_some());
    assert_eq!(reported(&apdu).0, 2);
}

/// An ID counts only while a report waits for one: A's Ack, with nothing
/// owed, counts for nothing when C is refused an ID later, so the next
/// look is A's own and C stays owed. And the host's word that IDs are free
/// names how many are, so the one the reporter gave back itself isn't
/// counted twice: one look at those owed, for one free ID.
#[test]
fn ids_count_only_while_a_report_waits_and_only_once() {
    let mut device = Device::new(OutboxLimits::default());
    device.ids.limit = 1;
    device.subscribe(1, true, T0);
    let (a, _) = device.pop(T0).unwrap();
    assert!(device.ack(a.invoke_id.unwrap(), T0), "nothing owed yet");
    device.subscribe(2, true, T0);
    let (b, _) = device.pop(T0).expect("B takes the free ID");
    device.subscribe(3, true, T0);
    assert!(device.pop(T0).is_none(), "C's waits");
    let looked = device.reporter.looked;
    device.deadline();
    assert_eq!(device.reporter.looked - looked, 1, "A's look alone");
    assert_eq!(device.reporter.owed_ids.len(), 1, "C still owed");

    device.subscribe(4, true, T0);
    assert!(device.pop(T0).is_none(), "D's waits too");
    assert!(device.ack(b.invoke_id.unwrap(), T0));
    device.reporter.invoke_ids_released(1);
    device.deadline();
    // C looked at, D still owed: counted twice, both would be.
    assert_eq!(device.reporter.owed_ids.len(), 1, "one free ID, one look");
}
