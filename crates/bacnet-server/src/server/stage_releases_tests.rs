//! A release task's place in the count goes with its future (#2495), so a
//! runtime that drops the task unrun does not leave `stop()` waiting for it.

use super::super::{DurableTarget, stage_held};
use super::*;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::RwLock;

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
    ObjectIdentifier::new(ObjectType::DEVICE, 2495).unwrap()
}

/// A database with one Device that keeps its Description in storage.
fn database() -> Arc<RwLock<ObjectDatabase>> {
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(2495, 0).unwrap(),
        ..Default::default()
    };
    let object = DeviceObject::with_tags_persistence(config, Arc::new(Discard)).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    Arc::new(RwLock::new(db))
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

/// Stage a Description write on `db` on `on`, counted in `releases`.
fn stage(
    on: &Runtime,
    db: &Arc<RwLock<ObjectDatabase>>,
    releases: &Arc<StageReleases>,
) -> super::super::HeldStage {
    let value = PropertyValue::CharacterString("lobby".into());
    let targets = DurableTarget::local(device(), PropertyIdentifier::DESCRIPTION, None, &value);
    on.block_on(stage_held(db, Some(releases), targets))
}

/// Whether a stop-time wait finds nothing outstanding without any time
/// passing.
fn waits_for_nothing(releases: &StageReleases) -> bool {
    let paused = Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    paused.block_on(async {
        let started = tokio::time::Instant::now();
        releases.wait_at_stop().await;
        started.elapsed().is_zero()
    })
}

#[test]
fn a_release_task_its_runtime_drops_unrun_gives_up_its_place() {
    let releases = Arc::new(StageReleases::default());
    let db = database();
    let staging = runtime();
    let held = stage(&staging, &db, &releases);
    // The guard is taken as the stage drops, so the release goes to a task
    // on `staging`, which nothing runs.
    let guard = db.try_write().unwrap();
    drop(held);
    assert_eq!(
        releases.outstanding(),
        1,
        "the release task was not counted"
    );
    drop(staging);
    assert_eq!(
        releases.outstanding(),
        0,
        "the task its runtime dropped kept its place"
    );
    drop(guard);
    assert!(waits_for_nothing(&releases));
}

#[test]
fn a_release_task_spawned_onto_a_runtime_already_gone_gives_up_its_place() {
    let releases = Arc::new(StageReleases::default());
    let db = database();
    let staging = runtime();
    let held = stage(&staging, &db, &releases);
    drop(staging);
    let guard = db.try_write().unwrap();
    drop(held);
    assert_eq!(
        releases.outstanding(),
        0,
        "the task a shut-down runtime refused kept its place"
    );
    drop(guard);
    assert!(waits_for_nothing(&releases));
}
