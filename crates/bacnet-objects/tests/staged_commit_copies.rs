//! What staging a request's writes, and making them, copies of an object's
//! kept reporting rows (#2526) and of its Tags (#2546). Each write is
//! staged and made under the database guard, so a copy per write of a
//! large state is time the guard is held. Measured in heap octets
//! allocated through this binary's own allocator, on every thread; the
//! tests take turns ([`one_at_a_time`]), so nothing else in the process
//! allocates meanwhile.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_encoding::constructed::encode_name_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::durable::{PendingWrite, StageStep};
use bacnet_objects::multistate::{
    MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES, MultiStateInputObject,
};
use bacnet_objects::object_profile::{MAX_TAGS, ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::write_config::{WriteConfigPersistence, WriteConfigSnapshot};
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

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

fn allocated() -> usize {
    ALLOCATED.load(Ordering::SeqCst)
}

/// Held for a whole test, so no other test in the process allocates
/// meanwhile (nextest runs each in a process of its own anyway).
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
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
    let _turn = one_at_a_time();
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

/// Tags storage in memory.
#[derive(Default)]
struct Tags(Mutex<Option<TagsSnapshot>>);

impl TagsPersistence for Tags {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn framed(tags: &[BACnetNameValue]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for tag in tags {
        encode_name_value(&mut octets, tag).unwrap();
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

/// The octets a request of `count` element writes allocates as it is
/// staged, and as its writes are made, on an object holding the most Tags
/// storage keeps: 1,024 tags whose names fill its 1 MiB, written once so
/// storage and the served list both hold them. Each write rewrites one of
/// eight with a short name.
fn tags_request(count: usize) -> (usize, usize) {
    let storage = Arc::new(Tags::default());
    let mut object =
        AnalogInputObject::with_tags_persistence(1, "AI", EngineeringUnits::NO_UNITS, storage)
            .unwrap();
    let tags: Vec<_> = (0..MAX_TAGS)
        .map(|at| BACnetNameValue::semantic(format!("{at:01018}")))
        .collect();
    let profile = ObjectProfile {
        tags: Some(Vec::new()),
        ..ObjectProfile::default()
    };
    object.set_profile(profile).unwrap();
    object
        .write_property(P::TAGS, None, framed(&tags), None)
        .unwrap();
    let writes: Vec<_> = (0..count)
        .map(|at| PendingWrite {
            property: P::TAGS,
            array_index: Some(at as u32 % 8 + 1),
            value: framed(&[BACnetNameValue::semantic(format!("w{at}"))]),
        })
        .collect();
    let before = allocated();
    let durable = object.durable_writes().expect("the object keeps its Tags");
    let StageStep::Staged(wait) = durable.stage_writes(&writes) else {
        panic!("the request stages");
    };
    wait.block();
    let staged = allocated() - before;
    let before = allocated();
    for write in &writes {
        object
            .write_property(write.property, write.array_index, write.value.clone(), None)
            .unwrap();
    }
    let made = allocated() - before;
    object.durable_writes().unwrap().release_staged_write(&wait);
    (staged, made)
}

/// A Tags write costs what it changes (#2546): the octets a request's stage
/// and its writes allocate grow, a write, by what the write changes, not by
/// the list it changes. The stage copies the list once and saves it once,
/// whatever its number of writes; before, each write's step copied it and
/// encoded it whole (about 3.3 MB a write), and each write made copied it
/// twice more (about 2.2 MB). The bound, 4 KiB a write, is far below one
/// copy of the list, about 1 MiB.
#[test]
fn a_tags_write_costs_what_it_changes() {
    let _turn = one_at_a_time();
    let (staged_100, made_100) = tags_request(WRITES);
    let (staged_200, made_200) = tags_request(2 * WRITES);
    let staged = (staged_200 - staged_100) / WRITES;
    let made = (made_200 - made_100) / WRITES;
    eprintln!("Tags at the cap: {staged} octets staged, {made} made, a write");
    assert!(staged < 4 << 10, "{staged} octets allocated a write's step");
    assert!(made < 4 << 10, "{made} octets allocated a write made");
    // Nor does the first write made copy the list: storage and the served
    // list hold it already.
    assert!(
        made_100 < WRITES * (4 << 10),
        "{made_100} octets for {WRITES} writes made"
    );
}
