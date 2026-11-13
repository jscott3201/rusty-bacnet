//! A written Default_Color or Default_Color_Temperature kept with the output
//! (#1956), and a device of many colour objects holding no idle threads.

use alloc::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::{
    ColorOperationInProgress as InProgress, ErrorClass, ErrorCode, PropertyIdentifier as P,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::*;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::object_profile::{ObjectProfile, TagsPersistence, TagsSnapshot};
use crate::traits::BACnetObject;

/// Output storage in memory, saves counted, refusing them while `fail`.
#[derive(Default)]
struct Memory {
    held: Mutex<Option<ColorOutputSnapshot>>,
    saves: AtomicUsize,
    fail: core::sync::atomic::AtomicBool,
}

impl Memory {
    fn held(&self) -> Option<ColorOutputSnapshot> {
        self.held.lock().unwrap().clone()
    }
}

impl ColorOutputPersistence for Memory {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(self.held())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("refused".into()));
        }
        self.saves.fetch_add(1, Ordering::SeqCst);
        *self.held.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// Tags storage in memory, saves counted.
#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
#[derive(Default)]
struct Tags(Mutex<Option<TagsSnapshot>>, AtomicUsize);

impl TagsPersistence for Tags {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.1.fetch_add(1, Ordering::SeqCst);
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn red() -> BACnetXyColor {
    BACnetXyColor::new(0.64, 0.33)
}

fn d65() -> BACnetXyColor {
    BACnetXyColor::new(0.3127, 0.3290)
}

/// Color 1 on `storage`, configured with a D65 default as an application
/// would at each start.
fn color(storage: &Arc<Memory>) -> ColorObject {
    let mut color = ColorObject::new(1, "CLR")
        .unwrap()
        .with_output_persistence(Arc::clone(storage) as Arc<dyn ColorOutputPersistence>)
        .unwrap();
    color.set_default_color(d65()).unwrap();
    color
}

fn temperature(storage: &Arc<Memory>) -> ColorTemperatureObject {
    let mut ct = ColorTemperatureObject::new(1, "CT")
        .unwrap()
        .with_output_persistence(Arc::clone(storage) as Arc<dyn ColorOutputPersistence>)
        .unwrap();
    ct.set_default_color_temperature(4000).unwrap();
    ct
}

fn in_progress(object: &dyn BACnetObject) -> InProgress {
    match object.read_property(P::IN_PROGRESS, None).unwrap() {
        PropertyValue::Enumerated(raw) => InProgress::from_raw(raw),
        other => panic!("In_Progress read as {other:?}"),
    }
}

/// The acceptance case: a written zero default brings the last value back
/// at the next start, over the configured default.
#[test]
fn a_written_zero_default_keeps_the_last_value_at_the_next_start() {
    let storage = Arc::new(Memory::default());
    {
        let mut color = color(&storage);
        color.start_output();
        color.set_present_value(red()).unwrap();
        color
            .write_property(
                P::DEFAULT_COLOR,
                None,
                xy_value(BACnetXyColor::new(0.0, 0.0)),
                None,
            )
            .unwrap();
    }
    let mut color = color(&storage);
    assert_eq!(color.default_color(), BACnetXyColor::new(0.0, 0.0));
    color.start_output();
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(red())
    );
    assert_eq!(in_progress(&color), InProgress::IDLE);

    let storage = Arc::new(Memory::default());
    {
        let mut ct = temperature(&storage);
        ct.start_output();
        ct.set_present_value(2700).unwrap();
        ct.write_property(
            P::DEFAULT_COLOR_TEMPERATURE,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
    }
    let mut ct = temperature(&storage);
    assert_eq!(ct.default_color_temperature(), 0);
    ct.start_output();
    assert_eq!(
        ct.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Unsigned(2700)
    );
}

/// A non-zero written default comes back too, and drives the next start.
#[test]
fn a_written_default_survives_a_restart() {
    let storage = Arc::new(Memory::default());
    let blue = BACnetXyColor::new(0.15, 0.06);
    color(&storage)
        .write_property(P::DEFAULT_COLOR, None, xy_value(blue), None)
        .unwrap();
    assert_eq!(
        storage.held(),
        Some(ColorOutputSnapshot {
            present_value: None,
            default: Some(ColorOutputValue::Xy(blue)),
            description: None
        })
    );
    let mut color = color(&storage);
    color.start_output();
    assert_eq!(
        color.read_property(P::PRESENT_VALUE, None).unwrap(),
        xy_value(blue)
    );
}

/// A never-driven output with a written zero default stays undriven: no
/// Present_Value was kept.
#[test]
fn a_default_written_before_any_drive_keeps_the_output_undriven() {
    let storage = Arc::new(Memory::default());
    let zero = xy_value(BACnetXyColor::new(0.0, 0.0));
    color(&storage)
        .write_property(P::DEFAULT_COLOR, None, zero, None)
        .unwrap();
    let mut color = color(&storage);
    assert!(!color.output_restored());
    color.start_output();
    assert_eq!(in_progress(&color), InProgress::NOT_CONTROLLED);
}

#[test]
fn a_default_that_cannot_be_saved_is_refused_and_the_old_one_stays() {
    let storage = Arc::new(Memory::default());
    let mut ct = temperature(&storage);
    storage.fail.store(true, Ordering::SeqCst);
    let error = ct
        .write_property(
            P::DEFAULT_COLOR_TEMPERATURE,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM),
        "{error:?}"
    );
    assert_eq!(ct.default_color_temperature(), 4000);
    // A value the object refuses anyway is refused as before.
    storage.fail.store(false, Ordering::SeqCst);
    assert!(
        ct.write_property(
            P::DEFAULT_COLOR_TEMPERATURE,
            None,
            PropertyValue::Unsigned(99),
            None
        )
        .is_err()
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
}

/// The server's path: the default stages, a Present_Value change while it
/// is staged waits, and the write takes the saved default; the waiting
/// change is saved after it.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_default_is_taken_and_a_waiting_present_value_follows_it() {
    let storage = Arc::new(Memory::default());
    let mut color = color(&storage);
    color.start_output();
    color.wait_for_output_saves();
    let zero = xy_value(BACnetXyColor::new(0.0, 0.0));
    let write = PendingWrite {
        property: P::DEFAULT_COLOR,
        array_index: None,
        value: zero.clone(),
    };
    let StageStep::Staged(wait) = color.stage_writes(core::slice::from_ref(&write)) else {
        panic!("the default didn't stage");
    };
    wait.block();
    color.set_present_value(red()).unwrap();
    color.wait_for_output_saves();
    // The staged save holds the default with the colour served at the stage.
    assert_eq!(
        storage.held().unwrap().present_value,
        Some(ColorOutputValue::Xy(d65()))
    );
    color
        .write_property(P::DEFAULT_COLOR, None, zero, None)
        .unwrap();
    color.release_staged_write(&wait);
    color.wait_for_output_saves();
    assert_eq!(
        storage.held(),
        Some(ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Xy(red())),
            default: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.0, 0.0))),
            description: None
        })
    );
}

/// A staged default its request never makes: storage goes back to what
/// the object serves, the waiting Present_Value included.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_default_never_made_puts_the_served_state_back() {
    let storage = Arc::new(Memory::default());
    let mut ct = temperature(&storage);
    let write = PendingWrite {
        property: P::DEFAULT_COLOR_TEMPERATURE,
        array_index: None,
        value: PropertyValue::Unsigned(0),
    };
    let StageStep::Staged(wait) = ct.stage_writes(&[write]) else {
        panic!("the default didn't stage");
    };
    wait.block();
    ct.set_present_value(3000).unwrap();
    ct.release_staged_write(&wait);
    ct.wait_for_output_saves();
    assert_eq!(
        storage.held(),
        Some(ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Kelvin(3000)),
            default: None,
            description: None
        })
    );
    assert_eq!(ct.default_color_temperature(), 4000);
}

/// One request writing Tags and Default_Color stages both, with one wait,
/// and the release lets each go.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_writing_tags_and_the_default_stages_both() {
    let storage = Arc::new(Memory::default());
    let tags = Arc::new(Tags::default());
    let mut color = ColorObject::with_tags_persistence(1, "CLR", Arc::clone(&tags) as _)
        .unwrap()
        .with_output_persistence(Arc::clone(&storage) as _)
        .unwrap();
    color
        .set_profile(ObjectProfile {
            tags: Some(Vec::new()),
            ..ObjectProfile::default()
        })
        .unwrap();
    // Two tags, by a write of the array's size.
    let size = PropertyValue::Unsigned(2);
    let zero = xy_value(BACnetXyColor::new(0.0, 0.0));
    let writes = [
        PendingWrite {
            property: P::TAGS,
            array_index: Some(0),
            value: size.clone(),
        },
        PendingWrite {
            property: P::DEFAULT_COLOR,
            array_index: None,
            value: zero.clone(),
        },
    ];
    let StageStep::Staged(wait) = color.stage_writes(&writes) else {
        panic!("the writes didn't stage");
    };
    assert!(color.has_staged_write());
    wait.block();
    color.write_property(P::TAGS, Some(0), size, None).unwrap();
    color
        .write_property(P::DEFAULT_COLOR, None, zero, None)
        .unwrap();
    color.release_staged_write(&wait);
    assert!(!color.has_staged_write());
    color.wait_for_output_saves();
    color.wait_for_tag_saves();
    // One save each: the staged ones, taken.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(tags.1.load(Ordering::SeqCst), 1);
    assert_eq!(color.default_color(), BACnetXyColor::new(0.0, 0.0));
    assert_eq!(
        tags.0
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|saved| saved.tags.as_ref())
            .map(Vec::len),
        Some(2)
    );
}

/// The same joined stage released with neither write made, as when the
/// request failed first: both staged writes go, and storage keeps what the
/// object serves.
#[cfg(feature = "durable-writes")]
#[test]
fn a_joined_stage_released_unmade_lets_both_go() {
    let storage = Arc::new(Memory::default());
    let tags = Arc::new(Tags::default());
    let mut color = ColorObject::with_tags_persistence(1, "CLR", Arc::clone(&tags) as _)
        .unwrap()
        .with_output_persistence(Arc::clone(&storage) as _)
        .unwrap();
    color
        .set_profile(ObjectProfile {
            tags: Some(Vec::new()),
            ..ObjectProfile::default()
        })
        .unwrap();
    let writes = [
        PendingWrite {
            property: P::TAGS,
            array_index: Some(0),
            value: PropertyValue::Unsigned(1),
        },
        PendingWrite {
            property: P::DEFAULT_COLOR,
            array_index: None,
            value: xy_value(BACnetXyColor::new(0.0, 0.0)),
        },
    ];
    let StageStep::Staged(wait) = color.stage_writes(&writes) else {
        panic!("the writes didn't stage");
    };
    wait.block();
    color.release_staged_write(&wait);
    assert!(
        !color.has_staged_write(),
        "a staged write outlived its release"
    );
    color.wait_for_output_saves();
    assert_eq!(storage.held().and_then(|saved| saved.default), None);
}

/// Clearing a written default lets the configured one apply again, now and
/// after a restart.
#[test]
fn clearing_a_written_default_lets_the_configured_one_apply() {
    let storage = Arc::new(Memory::default());
    let zero = xy_value(BACnetXyColor::new(0.0, 0.0));
    color(&storage)
        .write_property(P::DEFAULT_COLOR, None, zero, None)
        .unwrap();
    let mut rebuilt = color(&storage);
    assert!(rebuilt.default_color_saved());
    rebuilt.clear_written_default().unwrap();
    assert!(!rebuilt.default_color_saved());
    assert_eq!(storage.held().unwrap().default, None);
    rebuilt.set_default_color(d65()).unwrap();
    assert_eq!(rebuilt.default_color(), d65());
    drop(rebuilt);
    assert_eq!(color(&storage).default_color(), d65());

    let storage = Arc::new(Memory::default());
    temperature(&storage)
        .write_property(
            P::DEFAULT_COLOR_TEMPERATURE,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
    let mut rebuilt = temperature(&storage);
    assert!(rebuilt.default_color_temperature_saved());
    rebuilt.clear_written_default().unwrap();
    rebuilt.set_default_color_temperature(4000).unwrap();
    drop(rebuilt);
    assert_eq!(temperature(&storage).default_color_temperature(), 4000);
}

/// A staged default whose save failed is refused, and the Present_Value
/// change that waited for it is saved at once.
#[cfg(feature = "durable-writes")]
#[test]
fn a_refused_staged_default_saves_the_present_value_that_waited() {
    let storage = Arc::new(Memory::default());
    let mut color = color(&storage);
    color.start_output();
    color.wait_for_output_saves();
    let zero = xy_value(BACnetXyColor::new(0.0, 0.0));
    let write = PendingWrite {
        property: P::DEFAULT_COLOR,
        array_index: None,
        value: zero.clone(),
    };
    storage.fail.store(true, Ordering::SeqCst);
    let StageStep::Staged(wait) = color.stage_writes(core::slice::from_ref(&write)) else {
        panic!("the default didn't stage");
    };
    wait.block();
    color.set_present_value(red()).unwrap();
    storage.fail.store(false, Ordering::SeqCst);
    assert!(
        color
            .write_property(P::DEFAULT_COLOR, None, zero, None)
            .is_err()
    );
    color.release_staged_write(&wait);
    color.wait_for_output_saves();
    assert_eq!(
        storage.held(),
        Some(ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Xy(red())),
            default: None,
            description: None
        })
    );
}

/// The acceptance case for the writer threads: a device of many colour
/// objects with storage, each saved once, holds no thread once they are
/// idle (#1956).
// std: counts the writer threads.
#[cfg(feature = "std")]
#[test]
fn many_idle_colour_objects_hold_no_writer_threads() {
    let objects: Vec<(ColorObject, Arc<Memory>)> = (0..64)
        .map(|instance| {
            let storage = Arc::new(Memory::default());
            let mut color = ColorObject::new(instance, format!("CLR-{instance}"))
                .unwrap()
                .with_output_persistence(Arc::clone(&storage) as _)
                .unwrap();
            color.set_present_value(red()).unwrap();
            (color, storage)
        })
        .collect();
    for (color, storage) in &objects {
        color.wait_for_output_saves();
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    }
    // Each wait is short once the thread has ended; the first one that
    // isn't fails the test.
    assert!(
        objects.iter().all(|(color, _)| color.writer_thread_ended()),
        "an idle object still holds a writer thread"
    );
}

/// A colour object without output storage pays one pointer for it, however
/// much a snapshot holds (#2364).
#[test]
fn an_object_without_output_storage_pays_one_pointer_for_it() {
    use core::mem::size_of;
    assert_eq!(size_of::<saving::OutputStorage>(), size_of::<usize>());
}
