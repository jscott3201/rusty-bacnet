//! `get_notification_recipients` filtering tests — transition, weekday, and
//! time-window selection, including midnight-crossing windows and UTC offset.
//!
//! Split out of `tests/mod.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::make_time;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;

// -----------------------------------------------------------------------

fn make_dest(
    device_instance: u32,
    valid_days: u8,
    from: Time,
    to: Time,
    confirmed: bool,
    transitions: u8,
) -> BACnetDestination {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap();
    BACnetDestination {
        valid_days,
        from_time: from,
        to_time: to,
        recipient: BACnetRecipient::Device(dev_oid),
        process_identifier: device_instance,
        issue_confirmed_notifications: confirmed,
        transitions,
    }
}

#[test]
fn get_recipients_filters_by_transition() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();

    // Recipient 1: only TO_OFFNORMAL (bit 0)
    nc.add_destination(make_dest(
        10,
        0b0111_1111,
        make_time(0, 0),
        make_time(23, 59),
        false,
        0b0000_0001,
    ));
    // Recipient 2: only TO_NORMAL (bit 2)
    nc.add_destination(make_dest(
        20,
        0b0111_1111,
        make_time(0, 0),
        make_time(23, 59),
        true,
        0b0000_0100,
    ));
    // Recipient 3: all transitions
    nc.add_destination(make_dest(
        30,
        0b0111_1111,
        make_time(0, 0),
        make_time(23, 59),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    let now = make_time(12, 0);
    // BACnetDaysOfWeek: bit 0 = Monday (monday(0)..sunday(6), ASHRAE 135 Clause 21)
    let monday_bit = 0x01; // bit 0 = Monday

    // TO_OFFNORMAL should match recipients 1 and 3
    let r = get_notification_recipients(&db, 1, EventTransition::ToOffnormal, monday_bit, &now);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].1, 10); // process_id
    assert_eq!(r[1].1, 30);

    // TO_NORMAL should match recipients 2 and 3
    let r = get_notification_recipients(&db, 1, EventTransition::ToNormal, monday_bit, &now);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].1, 20);
    assert!(r[0].2); // recipient 2 is confirmed
    assert_eq!(r[1].1, 30);

    // TO_FAULT should match only recipient 3
    let r = get_notification_recipients(&db, 1, EventTransition::ToFault, monday_bit, &now);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].1, 30);
}

#[test]
fn get_recipients_filters_by_day() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(2, "NC-2").unwrap();

    // Recipient valid Mon-Fri only. BACnetDaysOfWeek bit 0 = Monday, so
    // Mon-Fri = bits 0..4 = 0b0001_1111.
    nc.add_destination(make_dest(
        10,
        0b0001_1111,
        make_time(0, 0),
        make_time(23, 59),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    let now = make_time(12, 0);

    // Monday (bit 0) — should match
    let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x01, &now);
    assert_eq!(r.len(), 1);

    // Friday (bit 4) — should match
    let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x10, &now);
    assert_eq!(r.len(), 1);

    // Sunday (bit 6) — should NOT match
    let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x40, &now);
    assert!(r.is_empty());

    // Saturday (bit 5) — should NOT match
    let r = get_notification_recipients(&db, 2, EventTransition::ToOffnormal, 0x20, &now);
    assert!(r.is_empty());
}

#[test]
fn get_recipients_filters_by_time_window() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(3, "NC-3").unwrap();

    // Recipient valid 08:00–17:00
    nc.add_destination(make_dest(
        10,
        0b0111_1111,
        make_time(8, 0),
        make_time(17, 0),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    // bit 0 = Monday per BACnetDaysOfWeek
    let monday_bit = 0x01;

    // 12:00 — inside window
    let r = get_notification_recipients(
        &db,
        3,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(12, 0),
    );
    assert_eq!(r.len(), 1);

    // 07:00 — before window
    let r = get_notification_recipients(
        &db,
        3,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(7, 0),
    );
    assert!(r.is_empty());

    // 18:00 — after window
    let r = get_notification_recipients(
        &db,
        3,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(18, 0),
    );
    assert!(r.is_empty());
}

#[test]
fn get_recipients_time_window_boundary_inclusive() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(4, "NC-4").unwrap();
    // Window 08:00–17:00; endpoints are inclusive.
    nc.add_destination(make_dest(
        10,
        0b0111_1111,
        make_time(8, 0),
        make_time(17, 0),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    let monday_bit = 0x01;

    // Exactly 08:00:00.00 — the start boundary, inclusive.
    let r = get_notification_recipients(
        &db,
        4,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(8, 0),
    );
    assert_eq!(r.len(), 1);

    // Exactly 17:00:00.00 — the end boundary, inclusive.
    let r = get_notification_recipients(
        &db,
        4,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(17, 0),
    );
    assert_eq!(r.len(), 1);

    // 07:59:59.99 — just before the window.
    let r = get_notification_recipients(
        &db,
        4,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(7, 59),
    );
    assert!(r.is_empty());
}

#[test]
fn get_recipients_overnight_window_crosses_midnight() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(5, "NC-5").unwrap();
    // Overnight window 22:00–02:00 (to < from → wraps past midnight).
    nc.add_destination(make_dest(
        10,
        0b0111_1111,
        make_time(22, 0),
        make_time(2, 0),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    let monday_bit = 0x01;

    // 23:00 — inside the overnight window (after from, before midnight).
    let r = get_notification_recipients(
        &db,
        5,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(23, 0),
    );
    assert_eq!(r.len(), 1);

    // 01:00 — inside the overnight window (after midnight, before to).
    let r = get_notification_recipients(
        &db,
        5,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(1, 0),
    );
    assert_eq!(r.len(), 1);

    // 12:00 — outside the overnight window (mid-day).
    let r = get_notification_recipients(
        &db,
        5,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(12, 0),
    );
    assert!(r.is_empty());

    // 03:00 — just after the window ends (to=02:00, exclusive on the far side).
    let r = get_notification_recipients(
        &db,
        5,
        EventTransition::ToOffnormal,
        monday_bit,
        &make_time(3, 0),
    );
    assert!(r.is_empty());
}

#[test]
fn get_recipients_day_filter_sunday_and_weekend_bits() {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(6, "NC-6").unwrap();
    // Sunday only: bit 6 = 0x40 per BACnetDaysOfWeek (sunday(6)).
    nc.add_destination(make_dest(
        10,
        0b0100_0000,
        make_time(0, 0),
        make_time(23, 59),
        false,
        0b0000_0111,
    ));
    db.add(Box::new(nc)).unwrap();

    let now = make_time(12, 0);

    // Sunday (bit 6 = 0x40) — should match.
    let r = get_notification_recipients(&db, 6, EventTransition::ToOffnormal, 0x40, &now);
    assert_eq!(r.len(), 1);

    // Monday (bit 0 = 0x01) — should NOT match a Sunday-only destination.
    let r = get_notification_recipients(&db, 6, EventTransition::ToOffnormal, 0x01, &now);
    assert!(r.is_empty());

    // Saturday (bit 5 = 0x20) — should NOT match.
    let r = get_notification_recipients(&db, 6, EventTransition::ToOffnormal, 0x20, &now);
    assert!(r.is_empty());
}

#[test]
fn local_day_and_time_day_bits_match_bacnet_days_of_week() {
    // Epoch 1970-01-01 was a Thursday. BACnetDaysOfWeek is monday(0)..sunday(6),
    // so Thu=bit3(0x08), Sun(epoch+3d)=bit6(0x40), Mon(epoch+4d)=bit0(0x01).
    assert_eq!(local_day_and_time(0, 0).0, 0x08, "epoch Thursday = bit 3");
    assert_eq!(local_day_and_time(3 * 86400, 0).0, 0x40, "Sunday = bit 6");
    assert_eq!(local_day_and_time(4 * 86400, 0).0, 0x01, "Monday = bit 0");
}

#[test]
fn local_day_and_time_offset_shifts_day_and_time() {
    // -60 min (east): UTC 23:00 Thu -> Fri 00:00 local (bit 4 = 0x10).
    let (bit, t) = local_day_and_time(23 * 3600, -60);
    assert_eq!(bit, 0x10, "UTC 23:00 Thu +60min rolls to Friday bit 4");
    assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
    // +60 min (west): UTC 00:30 Fri -> Thu 23:30 local (bit 3 = 0x08).
    let (bit, t) = local_day_and_time(86400 + 30 * 60, 60);
    assert_eq!(
        bit, 0x08,
        "UTC 00:30 next day -60min rolls back to Thursday"
    );
    assert_eq!((t.hour, t.minute), (23, 30));
}

#[test]
fn local_day_and_time_time_of_day_preserved_under_zero_offset() {
    // 12:34:56 UTC with 0 offset stays 12:34:56 local on the same day.
    let (_, t) = local_day_and_time(12 * 3600 + 34 * 60 + 56, 0);
    assert_eq!((t.hour, t.minute, t.second), (12, 34, 56));
}

#[test]
fn get_recipients_returns_empty_for_missing_class() {
    let db = ObjectDatabase::new();
    let r = get_notification_recipients(
        &db,
        99,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    );
    assert!(r.is_empty());
}

#[test]
fn get_recipients_returns_empty_for_empty_list() {
    let mut db = ObjectDatabase::new();
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    db.add(Box::new(nc)).unwrap();

    let r = get_notification_recipients(
        &db,
        1,
        EventTransition::ToOffnormal,
        0x01,
        &make_time(12, 0),
    );
    assert!(r.is_empty());
}

#[test]
fn event_state_change_transition_mapping() {
    use crate::event::EventStateChange;
    use bacnet_types::enums::EventState;

    let to_normal = EventStateChange {
        from: EventState::HIGH_LIMIT,
        to: EventState::NORMAL,
    };
    assert_eq!(to_normal.transition(), EventTransition::ToNormal);

    let to_fault = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::FAULT,
    };
    assert_eq!(to_fault.transition(), EventTransition::ToFault);

    let to_high = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::HIGH_LIMIT,
    };
    assert_eq!(to_high.transition(), EventTransition::ToOffnormal);

    let to_low = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::LOW_LIMIT,
    };
    assert_eq!(to_low.transition(), EventTransition::ToOffnormal);
}
