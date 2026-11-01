//! The forwarder takes a new monotonic clock as a change of time base and
//! drops what storage timed on the old one, but a database pass that rebinds
//! its clocks for another clock's sake leaves it alone (#2179).

use super::storage::{MemoryPersistence, block_on, persistent};
use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::database::ObjectDatabase;
use crate::durable::StageStep;
use bacnet_types::enums::PropertyIdentifier as P;

struct NoWallClock;

impl ClockReader for NoWallClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        None
    }
}

/// A database holding a persistent forwarder on `clock`, with a staged
/// Subscribed_Recipients write whose save has run.
fn staged_on(clock: Arc<MonotonicClock>) -> (ObjectDatabase, ObjectIdentifier) {
    let storage = Arc::new(MemoryPersistence::default());
    let nf = persistent(&storage);
    let oid = nf.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(nf)).unwrap();
    db.set_monotonic_clock(Some(clock));
    let list = framed_subscriptions(&[subscription(device(9), 1, 10)]);
    let writes = db.get_mut(&oid).unwrap().durable_writes().unwrap();
    let StageStep::Staged(wait) = writes.stage_write(P::SUBSCRIBED_RECIPIENTS, None, &list) else {
        panic!("expected a staged write");
    };
    block_on(wait);
    (db, oid)
}

fn has_staged_write(db: &mut ObjectDatabase, oid: ObjectIdentifier) -> bool {
    let object = db.get_mut(&oid).unwrap();
    object.durable_writes().unwrap().has_staged_write()
}

#[test]
fn binding_the_wall_clock_or_the_waker_keeps_a_staged_write() {
    let (clock, _set) = manual_clock();
    let (mut db, oid) = staged_on(clock);
    assert!(has_staged_write(&mut db, oid));
    db.set_clock_reader(Some(Arc::new(NoWallClock)));
    db.set_deadline_waker(Some(Arc::new(|| {})));
    assert!(has_staged_write(&mut db, oid));
}

#[test]
fn a_new_monotonic_clock_drops_the_staged_write() {
    let (clock, _set) = manual_clock();
    let (mut db, oid) = staged_on(clock);
    let (other, _set_other) = manual_clock();
    db.set_monotonic_clock(Some(other));
    assert!(!has_staged_write(&mut db, oid));
}
