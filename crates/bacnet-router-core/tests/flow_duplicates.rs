//! Duplicate suppression of relayed Router-Busy and Router-Available
//! (#1810): kept by sender as well as data, since their effect is scoped
//! to the routes through that sender (Clauses 6.6.3.6, 6.6.3.7), and reset
//! by the opposite message from the same sender.

mod fixture;

use bacnet_router_core::npdu::{Mac, NetworkMessageType};
use bacnet_router_core::router::{Discard, Duplicates, Permissive};
use bacnet_router_core::table::Reachability;
use bacnet_router_core::{Instant, PortId};
use fixture::{
    Outcome, R7, TestRouter, at_ms, deliver_broadcast, learn_9_via_r7, message, networks,
    router_with, two_ports,
};

/// Another router on port 1, beside R7, serving network 8.
const R8: &[u8] = &[0x08];

/// A router with duplicate suppression on, network 9 behind R7 and
/// network 8 behind R8, both on port 1.
fn router() -> TestRouter {
    let on = fixture::options(|options| options.duplicates = Some(Duplicates::DEFAULT));
    let mut router = router_with(&two_ports(), on);
    learn_9_via_r7(&mut router, Instant::ZERO);
    let r8 = Mac::from_slice(R8).unwrap();
    assert!(
        router
            .table_mut()
            .add_learned(8, PortId(1), r8, Instant::ZERO)
    );
    router
}

fn busy(list: &[u16]) -> Vec<u8> {
    message(NetworkMessageType::ROUTER_BUSY_TO_NETWORK, &networks(list))
}

fn available(list: &[u16]) -> Vec<u8> {
    message(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &networks(list),
    )
}

/// `frame` from `sender` on port 1 at `millis`: whether it was applied and
/// passed on (`true`) or dropped as a repeat (`false`).
fn hear(router: &mut TestRouter, sender: &[u8], frame: &[u8], millis: u64) -> bool {
    let (outcome, sent) =
        deliver_broadcast(router, &mut Permissive, 1, sender, frame, at_ms(millis));
    if outcome == Outcome::Discarded(Discard::Duplicate) {
        assert!(sent.is_empty(), "a repeat is not passed on");
        false
    } else {
        assert_eq!(sent.len(), 1, "passed on to port 0");
        true
    }
}

fn state(router: &TestRouter, network: u16, millis: u64) -> Option<Reachability> {
    router
        .table()
        .effective_reachability(network, at_ms(millis))
}

#[test]
fn two_senders_on_one_port_each_have_their_message_applied() {
    let mut router = router();
    // The same empty list, which means everything behind each sender.
    assert!(hear(&mut router, R7, &busy(&[]), 0));
    assert!(hear(&mut router, R8, &busy(&[]), 1), "another sender");
    assert_eq!(state(&router, 9, 2), Some(Reachability::Busy));
    assert_eq!(state(&router, 8, 2), Some(Reachability::Busy));
    assert!(hear(&mut router, R7, &available(&[]), 3));
    assert!(hear(&mut router, R8, &available(&[]), 4), "another sender");
    assert_eq!(state(&router, 9, 5), Some(Reachability::Reachable));
    assert_eq!(state(&router, 8, 5), Some(Reachability::Reachable));
    assert_eq!(router.counters().duplicate_drops, 0);
}

#[test]
fn a_dropped_repeat_leaves_the_table_untouched() {
    let mut router = router();
    // Busy for 30 s from 0; a repeat at 500 ms, if applied, would hold
    // it until 30.5 s.
    assert!(hear(&mut router, R7, &busy(&[9]), 0));
    assert!(!hear(&mut router, R7, &busy(&[9]), 500));
    assert_eq!(state(&router, 9, 30_200), Some(Reachability::Reachable));

    // Available from R7, then a busy mark from elsewhere: a repeat of the
    // Available within the time to live doesn't lift it.
    let mut router = self::router();
    assert!(hear(&mut router, R7, &available(&[9]), 0));
    router.table_mut().mark_busy(9, at_ms(30_000));
    assert!(!hear(&mut router, R7, &available(&[9]), 500));
    assert_eq!(state(&router, 9, 600), Some(Reachability::Busy));
    assert_eq!(router.counters().duplicate_drops, 1);
}

#[test]
fn the_opposite_message_resets_the_record_for_its_sender() {
    let mut router = router();
    // Busy, Available, Busy, Available from R7 within one time to live:
    // each is applied.
    for (millis, frame, expected) in [
        (0, busy(&[9]), Reachability::Busy),
        (100, available(&[9]), Reachability::Reachable),
        (200, busy(&[9]), Reachability::Busy),
        (300, available(&[9]), Reachability::Reachable),
    ] {
        assert!(hear(&mut router, R7, &frame, millis), "at {millis} ms");
        assert_eq!(state(&router, 9, millis), Some(expected), "at {millis} ms");
    }
    assert_eq!(router.counters().duplicate_drops, 0);
    // Another sender's Busy forgets nothing of R7's: R7's Available
    // repeated is still dropped.
    assert!(hear(&mut router, R8, &busy(&[9]), 400));
    assert!(!hear(&mut router, R7, &available(&[9]), 500));
}

/// The resets are capped per sender within one time to live
/// ([`Duplicates::RESET_LIMIT`]), so a looped pair can't reset itself for
/// good: past the cap, the opposite's entry is kept and the next repeat is
/// dropped; once the entries expire, resets are allowed again.
#[test]
fn the_resets_are_capped_within_one_time_to_live() {
    assert_eq!(Duplicates::RESET_LIMIT, 4);
    let mut router = router();
    // Busy at 0 resets nothing; the next four reset their opposite; the
    // sixth finds no live copy of itself but may reset nothing more.
    let mut at = 0;
    for frame in [busy(&[9]), available(&[9])].iter().cycle().take(6) {
        assert!(hear(&mut router, R7, frame, at), "at {at} ms");
        at += 100;
    }
    assert_eq!(state(&router, 9, at), Some(Reachability::Reachable));
    // The seventh and eighth repeat live entries: dropped, table untouched.
    assert!(!hear(&mut router, R7, &busy(&[9]), 600));
    assert!(!hear(&mut router, R7, &available(&[9]), 700));
    assert_eq!(state(&router, 9, 700), Some(Reachability::Reachable));
    assert_eq!(router.counters().duplicate_drops, 2);
    // Every entry from R7 has expired by 1.6 s: resets start over.
    for (millis, frame) in [
        (1_600, busy(&[9])),
        (1_700, available(&[9])),
        (1_800, busy(&[9])),
    ] {
        assert!(hear(&mut router, R7, &frame, millis), "at {millis} ms");
    }
    assert_eq!(state(&router, 9, 1_800), Some(Reachability::Busy));
}
