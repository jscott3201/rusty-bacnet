//! Unit tests of the outbox: its bounds, order, compaction and dead entries.

use super::*;
use crate::cov::{CovNotificationKind, CovSubscription};
use bacnet_types::MacAddr;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn key(process: u32) -> CovSubscriptionKey {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[7]),
        subscriber_network: None,
        subscriber_process_identifier: process,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_observation: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
    .key()
    .unwrap()
}

/// Queue notification `ticket` for subscription `ticket`'s own process.
fn push(outbox: &mut Outbox, ticket: u64, params: &[u8]) -> bool {
    push_for(outbox, ticket, ticket as u32, params).queued
}

fn push_for(outbox: &mut Outbox, ticket: u64, process: u32, params: &[u8]) -> Pushed {
    let endpoint = SubscriberEndpoint::new(&[7], None);
    let begun = Instant::from_secs(0);
    outbox.push(
        Token::for_test(ticket),
        key(process),
        endpoint,
        false,
        begun,
        params,
    )
}

fn pop(outbox: &mut Outbox) -> Option<(u64, Vec<u8>)> {
    outbox
        .pop(|entry, params| (entry.token, params.to_vec()))
        .map(|(token, params)| {
            assert_eq!(token, Token::for_test(u64::from(params[0])));
            (u64::from(params[0]), params)
        })
}

#[test]
fn entries_leave_in_order_within_both_bounds() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 3,
        octets: 10,
    });
    assert!(push(&mut outbox, 1, &[1; 4]));
    assert!(push(&mut outbox, 2, &[2; 4]));
    assert!(!push(&mut outbox, 3, &[3; 3]), "past the octets");
    assert!(push(&mut outbox, 3, &[3; 2]));
    assert!(!push(&mut outbox, 4, &[4]), "past the entries");
    assert_eq!(pop(&mut outbox), Some((1, vec![1; 4])));
    assert_eq!(pop(&mut outbox), Some((2, vec![2; 4])));
    assert_eq!(pop(&mut outbox), Some((3, vec![3; 2])));
    assert_eq!(pop(&mut outbox), None);
    assert!(outbox.is_empty());
}

#[test]
fn queued_parameters_move_to_the_start_without_growing_the_buffer() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 4,
        octets: 10,
    });
    let capacity = outbox.buf.capacity();
    for ticket in 1..=2 {
        assert!(push(&mut outbox, ticket, &[ticket as u8; 4]));
    }
    assert_eq!(pop(&mut outbox), Some((1, vec![1; 4])));
    // Six octets free in all, but only two at the end.
    assert!(push(&mut outbox, 3, &[3; 6]));
    assert_eq!(outbox.buf.len(), 10, "compacted");
    assert_eq!(outbox.buf.capacity(), capacity);
    assert_eq!(pop(&mut outbox), Some((2, vec![2; 4])));
    assert_eq!(pop(&mut outbox), Some((3, vec![3; 6])));
}

#[test]
fn a_newer_report_replaces_the_queued_one_in_its_turn() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 2,
        octets: 8,
    });
    assert!(push_for(&mut outbox, 1, 1, &[1; 3]).queued);
    assert!(push_for(&mut outbox, 2, 2, &[2; 3]).queued);
    assert_eq!(outbox.queued(&key(1)), Some(3));
    // Subscription 1 again, longer: it fits only in place of the first,
    // which ends unsent; it keeps that turn.
    let pushed = push_for(&mut outbox, 4, 1, &[4; 5]);
    assert!(pushed.queued);
    assert_eq!(pushed.replaced, Some(Token::for_test(1)));
    assert_eq!(outbox.live_for(&key(1)), 1);
    assert_eq!(pop(&mut outbox), Some((4, vec![4; 5])));
    assert_eq!(pop(&mut outbox), Some((2, vec![2; 3])));
    // A replacement that doesn't fit takes the queued one with it.
    assert!(push_for(&mut outbox, 5, 1, &[5; 8]).queued);
    let pushed = push_for(&mut outbox, 6, 1, &[6; 9]);
    assert_eq!(
        (pushed.queued, pushed.replaced),
        (false, Some(Token::for_test(5)))
    );
    assert!(outbox.is_empty());
}

#[test]
fn dead_entries_give_back_their_slots_and_octets() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 2,
        octets: 8,
    });
    assert!(push(&mut outbox, 1, &[1; 4]));
    assert!(push(&mut outbox, 2, &[2; 4]));
    outbox.kill(|entry| entry.token == Token::for_test(1));
    let capacity = (outbox.buf.capacity(), outbox.entries.capacity());
    assert!(push(&mut outbox, 3, &[3; 4]), "the dead entry's room");
    assert_eq!((outbox.buf.capacity(), outbox.entries.capacity()), capacity);
    assert_eq!(pop(&mut outbox), Some((2, vec![2; 4])));
    assert_eq!(pop(&mut outbox), Some((3, vec![3; 4])));
}

#[test]
fn reservations_count_down_and_a_replacement_takes_only_its_growth() {
    let outbox = Outbox::new(OutboxLimits {
        entries: 2,
        octets: 12,
    });
    let mut room = outbox.room();
    assert!(room.take(10, None));
    assert!(!room.take(3, None), "two octets left");
    assert!(room.take(5, Some(4)), "one more octet in place of four");
    assert!(!room.take(2, None), "one octet left");
    assert!(room.take(1, None));
    assert!(!room.take(0, None), "no entry left");
    assert!(room.take(1, Some(3)), "a shorter one gives octets back");
    assert!(room.take(4, Some(2)));
    assert!(!room.take(1, Some(0)));
}
