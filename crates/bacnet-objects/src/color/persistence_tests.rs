//! A colour object's Present_Value in storage (#1526): the file backend's
//! round trip and the files it refuses, a failed save tried again, and the
//! settle and drop that wait for the saves.

#[cfg(feature = "std")]
use std::path::PathBuf;
#[cfg(feature = "std")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::traits::BACnetObject;
#[cfg(feature = "std")]
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

// std: the file backend.
#[cfg(feature = "std")]
struct FileCase {
    directory: PathBuf,
    path: PathBuf,
}

// std: the file backend.
#[cfg(feature = "std")]
impl FileCase {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "rusty-color-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("output.bin");
        Self { directory, path }
    }

    fn store(&self) -> Arc<FileColorOutputPersistence> {
        Arc::new(FileColorOutputPersistence::new(&self.path).unwrap())
    }
}

// std: the file backend.
#[cfg(feature = "std")]
impl Drop for FileCase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

// std: the file backend.
#[cfg(feature = "std")]
fn color_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}

// std: the file backend.
#[cfg(feature = "std")]
fn ct_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap()
}

// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn color_round_trip_through_a_file() {
    let case = FileCase::new();
    let blue = BACnetXyColor::new(0.15, 0.06);
    {
        let mut color = ColorObject::new(1, "c")
            .unwrap()
            .with_output_persistence(case.store())
            .unwrap();
        assert!(!color.output_restored());
        color.set_present_value(blue).unwrap();
        // Dropping the object waits for its saves.
    }
    let color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(case.store())
        .unwrap();
    assert!(color.output_restored());
    assert_eq!(
        color
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        xy_value(blue)
    );
}

// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn temperature_round_trip_through_a_file() {
    let case = FileCase::new();
    {
        let mut ct = ColorTemperatureObject::new(1, "ct")
            .unwrap()
            .with_output_persistence(case.store())
            .unwrap();
        ct.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2700),
            None,
        )
        .unwrap();
        ct.wait_for_output_saves();
    }
    assert_eq!(
        case.store().load(ct_oid()).unwrap(),
        Some(ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Kelvin(2700)),
            default: None,
            description: None
        })
    );
    let ct = ColorTemperatureObject::new(1, "ct")
        .unwrap()
        .with_output_persistence(case.store())
        .unwrap();
    assert_eq!(
        ct.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(2700)
    );
}

/// Write `body` after a valid header for `oid`.
// std: the file backend.
#[cfg(feature = "std")]
fn write_body(case: &FileCase, oid: ObjectIdentifier, body: &[u8]) {
    let mut bytes = b"RBNCLR01".to_vec();
    bytes.extend_from_slice(&oid.encode());
    bytes.extend_from_slice(body);
    std::fs::write(&case.path, bytes).unwrap();
}

// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn corrupt_files_do_not_load() {
    let half = 0.5f32.to_be_bytes();
    let two = 2.0f32.to_be_bytes();
    let nan = f32::NAN.to_be_bytes();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty body", vec![]),
        ("unknown kind", vec![9, 0, 0, 0, 0]),
        ("short xy", [&[1u8][..], &half].concat()),
        ("long xy", [&[1u8][..], &half, &half, &[0]].concat()),
        ("x above 1.0", [&[1u8][..], &two, &half].concat()),
        ("NaN y", [&[1u8][..], &half, &nan].concat()),
        ("short kelvin", vec![2, 0, 0, 0x0F]),
        (
            "kelvin below 1000",
            [&[2u8][..], &999u32.to_be_bytes()].concat(),
        ),
        (
            "kelvin above 30000",
            [&[2u8][..], &30_001u32.to_be_bytes()].concat(),
        ),
    ];
    for (what, body) in cases {
        let case = FileCase::new();
        write_body(&case, color_oid(), &body);
        assert!(case.store().load(color_oid()).is_err(), "{what} loaded");
        assert!(
            ColorObject::new(1, "c")
                .unwrap()
                .with_output_persistence(case.store())
                .is_err(),
            "{what} built"
        );
    }

    // Another format's header, another object's file, garbage, oversize.
    let case = FileCase::new();
    std::fs::write(&case.path, b"RBNNCL01\x01\x00\x00\x00\x01").unwrap();
    assert!(case.store().load(color_oid()).is_err());
    write_body(&case, ct_oid(), &[2, 0, 0, 0x0F, 0xA0]);
    assert!(case.store().load(color_oid()).is_err());
    assert!(case.store().load(ct_oid()).unwrap().is_some());
    std::fs::write(&case.path, b"junk").unwrap();
    assert!(case.store().load(color_oid()).is_err());
    std::fs::write(&case.path, vec![0u8; 65]).unwrap();
    assert!(case.store().load(color_oid()).is_err());
}

/// Version 2 keeps a written default behind its own presence octet
/// (#1956); every cut of a saved file is refused, and a version 1 file
/// still loads, with no default.
// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn version_2_files_are_strict_and_version_1_still_loads() {
    let case = FileCase::new();
    let saved = ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Kelvin(3000)),
        default: Some(ColorOutputValue::Kelvin(0)),
        description: None,
    };
    case.store().save(ct_oid(), &saved).unwrap();
    let whole = std::fs::read(&case.path).unwrap();
    assert_eq!(&whole[..8], b"RBNCLR02");
    assert_eq!(case.store().load(ct_oid()).unwrap(), Some(saved));
    for length in 0..whole.len() {
        std::fs::write(&case.path, &whole[..length]).unwrap();
        assert!(case.store().load(ct_oid()).is_err(), "cut to {length}");
    }
    let refused = |at: usize, octets: &[u8]| {
        let mut damaged = whole.clone();
        damaged.splice(at..at + octets.len(), octets.iter().copied());
        std::fs::write(&case.path, &damaged).unwrap();
        case.store().load(ct_oid()).is_err()
    };
    // Body at 12: kind, then each value's presence octet and four octets.
    assert!(refused(13, &[2]), "a presence octet of 2 loaded");
    assert!(
        refused(14, &0u32.to_be_bytes()),
        "a Present_Value of 0 K loaded"
    );
    assert!(
        refused(19, &999u32.to_be_bytes()),
        "a default of 999 K loaded"
    );
    let mut longer = whole.clone();
    longer.push(0);
    std::fs::write(&case.path, &longer).unwrap();
    assert!(case.store().load(ct_oid()).is_err());

    write_body(&case, ct_oid(), &[2, 0, 0, 0x0B, 0xB8]);
    assert_eq!(
        case.store().load(ct_oid()).unwrap(),
        Some(ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Kelvin(3000)),
            default: None,
            description: None
        })
    );
}

/// A version 1 file loads, and saving what it held writes version 2, which
/// loads back the same.
// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn a_version_1_file_saves_back_as_version_2() {
    let case = FileCase::new();
    let half = 0.5f32.to_be_bytes();
    write_body(&case, color_oid(), &[&[1u8][..], &half, &half].concat());
    let loaded = case.store().load(color_oid()).unwrap().unwrap();
    assert_eq!(
        loaded,
        ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.5, 0.5))),
            default: None,
            description: None
        }
    );
    case.store().save(color_oid(), &loaded).unwrap();
    assert_eq!(&std::fs::read(&case.path).unwrap()[..8], b"RBNCLR02");
    assert_eq!(case.store().load(color_oid()).unwrap(), Some(loaded));
}

// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn the_wrong_kind_of_value_does_not_build() {
    let kelvin = Memory::holding(ColorOutputValue::Kelvin(3000));
    assert!(
        ColorObject::new(1, "c")
            .unwrap()
            .with_output_persistence(kelvin)
            .is_err()
    );
    let xy = Memory::holding(ColorOutputValue::Xy(BACnetXyColor::new(0.3, 0.3)));
    assert!(
        ColorTemperatureObject::new(1, "ct")
            .unwrap()
            .with_output_persistence(xy)
            .is_err()
    );
    assert!(FileColorOutputPersistence::new("").is_err());
}

/// Storage in memory that refuses the saves it is told to.
#[derive(Default)]
struct Memory {
    held: Mutex<Option<ColorOutputSnapshot>>,
    refusals: Mutex<usize>,
    saves: AtomicUsize,
}

impl Memory {
    // std: the file backend.
    #[cfg(feature = "std")]
    fn holding(value: ColorOutputValue) -> Arc<Self> {
        let memory = Self::default();
        *memory.held.lock().unwrap() = Some(ColorOutputSnapshot {
            present_value: Some(value),
            default: None,
            description: None,
        });
        Arc::new(memory)
    }

    fn held(&self) -> Option<ColorOutputValue> {
        self.held
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|saved| saved.present_value)
    }
}

impl ColorOutputPersistence for Memory {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(self.held.lock().unwrap().clone())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        {
            let mut refusals = self.refusals.lock().unwrap();
            if *refusals > 0 {
                *refusals -= 1;
                return Err(Error::Encoding("refused".into()));
            }
        }
        *self.held.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

#[cfg(feature = "durable-writes")]
#[test]
fn a_failed_save_is_tried_again_at_settle() {
    let storage = Arc::new(Memory::default());
    *storage.refusals.lock().unwrap() = 1;
    let mut ct = ColorTemperatureObject::new(1, "ct")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    let counters = ct.output_save_counters().expect("storage counts its saves");
    // The write isn't refused, though its save is; the counter shows it.
    ct.set_present_value(5000).unwrap();
    ct.wait_for_output_saves();
    assert_eq!(storage.held(), None);
    assert_eq!(counters.failed_saves(), 1);
    // The same value again is not saved again, as long as the last save
    // landed; here it failed, so it is.
    ct.durable_writes()
        .expect("the object keeps its output")
        .settle_forgotten_writes()
        .expect("a wait for the saves")
        .block();
    assert_eq!(storage.held(), Some(ColorOutputValue::Kelvin(5000)));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    // The retry landed: the count keeps the one failure and adds nothing.
    assert_eq!(counters.failed_saves(), 1);
    // Nothing changed and nothing failed: settling saves nothing more.
    ct.durable_writes()
        .unwrap()
        .settle_forgotten_writes()
        .unwrap()
        .block();
    ct.set_present_value(5000).unwrap();
    ct.wait_for_output_saves();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    assert_eq!(counters.failed_saves(), 1);
    assert!(
        ColorObject::new(1, "c")
            .unwrap()
            .output_save_counters()
            .is_none()
    );
}

#[test]
fn a_failed_save_is_tried_again_as_the_object_drops() {
    let storage = Arc::new(Memory::default());
    *storage.refusals.lock().unwrap() = 1;
    let mut color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    let blue = BACnetXyColor::new(0.15, 0.06);
    color.set_present_value(blue).unwrap();
    color.wait_for_output_saves();
    assert_eq!(storage.held(), None);
    drop(color);
    assert_eq!(storage.held(), Some(ColorOutputValue::Xy(blue)));
}

#[test]
fn clones_keep_no_storage() {
    let storage = Arc::new(Memory::default());
    let color = ColorObject::new(1, "c")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    let mut copy = color.clone();
    assert!(copy.durable_writes().is_none());
    copy.set_present_value(BACnetXyColor::new(0.2, 0.2))
        .unwrap();
    drop(copy);
    color.wait_for_output_saves();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
}

/// With Tags storage too, settling waits for both writers.
// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn settle_joins_the_tags_wait() {
    let case = FileCase::new();
    let tags = Arc::new(crate::object_profile::FileTagsPersistence::new(&case.path).unwrap());
    let storage = Arc::new(Memory::default());
    let mut color = ColorObject::with_tags_persistence(1, "c", tags)
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    color
        .set_present_value(BACnetXyColor::new(0.2, 0.2))
        .unwrap();
    let wait = color
        .durable_writes()
        .unwrap()
        .settle_forgotten_writes()
        .unwrap();
    wait.block();
    assert!(wait.is_ready());
    assert_eq!(
        storage.held(),
        Some(ColorOutputValue::Xy(BACnetXyColor::new(0.2, 0.2)))
    );
}
