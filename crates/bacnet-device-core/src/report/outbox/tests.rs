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
        (false, crate::report::NotifyService::Cov),
        begun,
        params,
    )
}

fn pop(outbox: &mut Outbox) -> Option<(u64, Vec<u8>)> {
    outbox.pop().map(|(_, entry, params)| {
        assert_eq!(entry.token, Token::for_test(u64::from(params[0])));
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

/// Queue event notification `ticket` to the recipient at link MAC `mac`.
#[cfg(feature = "event-reporting")]
fn push_event(outbox: &mut Outbox, ticket: u64, mac: u8, params: &[u8]) -> bool {
    let to = BACnetRecipient::Address(address(mac));
    let begun = Instant::from_secs(0);
    let priority = NetworkPriority::Normal;
    outbox
        .push_event(Token::for_test(ticket), (to, priority), true, begun, params)
        .queued
}

#[cfg(feature = "event-reporting")]
fn address(mac: u8) -> BACnetAddress {
    BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&[mac]),
    }
}

#[cfg(feature = "event-reporting")]
#[test]
fn a_held_entry_keeps_its_place_and_pop_passes_it_by() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 4,
        octets: 64,
    });
    assert!(push_event(&mut outbox, 1, 9, &[1]));
    assert!(push(&mut outbox, 2, &[2]));
    assert!(push_event(&mut outbox, 3, 8, &[3]));
    let (at, entry, params) = outbox.pop().expect("the event, oldest");
    assert_eq!(
        (at, entry.token, params.as_slice()),
        (0, Token::for_test(1), &[1][..])
    );
    outbox.hold(at, entry, &params, true);
    assert_eq!(outbox.held(), 1);
    assert!(outbox.holds_for(&address(9)));
    assert!(!outbox.holds_for(&address(8)));
    assert_eq!(pop(&mut outbox).map(|(ticket, _)| ticket), Some(2));
    assert_eq!(pop(&mut outbox).map(|(ticket, _)| ticket), Some(3));
    assert!(pop(&mut outbox).is_none(), "held, passed by");
    let held: Vec<_> = outbox
        .held_entries()
        .map(|(at, entry)| (at, entry.token))
        .collect();
    assert_eq!(held, vec![(0, Token::for_test(1))]);
    let (entry, params) = outbox.take_at(0);
    assert!(entry.counted);
    assert_eq!(params, vec![1]);
    assert!(outbox.is_empty());
}

#[cfg(feature = "event-reporting")]
#[test]
fn holding_an_entry_compacts_a_full_buffer_and_keeps_the_others() {
    // Two entries fill the octets; the held one's parameters go back at
    // the buffer's end, after the live ones move down.
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 2,
        octets: 4,
    });
    assert!(push_event(&mut outbox, 1, 9, &[1, 1]));
    assert!(push(&mut outbox, 2, &[2, 2]));
    let (at, entry, params) = outbox.pop().unwrap();
    outbox.hold(at, entry, &params, false);
    assert!(!push(&mut outbox, 3, &[3]), "both entries are taken");
    assert_eq!(pop(&mut outbox), Some((2, vec![2, 2])));
    let (entry, params) = outbox.take_at(0);
    assert_eq!((entry.token, params), (Token::for_test(1), vec![1, 1]));
    assert!(!entry.counted, "held behind another, not refused an ID");
}

#[cfg(feature = "event-reporting")]
#[test]
fn an_event_evicts_the_newest_cov_entry_not_a_held_one() {
    let mut outbox = Outbox::new(OutboxLimits {
        entries: 3,
        octets: 64,
    });
    assert!(push_event(&mut outbox, 1, 9, &[1]));
    let (at, entry, params) = outbox.pop().unwrap();
    outbox.hold(at, entry, &params, true);
    assert!(push(&mut outbox, 2, &[2]));
    assert!(push(&mut outbox, 3, &[3]));
    let to = (
        BACnetRecipient::Address(address(8)),
        NetworkPriority::Normal,
    );
    let begun = Instant::from_secs(0);
    let pushed = outbox.push_event(Token::for_test(4), to, false, begun, &[4]);
    assert!(pushed.queued);
    let evicted: Vec<_> = pushed.evicted.iter().map(|entry| entry.token).collect();
    assert_eq!(evicted, vec![Token::for_test(3)], "the newest COV entry");
    assert_eq!(outbox.held(), 1);
    assert_eq!(pop(&mut outbox).map(|(ticket, _)| ticket), Some(2));
    assert_eq!(pop(&mut outbox).map(|(ticket, _)| ticket), Some(4));
}
