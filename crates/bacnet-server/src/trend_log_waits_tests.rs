//! `poll_trend_logs` never waits for the database in its caller's task
//! (#2781). A poll dropped before its task holds the guard logs nothing;
//! one dropped after keeps what it logged. The objects are [`super`]'s:
//! Trend Log 1 logs AV-1 every 60 seconds, its first record due at once.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use super::*;

/// A poll as its caller holds it.
type Call<'a> = Pin<Box<dyn Future<Output = Duration> + Send + 'a>>;

/// Poll `call` once with a waker that wakes nothing: its caller then stops
/// polling it.
fn poll_once(call: &mut Call<'_>) -> Poll<Duration> {
    call.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

fn clock() -> Arc<MutableClock> {
    Arc::new(MutableClock(Mutex::new(Some(frame()))))
}

#[tokio::test(start_paused = true)]
async fn a_poll_dropped_while_its_task_waits_logs_nothing() {
    let (db, oid) = database(clock());
    let before = snapshot(&*db.read().await, oid);
    let held = Arc::clone(&db).write_owned().await;
    let mut call: Call<'_> = Box::pin(poll_trend_logs(&db));
    assert!(poll_once(&mut call).is_pending());
    drop(call);
    // The task sees its caller gone and ends without the guard.
    tokio::task::yield_now().await;
    drop(held);
    tokio::task::yield_now().await;
    assert_eq!(snapshot(&*db.read().await, oid), before);
}

#[tokio::test(start_paused = true)]
async fn a_poll_dropped_once_its_task_polled_keeps_its_record() {
    let (db, oid) = database(clock());
    let held = Arc::clone(&db).write_owned().await;
    let mut call: Call<'_> = Box::pin(poll_trend_logs(&db));
    assert!(poll_once(&mut call).is_pending());
    drop(held);
    // The task takes the guard and polls, with the caller there.
    tokio::task::yield_now().await;
    drop(call);
    let total = db
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(total, PropertyValue::Unsigned(1));
}

/// A parked poll, its task polling once the guard is free, returns the
/// wait an awaited poll at that moment returns.
#[tokio::test(start_paused = true)]
async fn a_parked_poll_answers_as_an_awaited_one() {
    let (db, oid) = database(clock());
    poll_trend_logs(&db).await;
    let awaited = poll_trend_logs(&db).await;
    let held = Arc::clone(&db).write_owned().await;
    let mut call: Call<'_> = Box::pin(poll_trend_logs(&db));
    assert!(poll_once(&mut call).is_pending());
    drop(held);
    assert_eq!(call.await, awaited);
    let total = db
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(
        total,
        PropertyValue::Unsigned(1),
        "one record, from the first"
    );
}
