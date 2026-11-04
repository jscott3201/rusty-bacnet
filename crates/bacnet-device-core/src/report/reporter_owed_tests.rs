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
