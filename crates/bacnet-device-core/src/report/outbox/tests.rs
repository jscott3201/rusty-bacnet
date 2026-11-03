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

fn push(outbox: &mut Outbox, ticket: u64, params: &[u8]) -> bool {
    let endpoint = SubscriberEndpoint::new(&[7], None);
    outbox.push(Token::for_test(ticket), key(1), endpoint, false, params)
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
    assert_eq!(outbox.head, 0, "compacted");
    assert_eq!(outbox.buf.capacity(), capacity);
    assert_eq!(pop(&mut outbox), Some((2, vec![2; 4])));
    assert_eq!(pop(&mut outbox), Some((3, vec![3; 6])));
}

#[test]
fn dead_entries_are_skipped_and_reservations_count_down() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 3,
        octets: 12,
    });
    for ticket in 1..=3 {
        assert!(push(&mut outbox, ticket, &[ticket as u8; 2]));
    }
    outbox.kill(|entry| entry.token == Token::for_test(2));
    assert_eq!(pop(&mut outbox), Some((1, vec![1; 2])));
    assert_eq!(pop(&mut outbox), Some((3, vec![3; 2])));
    assert_eq!(pop(&mut outbox), None);

    let mut room = outbox.room();
    assert!(room.take(10));
    assert!(!room.take(3), "two octets left");
    assert!(room.take(2));
    assert!(room.take(0), "one entry left after two");
    assert!(!room.take(0), "and none after three");
}
