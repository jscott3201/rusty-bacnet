//! `tick_schedules` never waits for the database in its caller's task
//! (#2781): a tick polled once and then left unpolled holds up no other
//! user of the database, through its pass and through the runs that pass
//! starts. A tick dropped before its pass makes none of it; one dropped
//! after keeps the whole pass. Ticks awaited in turn run in turn.
//!
//! The objects are [`super`]'s: SCH-1 drives CMD-1, whose list writes AO-1,
//! waits 5 seconds and starts CMD-2.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use super::*;

/// A tick as its caller holds it.
type Tick<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Long enough that only a waiter stuck behind a parked tick reaches it;
/// the paused clock jumps there at once when nothing else can run.
const STUCK: Duration = Duration::from_secs(30);

/// Poll `tick` once with a waker that wakes nothing: its caller then stops
/// polling it.
fn poll_once(tick: &mut Tick<'_>) -> Poll<()> {
    tick.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

/// A writer that queues for the database now, as the server's requests
/// and the application's other tasks do, and lets go at once.
async fn queue_writer(db: &Arc<RwLock<ObjectDatabase>>) -> tokio::task::JoinHandle<()> {
    let writer = tokio::spawn({
        let db = Arc::clone(db);
        async move { drop(db.write().await) }
    });
    // Let it take its place in the queue.
    tokio::task::yield_now().await;
    writer
}

async fn got_through(writer: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(STUCK, writer)
        .await
        .expect("the writer queued behind the parked tick never got the database")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_tick_holds_up_no_writer_at_its_pass() {
    let db = database();
    let held = Arc::clone(&db).write_owned().await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    let writer = queue_writer(&db).await;
    drop(held);
    got_through(writer).await;
    tick.await;
    // What an awaited tick leaves (`tick_schedules_runs_the_command_lists_its_writes_start`).
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(state(&db, cmd(1)).await, (false, true, vec![true, true]));
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Real(30.0));
}

#[tokio::test(start_paused = true)]
async fn a_parked_tick_holds_up_no_writer_while_its_runs_wait() {
    let db = database();
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    // The pass and CMD-1's first write, then its 5-second post delay.
    assert!(poll_once(&mut tick).is_pending());
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    let held = Arc::clone(&db).write_owned().await;
    // The delay runs out while the application holds the database, and the
    // caller polls the tick once more, as a `select!` loop would: CMD-1's
    // next step queues for the database.
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(poll_once(&mut tick).is_pending());
    let writer = queue_writer(&db).await;
    drop(held);
    got_through(writer).await;
    tick.await;
    assert_eq!(state(&db, cmd(1)).await, (false, true, vec![true, true]));
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Real(30.0));
}

/// The tasks alive on this runtime, the test's own included.
fn alive() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

/// SCH-2, which writes 7.0 to AO-2 at priority 16, beside SCH-1: a pass
/// makes two writes.
async fn add_second_schedule(db: &RwLock<ObjectDatabase>) {
    let mut schedule = ScheduleObject::new(2, "SCH-2", PropertyValue::Real(7.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            ao(2),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    db.write().await.add(Box::new(schedule)).unwrap();
}

/// What a pass writes: CMD-1's Present_Value and AO-2's slot 16.
async fn passed(db: &RwLock<ObjectDatabase>) -> (PropertyValue, PropertyValue) {
    (
        read(db, cmd(1), PropertyIdentifier::PRESENT_VALUE, None).await,
        read(db, ao(2), PropertyIdentifier::PRIORITY_ARRAY, Some(16)).await,
    )
}

#[tokio::test(start_paused = true)]
async fn a_tick_dropped_while_its_task_waits_makes_no_pass() {
    let db = database();
    add_second_schedule(&db).await;
    let before = passed(&db).await;
    let tasks = alive();
    let held = Arc::clone(&db).write_owned().await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    assert_eq!(alive(), tasks + 1, "one task waits for the pass");
    drop(tick);
    // The task sees its caller gone and ends without the guard.
    tokio::task::yield_now().await;
    assert_eq!(alive(), tasks, "the task ended");
    drop(held);
    tokio::task::yield_now().await;
    assert_eq!(passed(&db).await, before);
    assert!(!state(&db, cmd(1)).await.0, "no run started");
}

#[tokio::test(start_paused = true)]
async fn a_tick_dropped_once_its_pass_is_made_keeps_all_of_it() {
    let db = database();
    add_second_schedule(&db).await;
    let held = Arc::clone(&db).write_owned().await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    drop(held);
    // The task takes the guard and makes the pass, with the caller there.
    tokio::task::yield_now().await;
    drop(tick);
    assert_eq!(
        passed(&db).await,
        (PropertyValue::Unsigned(1), PropertyValue::Real(7.0))
    );
    // CMD-1's run, started by the pass and never handed on, ended with the
    // caller's answer: none of its writes made.
    assert_eq!(state(&db, cmd(1)).await, (false, false, vec![false, false]));
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Null);
}

/// Ticks awaited one after the other apply one after the other, each
/// through the runtime's task: each leaves AO-2 at the default SCH-2 had
/// when it ran, and the first returns only once the run its pass started
/// has ended.
#[tokio::test(start_paused = true)]
async fn ticks_awaited_in_turn_apply_in_turn() {
    let db = database();
    add_second_schedule(&db).await;
    for value in [7.0, 8.0] {
        let mut held = Arc::clone(&db).write_owned().await;
        held.get_mut(&oid(ObjectType::SCHEDULE, 2))
            .unwrap()
            .write_property(
                PropertyIdentifier::SCHEDULE_DEFAULT,
                None,
                PropertyValue::Real(value),
                None,
            )
            .unwrap();
        let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
        assert!(poll_once(&mut tick).is_pending());
        drop(held);
        tick.await;
        assert_eq!(
            read(&db, ao(2), PropertyIdentifier::PRIORITY_ARRAY, Some(16)).await,
            PropertyValue::Real(value)
        );
        assert_eq!(state(&db, cmd(1)).await, (false, true, vec![true, true]));
    }
}

/// The parked pass on a multi-thread runtime: the writer queues after the
/// tick took its place, and the tick's task runs on a thread of its own, so
/// the writer gets the database however the threads interleave.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_parked_tick_holds_up_no_writer_on_threads() {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(SettableClock::at(2026, 10, 3, 12, 0)));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(2, "AO-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let db = Arc::new(RwLock::new(db));
    add_second_schedule(&db).await;
    let held = Arc::clone(&db).write_owned().await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    let writer = tokio::spawn({
        let db = Arc::clone(&db);
        async move { drop(db.write().await) }
    });
    drop(held);
    tokio::time::timeout(STUCK, writer)
        .await
        .expect("the writer queued behind the parked tick never got the database")
        .unwrap();
    tick.await;
    assert_eq!(
        read(&db, ao(2), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Real(7.0)
    );
}

/// A clock that says where it is dropped: with the database, its last
/// holder.
struct DroppedOn(std::sync::mpsc::Sender<std::thread::ThreadId>);

impl bacnet_objects::clock::ClockReader for DroppedOn {
    fn read_clock(&self) -> Option<bacnet_objects::clock::ClockFrame> {
        None
    }
}

impl Drop for DroppedOn {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

/// A tick's task whose caller and application have both let go holds the
/// last handle on the database, and lets it go off the runtime (#1513): on
/// this current-thread runtime, not on the test's own thread.
#[tokio::test]
async fn the_last_handle_a_ticks_task_holds_goes_off_the_runtime() {
    let (dropped, dropped_on) = std::sync::mpsc::channel();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(DroppedOn(dropped))));
    let db = Arc::new(RwLock::new(db));
    let held = Arc::clone(&db).write_owned().await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    drop(tick);
    drop(db);
    drop(held);
    let thread = tokio::task::spawn_blocking(move || dropped_on.recv_timeout(STUCK))
        .await
        .unwrap()
        .expect("the database was dropped");
    assert_ne!(thread, std::thread::current().id());
}

#[path = "schedule_drop_tests.rs"]
mod drops;
