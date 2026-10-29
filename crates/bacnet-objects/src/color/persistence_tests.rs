//! A colour object's Present_Value in storage (#1526): the file backend's
//! round trip and the files it refuses, a failed save tried again, and the
//! settle and drop that wait for the saves.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

struct FileCase {
    directory: PathBuf,
    path: PathBuf,
}

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

impl Drop for FileCase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn color_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}

fn ct_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap()
}

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
            present_value: ColorOutputValue::Kelvin(2700)
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
fn write_body(case: &FileCase, oid: ObjectIdentifier, body: &[u8]) {
    let mut bytes = b"RBNCLR01".to_vec();
    bytes.extend_from_slice(&oid.encode());
    bytes.extend_from_slice(body);
    std::fs::write(&case.path, bytes).unwrap();
}

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
    fn holding(value: ColorOutputValue) -> Arc<Self> {
        let memory = Self::default();
        *memory.held.lock().unwrap() = Some(ColorOutputSnapshot {
            present_value: value,
        });
        Arc::new(memory)
    }

    fn held(&self) -> Option<ColorOutputValue> {
        self.held.lock().unwrap().map(|saved| saved.present_value)
    }
}

impl ColorOutputPersistence for Memory {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(*self.held.lock().unwrap())
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
        *self.held.lock().unwrap() = Some(*snapshot);
        Ok(())
    }
}

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
    ct.durable_writes_internal()
        .expect("the object keeps its output")
        .settle_forgotten_writes()
        .expect("a wait for the saves")
        .block();
    assert_eq!(storage.held(), Some(ColorOutputValue::Kelvin(5000)));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    // The retry landed: the count keeps the one failure and adds nothing.
    assert_eq!(counters.failed_saves(), 1);
    // Nothing changed and nothing failed: settling saves nothing more.
    ct.durable_writes_internal()
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
    assert!(copy.durable_writes_internal().is_none());
    copy.set_present_value(BACnetXyColor::new(0.2, 0.2))
        .unwrap();
    drop(copy);
    color.wait_for_output_saves();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
}

/// With Tags storage too, settling waits for both writers.
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
        .durable_writes_internal()
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
