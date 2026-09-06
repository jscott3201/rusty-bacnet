//! `resolve_transition_priority_ack` tests — per-transition Priority and
//! Ack_Required resolution against the referenced NotificationClass.
//!
//! Split out of `tests/mod.rs` to keep every file under the 700-LOC cap.

use super::super::*;

// -----------------------------------------------------------------------
// resolve_transition_priority_ack tests
// -----------------------------------------------------------------------

/// Build a NotificationClass whose instance matches `class_number`, with the
/// given per-transition `priority` and `ack_required` arrays, and add it to a
/// fresh ObjectDatabase. The struct fields are public so the test pins exact
/// values rather than relying on the all-255/all-false defaults.
fn make_nc_db_with(
    class_number: u32,
    priority: [u8; 3],
    ack_required: [bool; 3],
) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(class_number, "NC").unwrap();
    nc.priority = priority;
    nc.ack_required = ack_required;
    db.add(Box::new(nc)).unwrap();
    db
}

#[test]
fn resolve_priority_ack_offnormal_transition() {
    let db = make_nc_db_with(7, [50, 150, 250], [true, false, true]);
    let (p, a) = resolve_transition_priority_ack(&db, 7, EventTransition::ToOffnormal);
    assert_eq!(p, 50, "TO_OFFNORMAL priority is PRIORITY[0]");
    assert!(a, "TO_OFFNORMAL ack_required is ACK_REQUIRED bit 0");
}

#[test]
fn resolve_priority_ack_fault_transition() {
    let db = make_nc_db_with(7, [50, 150, 250], [true, false, true]);
    let (p, a) = resolve_transition_priority_ack(&db, 7, EventTransition::ToFault);
    assert_eq!(p, 150, "TO_FAULT priority is PRIORITY[1]");
    assert!(!a, "TO_FAULT ack_required is ACK_REQUIRED bit 1");
}

#[test]
fn resolve_priority_ack_normal_transition() {
    let db = make_nc_db_with(7, [50, 150, 250], [true, false, true]);
    let (p, a) = resolve_transition_priority_ack(&db, 7, EventTransition::ToNormal);
    assert_eq!(p, 250, "TO_NORMAL priority is PRIORITY[2]");
    assert!(a, "TO_NORMAL ack_required is ACK_REQUIRED bit 2");
}

#[test]
fn resolve_priority_ack_missing_class_falls_back_to_defaults() {
    let db = ObjectDatabase::new();
    // No NotificationClass with number 999 exists.
    let (p, a) = resolve_transition_priority_ack(&db, 999, EventTransition::ToOffnormal);
    assert_eq!(p, 255, "missing class falls back to lowest priority");
    assert!(!a, "missing class falls back to no acknowledgement");
}

#[test]
fn resolve_priority_ack_default_class_values() {
    // A freshly-constructed NotificationClass uses the BACnet defaults:
    // Priority = [255, 255, 255] and Ack_Required = [false, false, false].
    let db = make_nc_db_with(1, [255, 255, 255], [false, false, false]);
    for t in [
        EventTransition::ToOffnormal,
        EventTransition::ToFault,
        EventTransition::ToNormal,
    ] {
        let (p, a) = resolve_transition_priority_ack(&db, 1, t);
        assert_eq!(p, 255);
        assert!(!a);
    }
}

#[test]
fn resolve_priority_ack_finds_class_by_scan_when_oid_instance_differs() {
    // The Notification_Class property (42) differs from the object instance
    // (1), so the direct OID lookup misses and the scan fallback must find it.
    let mut db = ObjectDatabase::new();
    let mut nc = NotificationClass::new(1, "NC-1").unwrap();
    nc.notification_class = 42;
    nc.priority = [10, 20, 30];
    nc.ack_required = [false, true, false];
    db.add(Box::new(nc)).unwrap();

    let (p, a) = resolve_transition_priority_ack(&db, 42, EventTransition::ToFault);
    assert_eq!(p, 20);
    assert!(a, "TO_FAULT ack_required bit 1 set");
}
