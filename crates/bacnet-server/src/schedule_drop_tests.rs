//! A dropped `tick_schedules` ends the runs it started in the drop, as it
//! did when the runs lived in its caller's future (#2781): the next tick,
//! made at once, finds no Command in process and starts the list it asks
//! for. A task of the runtime's making the runs when the caller goes ends
//! them once its poll returns, writing nothing more.

use bacnet_objects::command::CommandObject;
use bacnet_types::constructed::BACnetActionList;

use super::*;

/// [`database`] with CMD-1's lists replaced by `lists`.
async fn with_lists(lists: Vec<Vec<BACnetActionCommand>>) -> Arc<RwLock<ObjectDatabase>> {
    let db = database();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(
            lists
                .into_iter()
                .map(|commands| BACnetActionList { commands })
                .collect(),
        )
        .unwrap();
    let mut db_w = db.write().await;
    db_w.remove(&cmd(1)).unwrap();
    db_w.add(Box::new(command)).unwrap();
    drop(db_w);
    db
}

/// CMD-1's list 1 as [`database`] has it: AO-1 50.0, a 5-second delay,
/// then CMD-2 started.
fn list_one() -> Vec<BACnetActionCommand> {
    let delayed = BACnetActionCommand {
        post_delay: Some(5),
        ..write(ao(1), PropertyValue::Real(50.0))
    };
    let start_cmd2 = BACnetActionCommand {
        priority: None,
        ..write(cmd(2), PropertyValue::Unsigned(1))
    };
    vec![delayed, start_cmd2]
}

/// Tick A starts list 1 and is dropped in its delay; tick B, made at once
/// on the same thread, asks for list 2. A's run is over by then, so B's
/// write to CMD-1 is taken and list 2 runs.
#[tokio::test(start_paused = true)]
async fn a_tick_made_at_once_after_a_dropped_one_starts_its_list() {
    let list_two = vec![write(ao(1), PropertyValue::Real(60.0))];
    let db = with_lists(vec![list_one(), list_two]).await;
    tokio::time::timeout(Duration::from_secs(1), tick_schedules(&db))
        .await
        .unwrap_err();
    db.write()
        .await
        .get_mut(&oid(ObjectType::SCHEDULE, 1))
        .unwrap()
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    tick_schedules(&db).await;
    assert_eq!(
        read(&db, cmd(1), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(2)
    );
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(60.0));
    assert!(!state(&db, cmd(1)).await.0, "CMD-1 left in process");
    assert_eq!(
        read(&db, cmd(2), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(0),
        "list 1 never got to start CMD-2"
    );
}

/// On threads: a tick dropped while its run waits out an hour's delay ends
/// the run, in the drop or once a poll of the task's in progress returns,
/// and the run writes nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_dropped_mid_run_on_threads_ends_it_and_writes_no_more() {
    let delayed = BACnetActionCommand {
        post_delay: Some(3600),
        ..write(ao(1), PropertyValue::Real(50.0))
    };
    let db = with_lists(vec![vec![delayed, write(ao(2), PropertyValue::Real(30.0))]]).await;
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    assert!(poll_once(&mut tick).is_pending());
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    drop(tick);
    tokio::time::timeout(STUCK, async {
        while state(&db, cmd(1)).await.0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the run ended");
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Null);
}

/// CMD-1 in process with a run of AO-1 50.0 then AO-2 30.0, no delays,
/// taken as a tick's pass takes it.
async fn taken_run(db: &Arc<RwLock<ObjectDatabase>>) -> crate::command_lists::TakenRuns {
    let mut db_w = db.write().await;
    db_w.get_mut(&cmd(1))
        .unwrap()
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    crate::command_lists::TakenRuns::take(db, &mut db_w, &[cmd(1)])
}

/// The caller goes while a poll of the task's is in progress on another
/// thread, which the test stands for by holding the slot as the task's
/// poll does: the drop leaves the runs to the task, the poll writes
/// nothing once the caller has gone, and the runs end as the task lets go.
#[tokio::test(start_paused = true)]
async fn runs_a_poll_holds_as_the_caller_goes_end_with_the_task() {
    use crate::caller_waits::HeldDatabase;
    use crate::command_lists::SharedRuns;
    let db = with_lists(vec![vec![
        write(ao(1), PropertyValue::Real(50.0)),
        write(ao(2), PropertyValue::Real(30.0)),
    ]])
    .await;
    let runs = SharedRuns::new(HeldDatabase::new(Arc::clone(&db)), taken_run(&db).await);
    let (driver, slot) = (runs.driver(), runs.slot());
    // Read without a wait, as the slot is held meanwhile.
    let now = |object, property, index| {
        let db = db.try_read().unwrap();
        db.get(&object)
            .unwrap()
            .read_property(property, index)
            .unwrap()
    };
    let in_process = || now(cmd(1), PropertyIdentifier::IN_PROCESS, None);
    {
        let mut polling = slot.lock().unwrap();
        drop(runs);
        assert_eq!(
            in_process(),
            PropertyValue::Boolean(true),
            "left to the poll"
        );
        // The poll in progress reaches the first write after the caller went.
        let mut cx = Context::from_waker(Waker::noop());
        let future = polling.as_mut().expect("still in the slot");
        assert!(future.as_mut().poll(&mut cx).is_pending());
        let ao1 = now(ao(1), PropertyIdentifier::PRIORITY_ARRAY, Some(8));
        assert_eq!(ao1, PropertyValue::Null, "no write");
    }
    // The task sees its caller gone and lets go of the slot.
    drop((driver, slot));
    assert_eq!(in_process(), PropertyValue::Boolean(false), "the run ended");
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Null);
}

/// Taken runs dropped as the last handle on the database let it go off the
/// runtime (#1513), as when a tick's task whose caller has gone drops the
/// runs its pass took: on this current-thread runtime, not on the test's
/// own thread.
#[tokio::test]
async fn taken_runs_holding_the_last_handle_let_it_go_off_the_runtime() {
    let (dropped, dropped_on) = std::sync::mpsc::channel();
    let mut objects = ObjectDatabase::new();
    objects.set_clock_reader(Some(Arc::new(DroppedOn(dropped))));
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![write(ao(1), PropertyValue::Real(50.0))],
        }])
        .unwrap();
    objects.add(Box::new(command)).unwrap();
    let db = Arc::new(RwLock::new(objects));
    let runs = taken_run(&db).await;
    assert!(!runs.is_empty());
    drop(db);
    drop(runs);
    let thread = tokio::task::spawn_blocking(move || dropped_on.recv_timeout(STUCK))
        .await
        .unwrap()
        .expect("the database was dropped");
    assert_ne!(thread, std::thread::current().id());
}

/// The real release: the call goes while the slot is held, as by a poll
/// of the task's on another thread, and leaves the run to the task, which,
/// polled again as its caller went, drops its driver and the run with it.
#[tokio::test(start_paused = true)]
async fn the_task_ends_a_run_its_caller_left_to_it() {
    use crate::caller_waits::{HeldDatabase, for_caller, on_runtime};
    use crate::command_lists::SharedRuns;
    let db = database();
    let runs = SharedRuns::new(HeldDatabase::new(Arc::clone(&db)), taken_run(&db).await);
    let slot = runs.slot();
    let mut call: Tick<'_> = Box::pin(async move {
        let _ = for_caller(on_runtime, runs.driver(), |()| ()).await;
        drop(runs);
    });
    // AO-1, then the 5-second delay, in the task once it has polled.
    assert!(poll_once(&mut call).is_pending());
    tokio::task::yield_now().await;
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    {
        let _polling = slot.lock().unwrap();
        drop(call);
    }
    assert!(state(&db, cmd(1)).await.0, "left to the task");
    tokio::task::yield_now().await;
    assert!(!state(&db, cmd(1)).await.0, "the task ended the run");
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Null);
}

/// A tick polled on one runtime, its run queued for the database in that
/// runtime's task, and the runtime dropped while the tick is kept unpolled:
/// the task's driver drops the run with the task, so the database, let go
/// of later, goes to a writer on another runtime, not to a waiter nothing
/// polls.
#[test]
fn a_ticks_task_dropped_with_its_runtime_leaves_no_waiter() {
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
    };
    let (first, second) = (runtime(), runtime());
    let db = database();
    let mut tick: Tick<'_> = Box::pin(tick_schedules(&db));
    let held = first.block_on(async {
        // The pass, AO-1, then the 5-second delay, in a task of this runtime.
        assert!(poll_once(&mut tick).is_pending());
        let held = Arc::clone(&db).write_owned().await;
        // The delay runs out: the run's next step queues behind the guard.
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        held
    });
    drop(first);
    drop(held);
    second.block_on(async {
        let guard = tokio::time::timeout(STUCK, db.write())
            .await
            .expect("the database went to a waiter nothing polls");
        drop(guard);
    });
    drop(tick);
}
