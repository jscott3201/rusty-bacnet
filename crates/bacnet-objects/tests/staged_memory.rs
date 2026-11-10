//! What a staged request holds of an object's kept rows (#2414). A peer can
//! send thousands of writes to one object in one WritePropertyMultiple; the
//! stage keeps the rows they leave once, not once a write. Measured in live
//! heap octets through this binary's own allocator, which counts what is
//! allocated and not yet freed, on every thread (the save writer's too).
//! One test, so nothing else in the process allocates meanwhile.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
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

static LIVE: AtomicIsize = AtomicIsize::new(0);

struct LiveOctets;

// SAFETY: every allocation and free goes to System unchanged, so the
// GlobalAlloc contract holds as System's does; the count is one atomic,
// which never allocates.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for LiveOctets {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: the caller's contract is System.alloc's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: the block came from System.alloc with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: LiveOctets = LiveOctets;

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

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

fn text(label: String) -> PropertyValue {
    PropertyValue::CharacterString(label)
}

/// The live octets `object` holds once `writes` are staged on it, beyond
/// what it held before; the stage is then released.
fn staged_octets(object: &mut dyn BACnetObject, writes: &[PendingWrite]) -> isize {
    let before = live();
    let durable = object.durable_writes().expect("the object keeps its rows");
    let StageStep::Staged(wait) = durable.stage_writes(writes) else {
        panic!("the request stages");
    };
    wait.block();
    let held = live() - before;
    object.durable_writes().unwrap().release_staged_write(&wait);
    held
}

#[test]
fn a_stage_holds_the_rows_once_whatever_its_number_of_writes() {
    const WRITES: usize = 2_000;
    let storage = Arc::new(Memory::default());

    // The largest State_Text a kept object holds: 1,024 labels of 512
    // octets, 512 KiB. Each write of the request changes one label.
    let mut input = MultiStateInputObject::new(1, "MSI", 5)
        .unwrap()
        .with_reporting_persistence(storage.clone())
        .unwrap();
    let labels = (0..MAX_NUMBER_OF_STATES)
        .map(|state| {
            text(format!(
                "{state:0width$}",
                width = MAX_KEPT_STATE_TEXT_OCTETS
            ))
        })
        .collect();
    input
        .write_property(P::STATE_TEXT, None, PropertyValue::List(labels), None)
        .unwrap();
    let writes: Vec<PendingWrite> = (0..WRITES)
        .map(|at| PendingWrite {
            property: P::STATE_TEXT,
            array_index: Some(1),
            value: text(format!("label {at}")),
        })
        .collect();
    let held = staged_octets(&mut input, &writes);
    eprintln!("State_Text, {WRITES} writes: {held} octets held");
    // The saved snapshot, the served one and the writes themselves: a few
    // copies of 512 KiB at most. Kept once a write, it is about 1 GiB.
    assert!(held < 4 << 20, "{held} octets held by the stage");

    // Another row: Time_Delay written over and over on an Analog Input
    // whose kept Description and message texts are at their 16 KiB caps.
    let storage = Arc::new(Memory::default());
    let mut analog = AnalogInputObject::new(1, "AI", EngineeringUnits::NO_UNITS)
        .unwrap()
        .with_reporting_persistence(storage)
        .unwrap();
    let long = || text("x".repeat(16 * 1024));
    analog
        .write_property(P::DESCRIPTION, None, long(), None)
        .unwrap();
    let texts = PropertyValue::List(vec![long(), long(), long()]);
    analog
        .write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, None, texts, None)
        .unwrap();
    let writes: Vec<PendingWrite> = (0..WRITES)
        .map(|at| PendingWrite {
            property: P::TIME_DELAY,
            array_index: None,
            value: PropertyValue::Unsigned(at as u64),
        })
        .collect();
    let held = staged_octets(&mut analog, &writes);
    eprintln!("Time_Delay, {WRITES} writes: {held} octets held");
    // Kept once a write, the 64 KiB of texts are about 128 MiB.
    assert!(held < 1 << 20, "{held} octets held by the stage");
}
