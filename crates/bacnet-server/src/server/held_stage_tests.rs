//! An endpoint local write's [`HeldStage`] (#2468) dropped on a thread with
//! no runtime while a database guard is held still lets go of the stage:
//! the release goes to the runtime it staged on and waits for the guard.

use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_types::error::Error;

/// Tags storage that keeps nothing and answers at once.
struct Discard;

impl TagsPersistence for Discard {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(None)
    }

    fn save(&self, _: ObjectIdentifier, _: &TagsSnapshot) -> Result<(), Error> {
        Ok(())
    }
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap()
}

async fn staged(db: &Arc<RwLock<ObjectDatabase>>) -> bool {
    let mut db = db.write().await;
    let device = db.get_mut(&device()).unwrap();
    device.durable_writes().unwrap().has_staged_write()
}

#[tokio::test(start_paused = true)]
async fn a_held_stage_dropped_off_the_runtime_under_a_guard_still_releases() {
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(123, 0).unwrap(),
        ..Default::default()
    };
    let object = DeviceObject::with_tags_persistence(config, Arc::new(Discard)).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let db = Arc::new(RwLock::new(db));
    let value = PropertyValue::CharacterString("lobby".into());
    let targets = DurableTarget::local(device(), PropertyIdentifier::DESCRIPTION, None, &value);
    let held = stage_held(&db, targets).await;
    assert!(staged(&db).await, "the Description is staged");

    // Dropped on a plain thread while a reader holds the guard.
    let reader = db.read().await;
    std::thread::spawn(move || drop(held)).join().unwrap();
    drop(reader);

    for _ in 0..100 {
        if !staged(&db).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("the dropped stage was never released");
}

#[tokio::test(start_paused = true)]
async fn a_held_stage_dropped_with_the_guard_free_has_released_when_the_drop_returns() {
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(123, 0).unwrap(),
        ..Default::default()
    };
    let object = DeviceObject::with_tags_persistence(config, Arc::new(Discard)).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let db = Arc::new(RwLock::new(db));
    let value = PropertyValue::CharacterString("lobby".into());
    let targets = DurableTarget::local(device(), PropertyIdentifier::DESCRIPTION, None, &value);
    let held = stage_held(&db, targets).await;
    assert!(staged(&db).await, "the Description is staged");

    // No task runs between the drop and the check on this current-thread
    // runtime, so only a release made by the drop itself shows.
    drop(held);
    assert!(!staged(&db).await, "the drop left the release to a task");
}
