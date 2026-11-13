//! What making a staged request's writes copies of an object's kept
//! reporting rows (#2526). Each write is made under the database guard, so
//! a copy per write of large rows is time the guard is held. Measured in
//! heap octets allocated through this binary's own allocator, on every
//! thread; one test, so nothing else in the process allocates meanwhile.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::durable::{PendingWrite, StageStep};
use bacnet_objects::multistate::{
    MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES, MultiStateInputObject,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::write_config::{WriteConfigPersistence, WriteConfigSnapshot};
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every allocation and free goes to System unchanged, so the
// GlobalAlloc contract holds as System's does; the count is one atomic,
// which never allocates.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is System.alloc's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the block came from System.alloc with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Storage in memory: the last snapshot saved.
#[derive(Default)]
struct Memory(Mutex<Option<WriteConfigSnapshot>>);

impl WriteConfigPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// The writes one request makes in this test.
const WRITES: usize = 100;

/// Stage `writes` on `object`, then make them as the server does: the
/// octets allocated a write while they are made.
fn committed_octets(object: &mut dyn BACnetObject, writes: &[PendingWrite]) -> usize {
    let durable = object.durable_writes().expect("the object keeps its rows");
    let StageStep::Staged(wait) = durable.stage_writes(writes) else {
        panic!("the request stages");
    };
    wait.block();
    let before = ALLOCATED.load(Ordering::SeqCst);
    for write in writes {
        object
            .write_property(write.property, write.array_index, write.value.clone(), None)
            .unwrap();
    }
    let made = ALLOCATED.load(Ordering::SeqCst) - before;
    object.durable_writes().unwrap().release_staged_write(&wait);
    made / writes.len()
}

#[test]
fn a_kept_row_write_is_made_with_two_copies_of_the_rows() {
    // The largest State_Text a kept object holds, about 540 KiB of labels
    // in memory; each write changes one label. Made, a write copies the
    // rows to work out what it leaves (`kept`) and to keep the served state
    // for the steps still staged (`correct`); before #2526 it also copied
    // them for the working copy and the labels again inside it, about 2.2
    // MB a write.
    let mut input = MultiStateInputObject::new(1, "MSI", 5)
        .unwrap()
        .with_reporting_persistence(Arc::new(Memory::default()))
        .unwrap();
    let labels = (0..MAX_NUMBER_OF_STATES)
        .map(|state| {
            let label = format!("{state:0width$}", width = MAX_KEPT_STATE_TEXT_OCTETS);
            PropertyValue::CharacterString(label)
        })
        .collect();
    input
        .write_property(P::STATE_TEXT, None, PropertyValue::List(labels), None)
        .unwrap();
    let writes: Vec<_> = (0..WRITES)
        .map(|at| PendingWrite {
            property: P::STATE_TEXT,
            array_index: Some(1),
            value: PropertyValue::CharacterString(format!("label {at}")),
        })
        .collect();
    let made = committed_octets(&mut input, &writes);
    eprintln!("State_Text, {WRITES} writes made: {made} octets a write");
    assert!(made < 1536 << 10, "{made} octets allocated a write");

    // Time_Delay beside 64 KiB of kept Description and message texts: two
    // copies of them a write now, three before.
    let mut analog = AnalogInputObject::new(1, "AI", EngineeringUnits::NO_UNITS)
        .unwrap()
        .with_reporting_persistence(Arc::new(Memory::default()))
        .unwrap();
    let long = || PropertyValue::CharacterString("x".repeat(16 * 1024));
    analog
        .write_property(P::DESCRIPTION, None, long(), None)
        .unwrap();
    let texts = PropertyValue::List(vec![long(), long(), long()]);
    analog
        .write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, None, texts, None)
        .unwrap();
    let writes: Vec<_> = (0..WRITES)
        .map(|at| PendingWrite {
            property: P::TIME_DELAY,
            array_index: None,
            value: PropertyValue::Unsigned(at as u64),
        })
        .collect();
    let made = committed_octets(&mut analog, &writes);
    eprintln!("Time_Delay, {WRITES} writes made: {made} octets a write");
    assert!(made < 160 << 10, "{made} octets allocated a write");
}
