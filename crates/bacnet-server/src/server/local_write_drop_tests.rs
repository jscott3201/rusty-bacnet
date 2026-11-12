//! A `write_local` dropped while its durable save runs, or while it waits
//! for the database guard to commit, lets go of what it staged at once
//! (#2493): storage goes back to what the Device serves, and the next write
//! on it is not held up for the stage's lifetime.
//!
//! The Device keeps its Description in storage (#2364). The first save the
//! storage sees is held until the test lets it go; the rest run at once.

use super::super::*;
use crate::server::clock::clocked_test_database;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use std::sync::Mutex as StdMutex;
use std::sync::mpsc as std_mpsc;
use tokio::sync::{Notify, oneshot};

/// A hang guard only: no test waits on it unless it has already failed.
const HANG: Duration = Duration::from_secs(30);

type Gate = (oneshot::Sender<()>, std_mpsc::Receiver<()>);

/// The Device's tags storage: every Description it was asked to keep, and a
/// gate that holds the next save.
#[derive(Default)]
struct Storage {
    saved: StdMutex<Vec<Option<String>>>,
    gate: StdMutex<Option<Gate>>,
    changed: Notify,
}

impl Storage {
    /// Hold the next save: it reports that it started, then waits for the
    /// returned sender to send or drop.
    fn hold_next(&self) -> (oneshot::Receiver<()>, std_mpsc::Sender<()>) {
        let (started, started_rx) = oneshot::channel();
        let (go, go_rx) = std_mpsc::channel();
        *self.gate.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    }

    /// The Description the storage holds now.
    fn latest(&self) -> Option<Option<String>> {
        self.saved.lock().unwrap().last().cloned()
    }

    /// Wait until the storage's latest Description is `expected`.
    async fn holds(&self, expected: Option<&str>) {
        let expected = expected.map(str::to_owned);
        tokio::time::timeout(HANG, async {
            loop {
                let changed = self.changed.notified();
                if self.latest().as_ref() == Some(&expected) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "storage never held {expected:?}; it has {:?}",
                self.saved.lock().unwrap()
            )
        });
    }
}

impl TagsPersistence for Storage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(None)
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        let gate = self.gate.lock().unwrap().take();
        if let Some((started, go)) = gate {
            let _ = started.send(());
            let _ = go.recv();
        }
        self.saved
            .lock()
            .unwrap()
            .push(snapshot.description.clone());
        self.changed.notify_one();
        Ok(())
    }
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 2493).unwrap()
}

async fn server(storage: &Arc<Storage>) -> BACnetServer<TestTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(2493, 0).unwrap();
    let object = DeviceObject::with_tags_persistence(
        DeviceConfig {
            identity: identity.clone(),
            ..DeviceConfig::default()
        },
        Arc::clone(storage) as Arc<dyn TagsPersistence>,
    )
    .unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(object)).unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .transport(TestTransport::new())
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap()
}

async fn write_description(server: &BACnetServer<TestTransport>, text: &str) -> Result<(), Error> {
    server
        .write_local(
            &device(),
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString(text.into()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

/// Whether the Device still holds a staged write.
async fn staged(server: &BACnetServer<TestTransport>) -> bool {
    let mut db = server.database().write().await;
    let device = db.get_mut(&device()).unwrap();
    device.durable_writes().unwrap().has_staged_write()
}

/// Let the task that releases a dropped write's stage run. Every ready task
/// runs on each yield of a current-thread test runtime, so the count only
/// has to cover the hand-offs; no time passes.
async fn let_the_release_run() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

/// What the Device serves for its Description.
async fn served(server: &BACnetServer<TestTransport>) -> String {
    let db = server.database().read().await;
    match db
        .get(&device())
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap()
    {
        PropertyValue::CharacterString(text) => text,
        other => panic!("Description is {other:?}"),
    }
}

/// Storage's latest Description for a Device serving `text`.
fn kept(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// The follow-up write on the same object saves at once and finds the
/// Device free.
async fn next_write_is_not_delayed(server: &BACnetServer<TestTransport>, storage: &Storage) {
    write_description(server, "next").await.unwrap();
    assert_eq!(
        super::super::durable_writes::busy_waits(server.database()),
        0,
        "the write found the Device busy with a forgotten stage"
    );
    assert_eq!(served(server).await, "next");
    storage.holds(Some("next")).await;
}

#[tokio::test]
async fn a_write_local_dropped_during_its_save_releases_the_stage_at_once() {
    let storage = Arc::new(Storage::default());
    let server = server(&storage).await;
    let before = served(&server).await;
    let (started, go) = storage.hold_next();

    // The save of "lobby" starts, and the caller drops the write there.
    tokio::select! {
        _ = write_description(&server, "lobby") => panic!("the held save finished"),
        started = started => started.unwrap(),
    }
    assert!(staged(&server).await, "the write staged its Description");

    // The release needs the guard, not the save, so it lands while the save
    // is still held.
    let_the_release_run().await;
    assert!(!staged(&server).await, "the dropped write left its stage");
    assert_eq!(served(&server).await, before, "the Device serves old text");

    // Once the held save is through, storage is put back to what it serves.
    drop(go);
    storage.holds(kept(&before)).await;
    next_write_is_not_delayed(&server, &storage).await;
}

#[tokio::test]
async fn a_write_local_dropped_waiting_for_the_guard_releases_the_stage_at_once() {
    let storage = Arc::new(Storage::default());
    let server = server(&storage).await;
    let before = served(&server).await;
    let (started, go) = storage.hold_next();

    // While the save of "lobby" is held the stage has dropped the guard, so
    // the test takes it. The write then has nothing left but the guard to
    // wait for.
    let guard;
    {
        let mut call = std::pin::pin!(write_description(&server, "lobby"));
        tokio::select! {
            _ = &mut call => panic!("the held save finished"),
            started = started => started.unwrap(),
        }
        guard = server.database().write().await;
        drop(go);
        storage.holds(Some("lobby")).await;
        // Polled until it waits on the guard; it must not finish.
        tokio::select! {
            _ = &mut call => panic!("the write finished under the test's guard"),
            () = let_the_release_run() => {}
        }
        // Dropped here, still waiting for the guard.
    }
    drop(guard);

    let_the_release_run().await;
    assert!(!staged(&server).await, "the dropped write left its stage");
    assert_eq!(served(&server).await, before, "the Device serves old text");
    storage.holds(kept(&before)).await;
    next_write_is_not_delayed(&server, &storage).await;
}

#[tokio::test]
async fn a_write_local_that_completes_releases_nothing_twice() {
    let storage = Arc::new(Storage::default());
    let server = server(&storage).await;
    write_description(&server, "lobby").await.unwrap();
    let_the_release_run().await;
    assert!(!staged(&server).await);
    assert_eq!(served(&server).await, "lobby");
    storage.holds(Some("lobby")).await;
    assert_eq!(
        storage.saved.lock().unwrap().len(),
        1,
        "a completed write saved once"
    );
}
