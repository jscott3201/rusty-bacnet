//! Timer writes over WriteProperty (#2192): each property the Clause 12.57
//! state machine takes, and its refusals, through the server's handlers.

use super::lighting_required_rows::{assert_refused, db_with, read_wire, write_wire};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::timer::TimerObject;

fn timer() -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = TimerObject::new(3, "TMR-3").unwrap();
    object.set_pres_value_range(1_000, 600_000).unwrap();
    db_with(Box::new(object))
}

/// Timer_State, Timer_Running and Last_State_Change as read over the wire.
fn state(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 3] {
    [P::TIMER_STATE, P::TIMER_RUNNING, P::LAST_STATE_CHANGE].map(|p| read_wire(db, oid, p))
}

#[test]
fn present_value_starts_and_expires_the_timer_over_the_wire() {
    let (mut db, oid) = timer();
    // 5000 ms is 0x1388: IDLE_TO_RUNNING (1).
    write_wire(
        &mut db,
        oid,
        P::PRESENT_VALUE,
        PropertyValue::Unsigned(5000),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 1], vec![0x11], vec![0x91, 1]]);
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x22, 0x13, 0x88]);
    assert_eq!(read_wire(&db, oid, P::INITIAL_TIMEOUT), [0x22, 0x13, 0x88]);
    // Below and above the range, and the wrong datatype, change nothing.
    for (value, code) in [
        (PropertyValue::Unsigned(999), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Unsigned(600_001),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Enumerated(1), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_refused(
            write_wire(&mut db, oid, P::PRESENT_VALUE, value, None),
            code,
        );
    }
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x22, 0x13, 0x88]);
    // Zero while running: FORCED_TO_EXPIRED (5).
    write_wire(
        &mut db,
        oid,
        P::PRESENT_VALUE,
        PropertyValue::Unsigned(0),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 2], vec![0x10], vec![0x91, 5]]);
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x21, 0]);
}

#[test]
fn timer_running_starts_with_the_default_and_forces_expiry_over_the_wire() {
    let (mut db, oid) = timer();
    // FALSE while idle changes nothing.
    write_wire(
        &mut db,
        oid,
        P::TIMER_RUNNING,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 0], vec![0x10], vec![0x91, 0]]);
    // TRUE starts with Default_Timeout, 60000 ms (0xEA60).
    write_wire(
        &mut db,
        oid,
        P::TIMER_RUNNING,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 1], vec![0x11], vec![0x91, 1]]);
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x22, 0xea, 0x60]);
    // TRUE again restarts: RUNNING_TO_RUNNING (3).
    write_wire(
        &mut db,
        oid,
        P::TIMER_RUNNING,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, P::LAST_STATE_CHANGE), [0x91, 3]);
    write_wire(
        &mut db,
        oid,
        P::TIMER_RUNNING,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 2], vec![0x10], vec![0x91, 5]]);
    assert_refused(
        write_wire(
            &mut db,
            oid,
            P::TIMER_RUNNING,
            PropertyValue::Unsigned(1),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
    );
}

#[test]
fn timer_state_idle_clears_and_other_states_are_refused_over_the_wire() {
    let (mut db, oid) = timer();
    write_wire(
        &mut db,
        oid,
        P::PRESENT_VALUE,
        PropertyValue::Unsigned(5000),
        None,
    )
    .unwrap();
    for refused in [1, 2, 7] {
        assert_refused(
            write_wire(
                &mut db,
                oid,
                P::TIMER_STATE,
                PropertyValue::Enumerated(refused),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(state(&db, oid), [vec![0x91, 1], vec![0x11], vec![0x91, 1]]);
    // RUNNING_TO_IDLE (2).
    write_wire(
        &mut db,
        oid,
        P::TIMER_STATE,
        PropertyValue::Enumerated(0),
        None,
    )
    .unwrap();
    assert_eq!(state(&db, oid), [vec![0x91, 0], vec![0x10], vec![0x91, 2]]);
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x21, 0]);
}

#[test]
fn default_timeout_reliability_and_initial_timeout_writes_over_the_wire() {
    let (mut db, oid) = timer();
    write_wire(
        &mut db,
        oid,
        P::DEFAULT_TIMEOUT,
        PropertyValue::Unsigned(2000),
        None,
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, P::DEFAULT_TIMEOUT), [0x22, 0x07, 0xd0]);
    assert_refused(
        write_wire(
            &mut db,
            oid,
            P::DEFAULT_TIMEOUT,
            PropertyValue::Unsigned(999),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    // Initial_Timeout belongs to the state machine.
    assert_refused(
        write_wire(
            &mut db,
            oid,
            P::INITIAL_TIMEOUT,
            PropertyValue::Unsigned(2000),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // Reliability is the client's only while out of service.
    let fault = PropertyValue::Enumerated(7);
    assert_refused(
        write_wire(&mut db, oid, P::RELIABILITY, fault.clone(), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    write_wire(
        &mut db,
        oid,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    write_wire(&mut db, oid, P::RELIABILITY, fault, None).unwrap();
    assert_eq!(read_wire(&db, oid, P::RELIABILITY), [0x91, 7]);
    // Out of service, the state machine still answers writes.
    write_wire(
        &mut db,
        oid,
        P::TIMER_RUNNING,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert_eq!(read_wire(&db, oid, P::PRESENT_VALUE), [0x22, 0x07, 0xd0]);
}
