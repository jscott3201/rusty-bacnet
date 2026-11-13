//! How long a staged request's stage and its writes take, the time the
//! server holds its database guard for them (#2546), and the octets each
//! allocates: one object at its caps per family, N writes in one request.
//! A measurement, not a gate: run it in a release build, by hand,
//!
//! ```text
//! cargo nextest run --release -p bacnet-objects --test staged_timing \
//!     --run-ignored only --no-capture
//! ```
//!
//! and set STAGED_TIMING_WRITES (default 100,1000,10000) to choose N.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use bacnet_objects::multistate::{
    MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES, MultiStateInputObject,
};
use bacnet_objects::object_profile::{MAX_TAGS, ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_objects::timer::{MAX_TIMER_REFERENCES, TimerObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::write_config::{WriteConfigPersistence, WriteConfigSnapshot};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetNameValue, BACnetTimerStateChangeValue,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P};
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

impl WriteConfigPersistence for Saved<WriteConfigSnapshot> {
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

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn text(label: String) -> PropertyValue {
    PropertyValue::CharacterString(label)
}

fn long() -> PropertyValue {
    text("x".repeat(16 * 1024))
}

fn remote(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_remote(
        oid(ObjectType::ANALOG_VALUE, instance),
        P::PRESENT_VALUE.to_raw(),
        oid(ObjectType::DEVICE, 7),
    )
}

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

fn framed(tags: &[BACnetNameValue]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for tag in tags {
        encode_name_value(&mut octets, tag).unwrap();
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

fn rule(instance: u32) -> BACnetAccessRule {
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
}

fn rules(rules: &[BACnetAccessRule]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for rule in rules {
        encode_access_rule(&mut octets, rule);
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

fn writes(
    n: usize,
    property: P,
    index: impl Fn(usize) -> Option<u32>,
    value: impl Fn(usize) -> PropertyValue,
) -> Vec<PendingWrite> {
    (0..n)
        .map(|at| PendingWrite {
            property,
            array_index: index(at),
            value: value(at),
        })
        .collect()
}

/// One family at its caps, built afresh, and the request of `n` writes made
/// to it.
type Case = fn(usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>);

fn tags(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
    let storage = Arc::new(Saved::<TagsSnapshot>::default());
    let mut object =
        AnalogInputObject::with_tags_persistence(1, "AI", EngineeringUnits::NO_UNITS, storage)
            .unwrap();
    let all: Vec<_> = (0..MAX_TAGS)
        .map(|at| BACnetNameValue::semantic(format!("{at:01018}")))
        .collect();
    let profile = ObjectProfile {
        tags: Some(Vec::new()),
        ..ObjectProfile::default()
    };
    object.set_profile(profile).unwrap();
    object
        .write_property(P::TAGS, None, framed(&all), None)
        .unwrap();
    let writes = writes(
        n,
        P::TAGS,
        |at| Some(at as u32 % 8 + 1),
        |at| framed(&[BACnetNameValue::semantic(format!("w{at}"))]),
    );
    (Box::new(object), writes)
}

fn state_text(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
    let mut input = MultiStateInputObject::new(1, "MSI", 5)
        .unwrap()
        .with_reporting_persistence(Arc::new(Saved::<WriteConfigSnapshot>::default()))
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
    let writes = writes(
        n,
        P::STATE_TEXT,
        |_| Some(1),
        |at| text(format!("label {at}")),
    );
    (Box::new(input), writes)
}

fn access_rights(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
    let storage = Arc::new(Saved::<AccessRightsSnapshot>::default());
    let mut object = AccessRightsObject::with_persistence(1, "AR", storage).unwrap();
    let all: Vec<_> = (0..MAX_ACCESS_RULES as u32).map(rule).collect();
    for property in [P::POSITIVE_ACCESS_RULES, P::NEGATIVE_ACCESS_RULES] {
        object
            .write_property(property, None, rules(&all), None)
            .unwrap();
    }
    let writes = writes(
        n,
        P::POSITIVE_ACCESS_RULES,
        |at| Some(at as u32 % 8 + 1),
        |at| rules(&[rule(at as u32)]),
    );
    (Box::new(object), writes)
}

fn channel(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
    let storage = Arc::new(Saved::<WriteConfigSnapshot>::default());
    let mut channel = ChannelObject::with_persistence(1, "CH", 1, storage).unwrap();
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
        n,
        P::EXECUTION_DELAY,
        |at| Some(at as u32 % 8 + 1),
        |at| PropertyValue::Unsigned(at as u64),
    );
    (Box::new(channel), writes)
}

fn timer(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
    let storage = Arc::new(Saved::<WriteConfigSnapshot>::default());
    let mut timer = TimerObject::with_persistence(1, "T", storage).unwrap();
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
        n,
        P::DEFAULT_TIMEOUT,
        |_| None,
        |at| PropertyValue::Unsigned(1_000 + at as u64),
    );
    (Box::new(timer), writes)
}

fn load_control(n: usize) -> (Box<dyn BACnetObject>, Vec<PendingWrite>) {
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
        n,
        P::DUTY_WINDOW,
        |_| None,
        |at| PropertyValue::Unsigned(at as u64),
    );
    (Box::new(control), writes)
}

/// Stage `writes` on `object`, then make them, as the server does under its
/// guard: the time each takes and the octets each allocates.
fn measure(
    object: &mut dyn BACnetObject,
    writes: &[PendingWrite],
) -> (Duration, Duration, usize, usize) {
    let allocated = || ALLOCATED.load(Ordering::SeqCst);
    let (before, started) = (allocated(), Instant::now());
    let durable = object.durable_writes().expect("the object stages");
    let StageStep::Staged(wait) = durable.stage_writes(writes) else {
        panic!("the request stages");
    };
    let stage = started.elapsed();
    let staged = allocated() - before;
    wait.block();
    let (before, started) = (allocated(), Instant::now());
    for write in writes {
        object
            .write_property(write.property, write.array_index, write.value.clone(), None)
            .unwrap();
    }
    let commit = started.elapsed();
    let made = allocated() - before;
    object.durable_writes().unwrap().release_staged_write(&wait);
    (stage, commit, staged, made)
}

#[test]
#[ignore = "a release-build measurement, run by hand (#2546)"]
fn stage_and_commit_times_at_the_caps() {
    let counts: Vec<usize> = std::env::var("STAGED_TIMING_WRITES")
        .unwrap_or_else(|_| "100,1000,10000".into())
        .split(',')
        .map(|n| n.trim().parse().unwrap())
        .collect();
    let cases: [(&str, Case); 6] = [
        ("Tags", tags),
        ("State_Text", state_text),
        ("Access Rights", access_rights),
        ("Channel", channel),
        ("Timer", timer),
        ("Load Control", load_control),
    ];
    eprintln!(
        "| Family | N | Stage | Stage a write | Commit | Commit a write | Staged octets a write | Made octets a write |"
    );
    eprintln!("|---|---:|---:|---:|---:|---:|---:|---:|");
    for (family, case) in cases {
        for &n in &counts {
            let (mut object, writes) = case(n);
            let (stage, commit, staged, made) = measure(object.as_mut(), &writes);
            eprintln!(
                "| {family} | {n} | {stage:.2?} | {:.2?} | {commit:.2?} | {:.2?} | {} | {} |",
                stage / n as u32,
                commit / n as u32,
                staged / n,
                made / n
            );
        }
    }
}

/// What one Tags step costs at the cap, piece by piece: copying the list
/// (the working copy at a stage, the saved and served copies at a claim)
/// and encoding it whole (the file cap check at a stage).
#[test]
#[ignore = "a release-build measurement, run by hand (#2546)"]
fn tags_step_pieces_at_the_cap() {
    let all: Vec<_> = (0..MAX_TAGS)
        .map(|at| BACnetNameValue::semantic(format!("{at:01018}")))
        .collect();
    let snapshot = TagsSnapshot {
        tags: Some(all.clone()),
        description: None,
    };
    let rounds = 1_000u32;
    let started = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(all.clone());
    }
    let copy = started.elapsed() / rounds;
    let started = Instant::now();
    for _ in 0..rounds {
        let octets =
            bacnet_objects::snapshot::tags::encode(oid(ObjectType::ANALOG_INPUT, 1), &snapshot);
        std::hint::black_box(octets.unwrap());
    }
    let encode = started.elapsed() / rounds;
    eprintln!("Tags at the cap: copy the list {copy:.2?}, encode it {encode:.2?}");
}
