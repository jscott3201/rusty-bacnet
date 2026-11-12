//! The file backend of a Load Control's shed request (#2479).

use super::*;
use crate::load_control::{FileLoadControlPersistence, LoadControlPersistence};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_file() -> PathBuf {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir()
        .join(format!(
            "rusty-bacnet-load-control-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ))
        .join("request")
}

fn store(path: &PathBuf) -> Arc<dyn LoadControlPersistence> {
    Arc::new(FileLoadControlPersistence::new(path).unwrap())
}

fn lc1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap()
}

#[test]
fn a_missing_file_loads_nothing_and_a_saved_one_round_trips() {
    let path = temp_file();
    let storage = FileLoadControlPersistence::new(&path).unwrap();
    assert_eq!(storage.path(), path);
    assert_eq!(storage.load(lc1()).unwrap(), None);
    // An object built on no file starts idle and writes nothing yet.
    let mut lc = LoadControlObject::with_persistence(1, "LC-1", store(&path)).unwrap();
    assert_eq!(present_value(&lc), INACTIVE);
    assert!(!path.exists());
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.write_property(
        P::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Chillers".into()),
        None,
    )
    .unwrap();
    lc.wait_for_saves();
    drop(lc);
    let saved = storage.load(lc1()).unwrap().unwrap();
    assert_eq!(saved.requested_shed_level, BACnetShedLevel::Level(2));
    assert_eq!(saved.description.as_deref(), Some("Chillers"));
    // Saved again as it was loaded, the octets don't change.
    let octets = std::fs::read(&path).unwrap();
    storage.save(lc1(), &saved).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), octets);
    let rebuilt = LoadControlObject::with_persistence(1, "LC-1", store(&path)).unwrap();
    assert_eq!(level(&rebuilt, P::REQUESTED_SHED_LEVEL), shed(2));
    // Another object's file is refused.
    let other = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 2).unwrap();
    assert!(storage.load(other).is_err());
    assert!(FileLoadControlPersistence::new("").is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_damaged_file_fails_the_build_and_is_left_as_it_was() {
    let path = temp_file();
    let storage = FileLoadControlPersistence::new(&path).unwrap();
    storage
        .save(lc1(), &crate::load_control::LoadControlSnapshot::default())
        .unwrap();
    let good = std::fs::read(&path).unwrap();
    let mut longer = good.clone();
    longer.push(0);
    let mut flag = good.clone();
    // The Enable octet, after the header and the first 45 octets.
    flag[12 + 45] = 7;
    for bad in [good[..good.len() - 1].to_vec(), longer, flag] {
        std::fs::write(&path, &bad).unwrap();
        assert!(LoadControlObject::with_persistence(1, "LC-1", store(&path)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bad);
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
