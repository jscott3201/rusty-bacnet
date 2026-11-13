//! What a staged request holds of an object's kept rows (#2414), of a
//! log's reporting settings (#2431) and of each object family that stages
//! its own state (#2526). A peer can send thousands of writes to one object
//! in one WritePropertyMultiple: the server takes up to 10,000 (the
//! request decoder's item cap), and a request of 256 segments carries that
//! many small writes. The stage should keep what they leave once, not once
//! a write. Measured in live heap octets through this binary's own
//! allocator, which counts what is allocated and not yet freed, on every
//! thread (the save writer's too). The count is the whole process's, so the
//! tests take turns ([`one_at_a_time`]).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_encoding::constructed::{
    encode_access_rule, encode_device_object_property_reference, encode_name_value,
};
use bacnet_objects::access_control::{
    AccessRightsObject, AccessRightsPersistence, AccessRightsSnapshot, MAX_ACCESS_RULES,
};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::channel::{ChannelObject, MAX_CHANNEL_MEMBERS, MAX_CONTROL_GROUPS};
use bacnet_objects::durable::{PendingWrite, StageStep};
use bacnet_objects::load_control::{
    LoadControlObject, LoadControlPersistence, LoadControlSnapshot, MAX_WRITTEN_SHED_LEVELS,
};
use bacnet_objects::log_persistence::{LogPersistence, LogSnapshot};
use bacnet_objects::multistate::{
    MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES, MultiStateInputObject,
};
use bacnet_objects::object_profile::{MAX_TAGS, ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_objects::timer::{MAX_TIMER_REFERENCES, TimerObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_objects::write_config::{WriteConfigPersistence, WriteConfigSnapshot};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetLogRecord, BACnetNameValue, BACnetTimerStateChangeValue,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

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

/// Held for a whole test, so no other test in the process allocates
/// meanwhile (nextest runs each in a process of its own anyway).
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The writes one request makes in these tests.
const WRITES: usize = 2_000;

/// Storage in memory: the last snapshot saved.
struct Saved<S>(Mutex<Option<S>>);

impl<S> Default for Saved<S> {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl<S: Clone> Saved<S> {
    fn get(&self) -> Result<Option<S>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn put(&self, snapshot: &S) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// A Timer's, a Channel's and the reporting rows' storage.
type Memory = Saved<WriteConfigSnapshot>;

impl WriteConfigPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        self.get()
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        self.put(snapshot)
    }
}

impl TagsPersistence for Saved<TagsSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        self.get()
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.put(snapshot)
    }
}

impl AccessRightsPersistence for Saved<AccessRightsSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        self.get()
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error> {
        self.put(snapshot)
    }
}

impl LoadControlPersistence for Saved<LoadControlSnapshot> {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        self.get()
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        self.put(snapshot)
    }
}

fn text(label: String) -> PropertyValue {
    PropertyValue::CharacterString(label)
}

/// Description at its 16 KiB cap.
fn long() -> PropertyValue {
    text("x".repeat(16 * 1024))
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A member in another Device, the most a reference holds.
fn remote(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_remote(
        oid(ObjectType::ANALOG_VALUE, instance),
        P::PRESENT_VALUE.to_raw(),
        oid(ObjectType::DEVICE, 7),
    )
}

/// `WRITES` writes of `value(at)` to `property`, at `index(at)`.
fn writes(
    property: P,
    index: impl Fn(usize) -> Option<u32>,
    value: impl Fn(usize) -> PropertyValue,
) -> Vec<PendingWrite> {
    (0..WRITES)
        .map(|at| PendingWrite {
            property,
            array_index: index(at),
            value: value(at),
        })
        .collect()
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
    let _turn = one_at_a_time();
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

    // A log keeps its reporting settings the same way (#2431): a Trend Log
    // whose kept Description and message texts are at their 16 KiB caps,
    // its Notification_Class written over and over.
    let mut log =
        TrendLogObject::with_persistence(1, "TL", 16, Arc::new(LogMemory::default())).unwrap();
    log.write_property(P::DESCRIPTION, None, long(), None)
        .unwrap();
    let texts = PropertyValue::List(vec![long(), long(), long()]);
    log.write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, None, texts, None)
        .unwrap();
    let writes: Vec<PendingWrite> = (0..WRITES)
        .map(|at| PendingWrite {
            property: P::NOTIFICATION_CLASS,
            array_index: None,
            value: PropertyValue::Unsigned(at as u64),
        })
        .collect();
    let held = staged_octets(&mut log, &writes);
    eprintln!("Trend Log Notification_Class, {WRITES} writes: {held} octets held");
    // Kept once a write, the 64 KiB of texts are about 128 MiB.
    assert!(held < 1 << 20, "{held} octets held by the stage");
}

/// Tags as a write carries them: one for an element write, every one for a
/// whole one.
fn framed(tags: &[BACnetNameValue]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for tag in tags {
        encode_name_value(&mut octets, tag).unwrap();
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

/// The most Tags storage keeps: 1,024 tags whose names fill its 1 MiB,
/// written once so storage and the served list both hold them, then a
/// request rewriting a few of them with short names, so the list stays at
/// the cap. Kept once a write, the list is about 2 GiB (#2526).
#[test]
fn a_tags_stage_holds_the_list_once_whatever_its_number_of_writes() {
    let _turn = one_at_a_time();
    let storage = Arc::new(Saved::<TagsSnapshot>::default());
    let mut object =
        AnalogInputObject::with_tags_persistence(1, "AI", EngineeringUnits::NO_UNITS, storage)
            .unwrap();
    // Each name encodes in its length plus five octets; 1,024 of them come
    // within a kilobyte of the cap.
    let tags: Vec<_> = (0..MAX_TAGS)
        .map(|at| BACnetNameValue::semantic(format!("{at:01018}")))
        .collect();
    object
        .set_profile(ObjectProfile {
            tags: Some(Vec::new()),
            ..ObjectProfile::default()
        })
        .unwrap();
    object
        .write_property(P::TAGS, None, framed(&tags), None)
        .unwrap();
    let writes = writes(
        P::TAGS,
        |at| Some(at as u32 % 8 + 1),
        |at| framed(&[BACnetNameValue::semantic(format!("w{at}"))]),
    );
    // The stage's copy of the served list (storage swaps its old snapshot
    // for the new one, the same size) and the writes themselves, about 1.2
    // MiB: a step holds its write and nothing else (#2546). 1.5 MiB leaves
    // about 150 octets a write: a step that kept even one of these 1 KiB
    // tags beside its write would pass it.
    let held = staged_octets(&mut object, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Tags, {WRITES} writes: {held} octets held, {per_write} a write");
    assert!(held < 3 << 19, "{held} octets held by the stage");
}

/// A request of whole-array Tags writes, each of 1,024 tags with empty
/// names, about 2 KiB on the wire: a step holds the write, not the list it
/// writes, which is about 60 KiB decoded, so 2,000 of them held about 120
/// MiB (#2546).
#[test]
fn a_whole_tags_write_holds_its_write_not_its_list() {
    let _turn = one_at_a_time();
    let storage = Arc::new(Saved::<TagsSnapshot>::default());
    let mut object =
        AnalogInputObject::with_tags_persistence(1, "AI", EngineeringUnits::NO_UNITS, storage)
            .unwrap();
    object
        .set_profile(ObjectProfile {
            tags: Some(Vec::new()),
            ..ObjectProfile::default()
        })
        .unwrap();
    let empty = vec![BACnetNameValue::semantic(""); MAX_TAGS];
    let writes = writes(P::TAGS, |_| None, |_| framed(&empty));
    let held = staged_octets(&mut object, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Tags, {WRITES} whole writes: {held} octets held, {per_write} a write");
    // The write's own 2 KiB and its step, and the stage's one list.
    assert!(per_write < 3 << 10, "{per_write} octets held a write");
}

/// An Access Rights object's rule arrays, each written once with its 1,024
/// rules naming a time range and a zone in another Device, so storage and
/// the served object hold both; the request rewrites a few positive rules.
#[test]
fn an_access_rights_stage_holds_the_rules_once_whatever_its_number_of_writes() {
    let _turn = one_at_a_time();
    let storage = Arc::new(Saved::<AccessRightsSnapshot>::default());
    let mut rights = AccessRightsObject::with_persistence(1, "AR", storage).unwrap();
    let rule = |instance| {
        let zone = BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 7)),
            object_identifier: oid(ObjectType::ACCESS_ZONE, instance),
        };
        let schedule = BACnetDeviceObjectPropertyReference::new_remote(
            oid(ObjectType::SCHEDULE, instance),
            P::PRESENT_VALUE.to_raw(),
            oid(ObjectType::DEVICE, 7),
        );
        BACnetAccessRule::new(Some(schedule), Some(zone), true)
    };
    let rules: Vec<_> = (0..MAX_ACCESS_RULES as u32).map(rule).collect();
    let mut octets = BytesMut::new();
    for rule in &rules {
        encode_access_rule(&mut octets, rule);
    }
    let whole = PropertyValue::ApplicationData(octets.to_vec());
    for property in [P::POSITIVE_ACCESS_RULES, P::NEGATIVE_ACCESS_RULES] {
        rights
            .write_property(property, None, whole.clone(), None)
            .unwrap();
    }
    let writes = writes(
        P::POSITIVE_ACCESS_RULES,
        |at| Some(at as u32 % 8 + 1),
        |at| {
            let mut octets = BytesMut::new();
            encode_access_rule(&mut octets, &rule(at as u32));
            PropertyValue::ApplicationData(octets.to_vec())
        },
    );
    let held = staged_octets(&mut rights, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Access Rights, {WRITES} writes: {held} octets held, {per_write} a write");
    // The stage's copy of both served arrays (128 KiB) and the writes
    // themselves, each with the rule it sets, about 450 KiB. 768 KiB leaves
    // about 160 octets a write: a step keeping either array, or a few of
    // its rules beside its own, would pass it.
    assert!(held < 768 << 10, "{held} octets held by the stage");
}

/// References as a whole write of a list of them carries them.
fn references(
    references: impl IntoIterator<Item = BACnetDeviceObjectPropertyReference>,
) -> PropertyValue {
    let mut octets = BytesMut::new();
    for reference in references {
        encode_device_object_property_reference(&mut octets, &reference);
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

fn unsigned_list(values: impl IntoIterator<Item = u64>) -> PropertyValue {
    PropertyValue::List(values.into_iter().map(PropertyValue::Unsigned).collect())
}

/// A Channel whose 1,024 members in another Device, their delays, its 64
/// control groups and a 16 KiB Description were each written once, so
/// storage and the served configuration hold them all; the request
/// rewrites a few delays (#2526).
#[test]
fn a_channel_stage_holds_its_configuration_once_whatever_its_number_of_writes() {
    let _turn = one_at_a_time();
    let mut channel =
        ChannelObject::with_persistence(1, "CH", 1, Arc::new(Memory::default())).unwrap();
    let members = (0..MAX_CHANNEL_MEMBERS as u32)
        .map(|at| references([remote(at)]))
        .collect();
    for (property, value) in [
        (
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            PropertyValue::List(members),
        ),
        (
            P::EXECUTION_DELAY,
            unsigned_list(1..=MAX_CHANNEL_MEMBERS as u64),
        ),
        (
            P::CONTROL_GROUPS,
            unsigned_list(1..=MAX_CONTROL_GROUPS as u64),
        ),
        (P::DESCRIPTION, long()),
    ] {
        channel.write_property(property, None, value, None).unwrap();
    }
    let writes = writes(
        P::EXECUTION_DELAY,
        |at| Some(at as u32 % 8 + 1),
        |at| PropertyValue::Unsigned(at as u64),
    );
    let held = staged_octets(&mut channel, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Channel, {WRITES} writes: {held} octets held, {per_write} a write");
    // The stage's copy of what storage keeps of the served state and the
    // writes themselves, about 150 KiB. 512 KiB leaves about 180 octets a
    // write: a step keeping any of the lists above would pass it.
    assert!(held < 512 << 10, "{held} octets held by the stage");
}

/// A Timer whose 1,024 references in another Device and 16 KiB Description
/// were each written once, so storage and the served configuration hold
/// them; the request writes Default_Timeout over and over (#2526).
#[test]
fn a_timer_stage_holds_its_configuration_once_whatever_its_number_of_writes() {
    let _turn = one_at_a_time();
    let mut timer = TimerObject::with_persistence(1, "T", Arc::new(Memory::default())).unwrap();
    let value = || BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(1.0));
    timer
        .set_state_change(core::array::from_fn(|_| value()), vec![remote(0)], 16)
        .unwrap();
    let all = references((0..MAX_TIMER_REFERENCES as u32).map(remote));
    for (property, value) in [
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, all),
        (P::DESCRIPTION, long()),
    ] {
        timer.write_property(property, None, value, None).unwrap();
    }
    let writes = writes(
        P::DEFAULT_TIMEOUT,
        |_| None,
        |at| PropertyValue::Unsigned(1_000 + at as u64),
    );
    let held = staged_octets(&mut timer, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Timer, {WRITES} writes: {held} octets held, {per_write} a write");
    // The stage's copy of what storage keeps of the served state and the
    // writes themselves, about 150 KiB. 512 KiB leaves about 180 octets a
    // write: a step keeping any of the lists above would pass it.
    assert!(held < 512 << 10, "{held} octets held by the stage");
}

/// A Load Control whose 1,024 Shed_Levels elements and 16 KiB Description
/// were written, so storage and the served request hold them; the request
/// writes Duty_Window over and over (#2526).
#[test]
fn a_load_control_stage_holds_its_request_once_whatever_its_number_of_writes() {
    let _turn = one_at_a_time();
    let storage = Arc::new(Saved::<LoadControlSnapshot>::default());
    let mut control = LoadControlObject::with_persistence(1, "LC", storage).unwrap();
    let levels = MAX_WRITTEN_SHED_LEVELS as u64;
    control
        .set_shed_levels((1..=levels).map(|level| (level, String::new())).collect())
        .unwrap();
    let written = (1..=levels).map(PropertyValue::Unsigned).collect();
    control
        .write_property(P::SHED_LEVELS, None, PropertyValue::List(written), None)
        .unwrap();
    control
        .write_property(P::DESCRIPTION, None, long(), None)
        .unwrap();
    let writes = writes(
        P::DUTY_WINDOW,
        |_| None,
        |at| PropertyValue::Unsigned(at as u64),
    );
    let held = staged_octets(&mut control, &writes);
    let per_write = held / WRITES as isize;
    eprintln!("Load Control, {WRITES} writes: {held} octets held, {per_write} a write");
    // The stage's copy of what storage keeps of the served state and the
    // writes themselves, about 150 KiB. 512 KiB leaves about 180 octets a
    // write: a step keeping any of the lists above would pass it.
    assert!(held < 512 << 10, "{held} octets held by the stage");
}

/// A log's storage in memory: the last snapshot committed.
#[derive(Default)]
struct LogMemory(Mutex<Option<LogSnapshot<BACnetLogRecord>>>);

impl LogPersistence<BACnetLogRecord> for LogMemory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LogSnapshot<BACnetLogRecord>>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &LogSnapshot<BACnetLogRecord>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}
