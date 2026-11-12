//! A `write_local` dropped while its durable save runs, or while it waits
//! for the database guard to commit, lets go of what it staged at once
//! (#2493): storage goes back to what the Device serves, and the next write
//! on it is not held up for the stage's lifetime. A drop that finds the
//! guard taken leaves the release to a task, and `stop()` waits for it, for
//! a bounded time, before it settles the objects (#2495).
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

/// Drop a `write_local` of "lobby" while it waits for the guard, which the
/// test holds with the returned reader: its save has run, and the release of
/// its stage goes to a task that waits behind the reader.
///
/// Seeing some writer queued for the guard is not enough: the server's own
/// tasks queue for it too, the Schedule, intrinsic reporting, operation and
/// Trend Log passes soon after the server starts (#2504). The write would
/// then be dropped while its save still runs, before storage held "lobby".
async fn drop_waiting_for_the_guard(
    server: &BACnetServer<TestTransport>,
    storage: &Storage,
) -> tokio::sync::OwnedRwLockReadGuard<ObjectDatabase> {
    let releases = server.request_tasks.stage_releases();
    let (started, go) = storage.hold_next();
    let mut call = std::pin::pin!(write_description(server, "lobby"));
    tokio::select! {
        _ = &mut call => panic!("the held save finished"),
        started = started => started.unwrap(),
    }
    // While the save is held the stage has dropped the guard, so the test
    // can take a reader. Once the save is through, the write has nothing
    // left but the guard to wait for.
    let guard = Arc::clone(server.database()).read_owned().await;
    let saved = releases.stages_saved();
    drop(go);
    // The write notes its stage saved, then asks for the guard in that same
    // poll, so it is queued behind the reader by the time this ends.
    tokio::select! {
        _ = &mut call => panic!("the write finished under the test's guard"),
        () = stage_saved(releases, saved) => {}
    }
    assert!(
        server.database().try_read().is_err(),
        "the write is not waiting for the guard"
    );
    assert_eq!(storage.latest(), Some(Some("lobby".to_owned())));
    guard
}

/// Return once a stage counted in `releases` has seen its saves run since
/// `before` stages had.
async fn stage_saved(releases: &super::super::durable_writes::StageReleases, before: usize) {
    tokio::time::timeout(HANG, async {
        while releases.stages_saved() == before {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the write's save never finished");
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

    // The guard was free, so the stage went as the call dropped, while the
    // save is still held: no yield was needed.
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
    let guard = drop_waiting_for_the_guard(&server, &storage).await;
    drop(guard);

    let_the_release_run().await;
    assert!(!staged(&server).await, "the dropped write left its stage");
    assert_eq!(served(&server).await, before, "the Device serves old text");
    storage.holds(kept(&before)).await;
    next_write_is_not_delayed(&server, &storage).await;
}

#[tokio::test]
async fn a_write_task_aborted_during_its_save_has_released_when_the_abort_is_joined() {
    let storage = Arc::new(Storage::default());
    let server = Arc::new(server(&storage).await);
    let before = served(&server).await;
    let (started, go) = storage.hold_next();
    let writing = tokio::spawn({
        let server = Arc::clone(&server);
        async move { write_description(&server, "lobby").await }
    });
    started.await.unwrap();
    writing.abort();
    assert!(writing.await.unwrap_err().is_cancelled());
    // No yield since the join: the stage went as the task dropped, with the
    // guard free, so `stop()` joining an aborted run task has nothing of its
    // write left to settle.
    assert!(!staged(&server).await, "the aborted task left its stage");
    drop(go);
    storage.holds(kept(&before)).await;
}

/// Return once `stop()` waits for a release task.
async fn stop_waits_for(releases: &super::super::durable_writes::StageReleases) {
    tokio::time::timeout(HANG, async {
        while releases.waits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stop() never waited for the release task");
}

#[tokio::test]
async fn stop_waits_for_a_release_task_that_holds_the_guard_as_it_settles() {
    let storage = Arc::new(Storage::default());
    let mut server = server(&storage).await;
    let before = served(&server).await;
    let releases = Arc::clone(server.request_tasks.stage_releases());
    let (reached, release_go) = releases.hold_next();
    let reader = drop_waiting_for_the_guard(&server, &storage).await;
    assert_eq!(
        releases.outstanding(),
        1,
        "the release task was not counted"
    );
    // The reader goes, and the release task takes the guard and holds it at
    // its gate, before it releases: as a release still running when
    // `stop()` settles would.
    drop(reader);
    tokio::time::timeout(HANG, reached)
        .await
        .expect("the release task never took the guard")
        .unwrap();
    assert_eq!(storage.latest(), Some(Some("lobby".to_owned())));

    let mut release_go = Some(release_go);
    let waited = {
        let mut stopping = std::pin::pin!(server.stop());
        let waited = tokio::select! {
            result = &mut stopping => {
                result.unwrap();
                false
            }
            () = stop_waits_for(&releases) => true,
        };
        if waited {
            // Let the release through while `stop()` waits for it.
            drop(release_go.take());
            tokio::time::timeout(HANG, stopping)
                .await
                .expect("stop() hung after the release")
                .unwrap();
        }
        waited
    };
    // What storage holds the moment `stop()` returns, before anything else
    // runs.
    let at_stop = storage.latest();
    drop(release_go);
    assert_eq!(
        at_stop,
        Some(kept(&before).map(str::to_owned)),
        "stop() returned before storage went back to what the Device serves"
    );
    assert!(waited, "stop() did not wait for the release task");
    assert_eq!(releases.outstanding(), 0);
    assert!(!staged(&server).await, "the stage outlived stop()");
    assert_eq!(served(&server).await, before);
}

/// The earlier drop the test above could once make by accident (#2504): the
/// write goes while its save still runs and the guard is taken, so its
/// stage is released from a task with the save unfinished. Storage takes
/// the held save, then the served value, and `stop()` returns only after
/// both.
#[tokio::test]
async fn stop_waits_for_the_save_of_a_write_dropped_during_it_under_a_guard() {
    let storage = Arc::new(Storage::default());
    let mut server = server(&storage).await;
    let before = served(&server).await;
    let releases = Arc::clone(server.request_tasks.stage_releases());
    let (reached, release_go) = releases.hold_next();
    let (started, go) = storage.hold_next();
    let reader = {
        let mut call = std::pin::pin!(write_description(&server, "lobby"));
        tokio::select! {
            _ = &mut call => panic!("the held save finished"),
            started = started => started.unwrap(),
        }
        Arc::clone(server.database()).read_owned().await
        // Dropped here, its save held, with the guard taken.
    };
    assert_eq!(
        releases.outstanding(),
        1,
        "the release task was not counted"
    );
    drop(reader);
    tokio::time::timeout(HANG, reached)
        .await
        .expect("the release task never took the guard")
        .unwrap();
    assert_eq!(storage.latest(), None, "the held save landed");

    {
        let mut stopping = std::pin::pin!(server.stop());
        tokio::select! {
            _ = &mut stopping => panic!("stop() did not wait for the release task"),
            () = stop_waits_for(&releases) => {}
        }
        // The release goes with the save still held, and `stop()` moves on
        // to the saves the Device has queued.
        drop(release_go);
        tokio::select! {
            _ = &mut stopping => panic!("stop() returned while the dropped write still saved"),
            () = let_the_release_run() => {}
        }
        assert_eq!(releases.outstanding(), 0);
        drop(go);
        tokio::time::timeout(HANG, stopping)
            .await
            .expect("stop() hung after the save")
            .unwrap();
    }
    // The held save first, then the served value queued behind it.
    let kept_before = kept(&before).map(str::to_owned);
    assert_eq!(
        *storage.saved.lock().unwrap(),
        [Some("lobby".to_owned()), kept_before],
        "storage does not end at what the Device serves"
    );
    assert!(!staged(&server).await, "the stage outlived stop()");
    assert_eq!(served(&server).await, before);
}

#[tokio::test]
async fn stop_waits_for_a_release_behind_an_application_guard_only_so_long() {
    use super::super::durable_writes::stage_releases::RELEASE_WAIT;
    let storage = Arc::new(Storage::default());
    let mut server = server(&storage).await;
    let before = served(&server).await;
    let releases = Arc::clone(server.request_tasks.stage_releases());
    // The application holds the database across `stop()`.
    let reader = drop_waiting_for_the_guard(&server, &storage).await;
    assert_eq!(
        releases.outstanding(),
        1,
        "the release task was not counted"
    );

    tokio::time::pause();
    let started = tokio::time::Instant::now();
    tokio::time::timeout(HANG, server.stop())
        .await
        .expect("stop() waited on the application's guard")
        .unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= RELEASE_WAIT && waited < RELEASE_WAIT + Duration::from_secs(1),
        "stop() waited {waited:?} for the release task"
    );
    // Storage still leads the Device, as when nothing was waited for.
    assert_eq!(releases.outstanding(), 1);
    assert_eq!(storage.latest(), Some(Some("lobby".to_owned())));

    // Once the application lets go, the release and the settle run, and
    // storage goes back to what the Device serves.
    tokio::time::resume();
    drop(reader);
    storage.holds(kept(&before)).await;
    assert!(!staged(&server).await, "the stage outlived the guard");
    assert_eq!(releases.outstanding(), 0);
    assert_eq!(served(&server).await, before);
}

#[tokio::test]
async fn run_writes_count_their_release_tasks_where_stop_waits() {
    let storage = Arc::new(Storage::default());
    let server = server(&storage).await;
    let own = server.request_tasks.stage_releases();
    // Runs a request or WriteGroup starts get the server's count through
    // the task spawner, and those `write_local` starts through the server.
    let spawned = server.request_tasks.spawner().stage_releases();
    assert!(Arc::ptr_eq(&spawned, own));
    let runner = super::super::command_runs::CommandRunner::for_server(&server);
    assert!(Arc::ptr_eq(runner.writer().stage_releases, own));
    assert!(Arc::ptr_eq(server.local_writer().stage_releases, own));
}

#[tokio::test]
async fn a_completed_write_local_saves_once_and_leaves_nothing_staged() {
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
