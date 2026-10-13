//! Whichever endpoint holder lets go of the object database last drops it
//! off the async runtime (#1561): the session as it drops, stopped or not;
//! the server role's responder, held by a cloned role handle or by the
//! aborted dispatch task; the source Audit runtime, held by an audited read
//! in flight; the Number task's handle, upgraded across its lock wait; and a
//! registered Number owner. A durable class whose save storage still holds
//! no longer parks the runtime's thread while it drops.
//!
//! Each test runs on a current-thread runtime, whose one thread is the
//! test's own, and forces one holder to be the last by the order it lets
//! the others go in.
use super::*;
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_objects::durable::{PendingWrite, StageStep};
use bacnet_objects::notification_class::{
    NotificationClass, NotificationClassPersistence, NotificationClassSnapshot,
};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::BACnetDestination;
use bacnet_types::primitives::{PropertyValue, Time};
use std::sync::mpsc as std_mpsc;
use std::sync::{Mutex as StdMutex, Weak};
use std::thread::ThreadId;

#[path = "durable_stop_tests.rs"]
mod durable_stop;

const WAIT: Duration = Duration::from_secs(1);

fn destination(instance: u32) -> BACnetDestination {
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, instance)),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

fn snapshot(list: &[BACnetDestination]) -> NotificationClassSnapshot {
    NotificationClassSnapshot {
        recipient_list: Some(list.to_vec()),
    }
}

/// Notification Class 1's storage, which starts out holding
/// `[destination(1)]`. Once [`hold`](Self::hold) is called, each save
/// reports that it started and waits to be let go.
#[derive(Default)]
struct Storage {
    saved: StdMutex<Option<NotificationClassSnapshot>>,
    hold: StdMutex<Option<(std_mpsc::Sender<()>, std_mpsc::Receiver<()>)>>,
    attempts: std::sync::atomic::AtomicUsize,
    fail_on: std::sync::atomic::AtomicUsize,
}

impl Storage {
    fn holding() -> Arc<Self> {
        let storage = Arc::new(Self::default());
        *storage.saved.lock().unwrap() = Some(snapshot(&[destination(1)]));
        storage
    }

    fn hold(&self) -> (std_mpsc::Receiver<()>, std_mpsc::Sender<()>) {
        let (started, started_rx) = std_mpsc::channel();
        let (go, go_rx) = std_mpsc::channel();
        *self.hold.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    }
}

/// The class's persistence: [`Storage`], reporting the thread it is dropped
/// on. The class drops it once its own drop, which waits for its saves, is
/// over, so that is the thread the database was dropped on.
struct Reporting {
    storage: Arc<Storage>,
    dropped: std_mpsc::Sender<ThreadId>,
}

impl NotificationClassPersistence for Reporting {
    fn load(&self, _class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        Ok(self.storage.saved.lock().unwrap().clone())
    }

    fn save(
        &self,
        _class: ObjectIdentifier,
        saved: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        if let Some((started, go)) = &*self.storage.hold.lock().unwrap() {
            let _ = started.send(());
            let _ = go.recv();
        }
        let attempt = self.storage.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if self.storage.fail_on.load(Ordering::SeqCst) == attempt {
            return Err(Error::Encoding("injected durable save failure".into()));
        }
        *self.storage.saved.lock().unwrap() = Some(saved.clone());
        Ok(())
    }
}

impl Drop for Reporting {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}

/// Class 1 kept in `storage`, and where its storage is dropped.
fn reporting_class(storage: &Arc<Storage>) -> (NotificationClass, std_mpsc::Receiver<ThreadId>) {
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

/// Put `class` into the session's database before it starts.
fn add_class(endpoint: &mut Endpoint, class: NotificationClass) {
    let db = endpoint.session.database.as_mut().expect("a database");
    Arc::get_mut(db)
        .expect("unshared before start")
        .get_mut()
        .add(Box::new(class))
        .unwrap();
}

/// A started session of `role` on the capture link, holding a Device and
/// `class`.
async fn started_with(role: SessionRole, class: NotificationClass) -> Endpoint {
    let mut endpoint = Endpoint::new(role);
    let db = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .build_database()
        .unwrap();
    endpoint.session = endpoint.session.with_database(db);
    add_class(&mut endpoint, class);
    endpoint.start(false).await;
    endpoint
}

fn database(endpoint: &Endpoint) -> Arc<RwLock<ObjectDatabase>> {
    Arc::clone(endpoint.session.database.as_ref().unwrap())
}

/// Stage a Recipient_List write on class 1 that storage holds, as a request
/// would before it is gone, and wait until the save has started. Returns
/// the sender whose drop lets the save through.
async fn stage_held_save(db: &RwLock<ObjectDatabase>, storage: &Storage) -> std_mpsc::Sender<()> {
    let (started, go) = storage.hold();
    let mut list = BytesMut::new();
    encode_destination_list(&mut list, &[destination(11)]).unwrap();
    let step = db
        .write()
        .await
        .get_mut(&oid(ObjectType::NOTIFICATION_CLASS, 1))
        .and_then(|class| class.durable_writes_internal())
        .expect("the class saves")
        .stage_writes(&[PendingWrite {
            property: PropertyIdentifier::RECIPIENT_LIST,
            array_index: None,
            value: PropertyValue::ApplicationData(list.to_vec()),
        }]);
    assert!(matches!(step, StageStep::Staged(_)));
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    go
}

/// Watch the runtime's thread while storage holds a save. Should whatever
/// the test does next hold that thread until storage lets the save go,
/// nothing would; the watchdog lets it go after [`WAIT`] instead, so the
/// test fails rather than hanging. A task sending on the returned channel
/// shows the thread is free; the watchdog then lets the save go.
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
/// database `weak` watches went: once nothing holds one, a task spawned then
/// runs before the watchdog gives up only if the thread is free.
async fn thread_was_free(
    weak: Weak<RwLock<ObjectDatabase>>,
    progress: std_mpsc::Sender<()>,
    watchdog: std::thread::JoinHandle<bool>,
) -> bool {
    timeout(WAIT * 3, async {
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
async fn dropped_off_the_runtime(dropped_on: std_mpsc::Receiver<ThreadId>, storage: &Storage) {
    let thread = tokio::task::spawn_blocking(move || dropped_on.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the database was dropped");
    assert_ne!(thread, std::thread::current().id());
    assert_eq!(
        *storage.saved.lock().unwrap(),
        Some(snapshot(&[destination(1)]))
    );
}

#[tokio::test]
async fn a_session_dropped_last_lets_its_database_go_off_the_runtime() {
    for stop_first in [true, false] {
        let storage = Storage::holding();
        let (class, dropped_on) = reporting_class(&storage);
        let mut endpoint = started_with(SessionRole::ClientOnly, class).await;
        if stop_first {
            bounded(endpoint.session.stop()).await.unwrap();
        }
        let db = database(&endpoint);
        // For the stopped case, stage only after graceful settlement so this
        // fixture still makes Drop encounter an unfinished save.
        let go = stage_held_save(&db, &storage).await;
        let weak = Arc::downgrade(&db);
        drop(db);
        // The session keeps its handle through stop(); nothing else holds
        // one, so it is the last either way.
        let (progress, watchdog) = watchdog(go);
        drop(endpoint);
        assert!(
            thread_was_free(weak, progress, watchdog).await,
            "dropping the session held the runtime's thread (stopped first: {stop_first})"
        );
        dropped_off_the_runtime(dropped_on, &storage).await;
    }
}

#[tokio::test]
async fn a_responder_let_go_of_last_drops_the_database_off_the_runtime() {
    for by_handle in [true, false] {
        let storage = Storage::holding();
        let (class, dropped_on) = reporting_class(&storage);
        let mut endpoint = started_with(SessionRole::ServerOnly, class).await;
        let handle = if by_handle {
            let handle = endpoint.session.cloned_server_handle().unwrap();
            bounded(endpoint.session.stop()).await.unwrap();
            Some(handle)
        } else {
            None
        };
        let db = database(&endpoint);
        // Keep an unfinished save for the final holder's Drop, including
        // when the retained role outlives an already stopped session.
        let go = stage_held_save(&db, &storage).await;
        let weak = Arc::downgrade(&db);
        drop(db);
        // A cloned server role handle keeps the responder past a stopped
        // session's drop. Without stop(), the drop aborts the dispatch task,
        // which lets its responder go only once the runtime cancels it,
        // after the session's own handle has gone.
        let (progress, watchdog) = watchdog(go);
        drop(endpoint);
        drop(handle);
        assert!(
            thread_was_free(weak, progress, watchdog).await,
            "the responder held the runtime's thread (by handle: {by_handle})"
        );
        dropped_off_the_runtime(dropped_on, &storage).await;
    }
}

#[tokio::test]
async fn an_audited_read_in_flight_lets_the_source_database_go_off_the_runtime() {
    let storage = Storage::holding();
    let (class, dropped_on) = reporting_class(&storage);
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, None);
    add_class(&mut endpoint, class);
    endpoint.start(false).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    // Reading the database holds the audited read at its write lock, with
    // the source Audit runtime in hand once it has taken its permit.
    let reading = Arc::clone(&db).read_owned().await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let read = endpoint.read(EndpointApduDestination::Direct {
        destination_mac: mac(PEER),
    });
    bounded(async {
        while source.available_operations() == 64 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    drop(source);
    let weak = Arc::downgrade(&db);
    drop(db);
    let (progress, watchdog) = watchdog(go);
    // The session and its source let go first. Then the read takes the
    // lock, finds the source closed and returns, letting go of the last
    // handle.
    drop(endpoint);
    drop(reading);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "the source Audit runtime held the runtime's thread"
    );
    assert!(bounded(read).await.unwrap().is_err());
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn the_number_task_lets_its_upgraded_database_go_off_the_runtime() {
    let storage = Storage::holding();
    let (class, dropped_on) = reporting_class(&storage);
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, None);
    add_class(&mut endpoint, class);
    endpoint.start(false).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    // A learned number makes the Number task upgrade its handle and wait to
    // read the database for the source recipient, which this write holds.
    let writing = Arc::clone(&db).write_owned().await;
    let before = Arc::strong_count(&db);
    let [high, low] = THIS_NETWORK.to_be_bytes();
    let announcement = Bytes::copy_from_slice(&[1, 0x80, 0x13, high, low, 0]);
    endpoint.deliver(mac(ROUTER), announcement, true).await;
    bounded(async {
        while Arc::strong_count(&db) == before {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let weak = Arc::downgrade(&db);
    drop(db);
    let (progress, watchdog) = watchdog(go);
    // The drop aborts the task mid-wait; every other handle goes first.
    drop(endpoint);
    drop(writing);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "the Number task held the runtime's thread"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}

#[tokio::test]
async fn a_registered_number_owner_lets_its_database_go_off_the_runtime() {
    let storage = Storage::holding();
    let (class, dropped_on) = reporting_class(&storage);
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, Some(THIS_NETWORK));
    add_class(&mut endpoint, class);
    endpoint.start(false).await;
    let db = database(&endpoint);
    let go = stage_held_save(&db, &storage).await;
    let weak = Arc::downgrade(&db);
    drop(db);
    let (progress, watchdog) = watchdog(go);
    // The registered port's Number owner holds the database in the Number
    // task, which the drop aborts: it goes once the runtime cancels the
    // task, after the session and its source have let go.
    drop(endpoint);
    assert!(
        thread_was_free(weak, progress, watchdog).await,
        "the Number owner held the runtime's thread"
    );
    dropped_off_the_runtime(dropped_on, &storage).await;
}
