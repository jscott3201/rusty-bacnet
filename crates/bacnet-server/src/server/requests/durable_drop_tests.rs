//! Whoever holds the object database's last handle drops it off the async
//! runtime: a server dropped without `stop()` (#1409), an application that
//! lets go through `drop_database_off_runtime`, and the tasks a `stop()`
//! leaves to settle and end runs once the application lets go (#1513). A
//! class whose save storage still holds no longer parks the runtime's
//! thread while it drops.
//!
//! Each test runs on a current-thread runtime, whose one thread is the
//! test's own.

use super::*;
use crate::server::drop_database_off_runtime;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::durable::{PendingWrite, StageStep};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::EnableDisable;
use std::sync::mpsc as std_mpsc;
use std::sync::Weak;
use std::thread::ThreadId;

/// Storage for one class that reports the thread it is dropped on. The
/// class's writer drops it last, once the class's own drop, which waits for
/// its saves, is over.
struct Reporting {
    storage: Arc<ClassStorage>,
    dropped: std_mpsc::Sender<ThreadId>,
}

impl NotificationClassPersistence for Reporting {
    fn load(&self, _class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        Ok(self.storage.load_saved())
    }

    fn save(
        &self,
        _class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        self.storage.store(snapshot)
    }
}

impl Drop for Reporting {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}

/// Class 1, kept in `storage`, and where its storage is dropped.
fn reporting_class(
    storage: &Arc<ClassStorage>,
) -> (NotificationClass, std_mpsc::Receiver<ThreadId>) {
    let (dropped, dropped_on) = std_mpsc::channel();
    let persistence = Arc::new(Reporting {
        storage: Arc::clone(storage),
        dropped,
    });
    let class = NotificationClass::with_persistence(
        1,
        "NC-1",
        persistence as Arc<dyn NotificationClassPersistence>,
    )
    .unwrap();
    (class, dropped_on)
}

/// A started server holding a Device and `class`, and the channel that
/// feeds it requests. With `dcc_audit`, it also reports through Audit
/// Reporter 1 and takes a DeviceCommunicationControl without a password, so
/// a timed disable's timer holds the database for the record its expiry
/// owes.
async fn serving(
    class: NotificationClass,
    dcc_audit: bool,
) -> (BACnetServer<TestTransport>, mpsc::Sender<ReceivedNpdu>) {
    let (transport, inbound) = TestTransport::inbound(4);
    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 100,
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut builder = BACnetServer::generic_builder()
        .transport(transport)
        .enable_event_enrollment(false);
    if dcc_audit {
        let logger = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
        device
            .provision_audit_recipient(BACnetRecipient::Device(logger))
            .unwrap();
        let reporter = AuditReporterObject::new(1, "reporter").unwrap();
        builder = builder
            .audit_reporters(AuditReportersConfig {
                reporters: vec![reporter.object_identifier()],
            })
            .dcc_policy(DccPolicy::LegacyPermissive)
            .device_binding(DeviceBinding::local(logger, [2]).unwrap())
            .unwrap();
        db.add(Box::new(reporter)).unwrap();
    }
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(class)).unwrap();
    let server = builder.database(db).build().await.unwrap();
    (server, inbound)
}

/// Put `server` under DISABLE_INITIATION for an hour through `inbound`, as a
/// peer would, so a timer that re-enables it is waiting.
async fn disable_initiation(
    server: &BACnetServer<TestTransport>,
    inbound: &mpsc::Sender<ReceivedNpdu>,
) {
    let mut request = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(60),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    }
    .encode(&mut request)
    .unwrap();
    let service = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    send(inbound, service, request.freeze()).await;
    tokio::time::timeout(WAIT, async {
        while server.comm_state() != DccState::DisableInitiation {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the disable took effect");
}

/// Send class 1 a Recipient_List write through `inbound` that stages a save
/// `storage` holds, and wait until the save has started. Returns the sender
/// whose drop lets the save through.
async fn stage_held_save(
    storage: &Arc<ClassStorage>,
    inbound: &mpsc::Sender<ReceivedNpdu>,
) -> std_mpsc::Sender<()> {
    let (started, go) = storage.hold();
    let request = write_property(1, &[destination(11)]);
    send(inbound, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
    save_started(started).await;
    go
}

/// Watch the runtime's thread while storage holds a save. Should whatever
/// the test does next hold that thread until storage lets the save go,
/// nothing would; the watchdog lets it go after [`WAIT`] instead, so the
/// test fails rather than hanging. Send on the returned channel from a task
/// to show the thread is free; the watchdog then lets the save go.
fn watchdog(go: std_mpsc::Sender<()>) -> (std_mpsc::Sender<()>, std::thread::JoinHandle<bool>) {
    let (progress, progressed) = std_mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        let stalled = progressed.recv_timeout(WAIT).is_err();
        drop(go);
        stalled
    });
    (progress, watchdog)
}

/// Whether the runtime's thread stayed free as the last handle on the
/// database `weak` watches went: once nothing holds one, a task spawned
/// then runs before the watchdog gives up only if the thread is free. A
/// database dropped on this thread holds it until the watchdog lets the
/// save go.
async fn thread_was_free(
    weak: Weak<RwLock<ObjectDatabase>>,
    progress: std_mpsc::Sender<()>,
    watchdog: std::thread::JoinHandle<bool>,
) -> bool {
    tokio::time::timeout(WAIT, async {
        while weak.strong_count() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("every handle on the database went");
    tokio::spawn(async move {
        let _ = progress.send(());
    })
    .await
    .unwrap();
    let stalled = tokio::task::spawn_blocking(move || watchdog.join().unwrap())
        .await
        .unwrap();
    !stalled
}

/// Check that the class went on another thread once storage let the save
/// go, and put storage back to the list it served as it went (#1363).
async fn dropped_off_the_runtime(
    dropped_on: std_mpsc::Receiver<ThreadId>,
    storage: &Arc<ClassStorage>,
) {
    let thread = tokio::task::spawn_blocking(move || dropped_on.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the database was dropped");
    assert_ne!(thread, std::thread::current().id());
    assert_eq!(storage.load_saved(), Some(snapshot(&[destination(1)])));
}

#[tokio::test]
async fn a_server_dropped_without_stop_lets_its_database_go_off_the_runtime() {
    let storage = holding(&[destination(1)]);
    let (class, dropped_on) = reporting_class(&storage);
    let (server, inbound) = serving(class, false).await;
    // A staged save that storage holds: the class's drop waits for it, and
    // then puts storage back to the list the class serves.
    let go = stage_held_save(&storage, &inbound).await;
    let weak = Arc::downgrade(server.database());
    let (progress, watchdog) = watchdog(go);
    drop(server);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "dropping the server held the runtime's thread"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn an_application_handle_let_go_of_last_drops_the_database_off_the_runtime() {
    let storage = holding(&[destination(1)]);
    let (class, dropped_on) = reporting_class(&storage);
    let (server, inbound) = serving(class, false).await;
    let db = Arc::clone(server.database());
    let go = stage_held_save(&storage, &inbound).await;
    drop(server);
    // The dropped server's task lets go of its handle only once the server's
    // tasks are done. Spin until it has, so the application's handle is the
    // last and the helper, not the server's task, drops the database.
    // Called sooner, the helper would just let go and return `None`.
    tokio::time::timeout(WAIT, async {
        while Arc::strong_count(&db) > 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the server let go of the database");
    let weak = Arc::downgrade(&db);
    let (progress, watchdog) = watchdog(go);
    let dropping = drop_database_off_runtime(db);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "letting go of the last handle held the runtime's thread"
    );
    dropping
        .expect("the last handle went to the blocking pool")
        .await
        .unwrap();
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn the_tasks_a_stop_leaves_let_the_database_go_off_the_runtime() {
    let storage = holding(&[destination(1)]);
    let (class, dropped_on) = reporting_class(&storage);
    let (mut server, inbound) = serving(class, false).await;
    let go = stage_held_save(&storage, &inbound).await;
    // The application reads the database through stop(), so stop() leaves
    // settling the staged write, and ending runs nothing owns, to tasks that
    // wait for the database.
    let reading = Arc::clone(server.database()).read_owned().await;
    let weak = Arc::downgrade(server.database());
    server.stop().await.unwrap();
    drop(server);
    let (progress, watchdog) = watchdog(go);
    // Those tasks, and the dropped server's, now hold the only handles; the
    // last to finish drops the database.
    drop(reading);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "a task stop() left held the runtime's thread"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn a_settle_task_that_ends_last_drops_the_database_off_the_runtime() {
    let storage = holding(&[destination(1)]);
    let (class, dropped_on) = reporting_class(&storage);
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(class)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    // A request stages a list write, as the server stages one, and is gone
    // before it makes the write. Storage holds the save.
    let (started, go) = storage.hold();
    let step = db
        .write()
        .await
        .get_mut(&nc(1))
        .and_then(|class| class.durable_writes_internal())
        .expect("the class saves")
        .stage_writes(&[PendingWrite {
            property: RECIPIENT_LIST,
            array_index: None,
            value: PropertyValue::ApplicationData(encoded(&[destination(11)])),
        }]);
    assert!(matches!(step, StageStep::Staged(_)));
    save_started(started).await;
    // The application reads the database as stop() settles it, so the
    // settling waits for it in a task of its own.
    let reading = Arc::clone(&db).read_owned().await;
    crate::server::durable_writes::settle_forgotten(&db).await;
    let weak = Arc::downgrade(&db);
    drop(db);
    let (progress, watchdog) = watchdog(go);
    // That task now holds the only handle, and drops the database.
    drop(reading);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "the settle task held the runtime's thread"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn a_dcc_timer_the_drop_cannot_take_lets_go_of_the_database() {
    let storage = holding(&[destination(1)]);
    let (class, dropped_on) = reporting_class(&storage);
    let (server, inbound) = serving(class, true).await;
    disable_initiation(&server, &inbound).await;
    let go = stage_held_save(&storage, &inbound).await;
    let weak = Arc::downgrade(server.database());
    // Something holds the timer's slot as the server drops, as a request
    // replacing the timer would, so the drop can't take the timer (#1560).
    let slot = Arc::clone(&server.dcc_timer);
    let held = slot.lock().await;
    let (progress, watchdog) = watchdog(go);
    drop(server);
    drop(held);
    drop(slot);
    // The timer's handle on the database goes with the server's, which
    // drops the database off the runtime, and not when the timer runs out.
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "the DCC timer held the database"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}
