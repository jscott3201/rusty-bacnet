//! The start's paths that had no test of their own when it was split into
//! phases (#2558), each checked for what the phases before it leave: the
//! Audit routes refused before any output or link starts, the clocks bound
//! before the outputs start, a start dropped while it lets go of its link,
//! and a start dropped at its Staging pass once every task is spawned.
use std::future::Future;
use std::sync::atomic::AtomicBool;
use std::task::{Context, Waker};

use super::*;
use bacnet_objects::traits::{Clocks, Timed};

/// What an [`OutputWitness`] saw.
#[derive(Default)]
struct OutputLog {
    started: AtomicUsize,
    released: AtomicUsize,
    /// Whether the server's monotonic clock was bound when the output
    /// started.
    clocked_at_start: AtomicBool,
    /// The runtime's live tasks when the output was released.
    alive_at_release: AtomicUsize,
}

impl OutputLog {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn released(&self) -> usize {
        self.released.load(Ordering::SeqCst)
    }
}

/// A timed object that counts the outputs a start drives and releases, and
/// says which thread drops it.
struct OutputWitness {
    log: Arc<OutputLog>,
    clocked: bool,
    dropped: std_mpsc::Sender<ThreadId>,
}

impl Drop for OutputWitness {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}

impl Timed for OutputWitness {
    fn bind_clocks(&mut self, clocks: &Clocks) {
        self.clocked = clocks.monotonic.is_some();
    }

    fn start_output(&mut self) {
        self.log.started.fetch_add(1, Ordering::SeqCst);
        self.log
            .clocked_at_start
            .store(self.clocked, Ordering::SeqCst);
    }

    fn release_output(&mut self) {
        self.log.released.fetch_add(1, Ordering::SeqCst);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let alive = runtime.metrics().num_alive_tasks();
            self.log.alive_at_release.store(alive, Ordering::SeqCst);
        }
    }
}

impl BACnetObject for OutputWitness {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(ObjectType::BINARY_VALUE, 7)
    }
    fn object_name(&self) -> &str {
        "Outputs"
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
    fn timed(&mut self) -> Option<&mut dyn Timed> {
        Some(self)
    }
}

/// `db` with an [`OutputWitness`] added, its log, and where it says it was
/// dropped.
fn witnessed(
    mut db: ObjectDatabase,
) -> (ObjectDatabase, Arc<OutputLog>, std_mpsc::Receiver<ThreadId>) {
    let log = Arc::new(OutputLog::default());
    let (dropped, dropped_on) = std_mpsc::channel();
    db.add(Box::new(OutputWitness {
        log: Arc::clone(&log),
        clocked: false,
        dropped,
    }))
    .unwrap();
    (db, log, dropped_on)
}

fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

/// The Audit routes refused, after the settings and before the outputs or
/// the link start: the database comes back with no output driven and the
/// link never started, and the next start serves it.
#[tokio::test(start_paused = true)]
async fn a_start_refused_by_its_audit_routes_hands_the_database_back_unstarted() {
    let (db, log, _) = witnessed(database());
    let objects: Vec<_> = db.object_list().collect();
    // The plain Device has no Audit recipient provisioned, which the route
    // checks refuse.
    let (error, db) = refused(
        BACnetServer::start(audited(), db, TestTransport::never_start()).await,
        &objects,
    );
    assert_eq!(
        error.remote(),
        Some(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::WRITE_ACCESS_DENIED,
        }),
        "{error}"
    );
    assert_eq!((log.started(), log.released()), (0, 0));
    let mut server = BACnetServer::start(config(), db, TestTransport::new())
        .await
        .unwrap();
    assert_eq!((log.started(), log.released()), (1, 0));
    serves(&server, &objects).await;
    server.stop().await.unwrap();
}

/// Each timed object starts its output on the server's clocks: they are
/// bound before the outputs start.
#[tokio::test(start_paused = true)]
async fn a_start_binds_its_clocks_before_it_starts_the_outputs() {
    let (db, log, _) = witnessed(database());
    let mut server = BACnetServer::start(config(), db, TestTransport::new())
        .await
        .unwrap();
    assert_eq!(log.started(), 1);
    assert!(log.clocked_at_start.load(Ordering::SeqCst));
    server.stop().await.unwrap();
}

/// Target Audit refused after the link started, and the start dropped while
/// it lets go of the link: the stop it began goes no further, the outputs
/// it drove are released, and the database goes with the start.
#[tokio::test(start_paused = true)]
async fn a_start_dropped_while_it_lets_go_of_its_link_releases_its_outputs() {
    let borrows = Arc::new(AtomicUsize::new(0));
    let (db, log, dropped_on) = witnessed(refusing_audit(&borrows));
    let stopping = Arc::new(tokio::sync::Notify::new());
    let link = TestTransport::builder()
        .on_stop({
            let stopping = Arc::clone(&stopping);
            move || {
                stopping.notify_one();
                std::future::pending::<Result<(), Error>>()
            }
        })
        .build();
    let handle = link.handle();
    let start = tokio::spawn(BACnetServer::start(audited(), db, link));
    stopping.notified().await;
    assert_eq!(borrows.load(Ordering::Acquire), 3, "refused at the third");
    assert_eq!(handle.stops(), 1, "the link is being let go of");
    assert_eq!((log.started(), log.released()), (1, 0));
    start.abort();
    assert!(start.await.is_err_and(|error| error.is_cancelled()));
    assert_eq!((log.started(), log.released()), (1, 1));
    assert!(dropped_on.try_recv().is_ok(), "the database went with it");
    assert_eq!(handle.stops(), 1);
}

/// A start dropped at its last wait, with every phase before it done, with
/// and without target Audit: the server drops first, then the outputs it
/// drove are released, the tasks it spawned end, and the database goes off
/// the runtime once the tasks have let go of it.
#[tokio::test]
async fn a_start_dropped_at_its_staging_pass_lets_its_tasks_and_database_go() {
    let cases = [
        (config(), database(), TestTransport::new()),
        (audited_config(), audited_database(), bip_link(47812, false)),
    ];
    for (config, db, link) in cases {
        let audited = config.audit_reporters.is_some();
        let (db, log, dropped_on) = witnessed(db);
        let handle = link.handle();
        let before = alive_tasks();
        let mut start = Box::pin(BACnetServer::start(config, db, link));
        // One poll runs every phase up to the Staging pass, whose task can
        // only run once this one yields.
        let polled = start.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(polled.is_pending(), "audited: {audited}");
        assert_eq!(handle.starts(), 1, "the link is up; audited: {audited}");
        let spawned = alive_tasks();
        assert!(spawned > before, "the server's tasks are spawned");
        assert_eq!((log.started(), log.released()), (1, 0));
        drop(start);
        // The database's guard was free: the outputs are released at once,
        // after the server dropped, whose drop spawned the task that lets its
        // database go.
        assert_eq!((log.started(), log.released()), (1, 1));
        assert!(
            log.alive_at_release.load(Ordering::SeqCst) > spawned,
            "the server dropped before the outputs; audited: {audited}"
        );
        let thread = tokio::task::spawn_blocking(move || {
            dropped_on.recv_timeout(std::time::Duration::from_secs(5))
        })
        .await
        .unwrap()
        .expect("the database was dropped");
        assert_ne!(thread, std::thread::current().id(), "off the runtime");
        tokio::time::timeout(Duration::from_secs(5), async {
            while alive_tasks() > before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("every task the start spawned ends");
    }
}
