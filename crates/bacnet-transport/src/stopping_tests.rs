use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use super::{StopGate, Tasks};

/// A stop that runs while another holds the gate waits for it, and only
/// one of them runs the stop.
#[tokio::test(start_paused = true)]
async fn a_second_stop_waits_for_the_first_and_runs_nothing() {
    let gate = Arc::new(StopGate::default());
    let runs = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());
    let stop = |gate: Arc<StopGate>, runs: Arc<AtomicUsize>, release: Arc<Notify>| async move {
        if let Some(stopping) = gate.enter().await {
            runs.fetch_add(1, Ordering::SeqCst);
            release.notified().await;
            stopping.finish();
        }
    };
    let first = tokio::spawn(stop(gate.clone(), runs.clone(), release.clone()));
    tokio::time::sleep(Duration::from_millis(1)).await;
    let second = tokio::spawn(stop(gate.clone(), runs.clone(), release.clone()));
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(!second.is_finished(), "the second stop waits for the first");
    release.notify_one();
    first.await.unwrap();
    second.await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(gate.is_stopped());
    assert!(
        gate.enter().await.is_none(),
        "a stop after one finished runs nothing"
    );
}

/// A cancelled stop opens the gate again: the stop waiting for it runs.
#[tokio::test(start_paused = true)]
async fn a_cancelled_stop_hands_the_stop_to_the_next() {
    let gate = Arc::new(StopGate::default());
    let first = {
        let gate = gate.clone();
        tokio::spawn(async move {
            let _stopping = gate.enter().await;
            std::future::pending::<()>().await;
        })
    };
    tokio::time::sleep(Duration::from_millis(1)).await;
    let second = {
        let gate = gate.clone();
        tokio::spawn(async move { gate.enter().await.map(super::Stopping::finish).is_some() })
    };
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(!second.is_finished());
    first.abort();
    assert!(
        second.await.unwrap(),
        "the waiting stop takes the stop over"
    );
    assert!(gate.is_stopped());
}

/// A start opens the gate for the next stop.
#[tokio::test]
async fn open_lets_the_next_stop_run() {
    let mut gate = StopGate::default();
    gate.enter().await.unwrap().finish();
    assert!(gate.enter().await.is_none());
    gate.open();
    assert!(!gate.is_stopped());
    assert!(gate.enter().await.is_some());
}

/// A join cancelled part way keeps the task it waited for, and the next
/// stop joins it.
#[tokio::test(start_paused = true)]
async fn a_cancelled_join_leaves_its_task_for_the_next_stop() {
    let mut tasks = Tasks::default();
    // A task that ends only when its drop is noticed: abort ends it at its
    // next poll, which a paused runtime gives it here.
    tasks.push(tokio::spawn(std::future::pending()));
    let tasks = Arc::new(tasks);
    // Cancel the first stop before it can see the aborted task finish.
    {
        let stop = tasks.stop();
        tokio::pin!(stop);
        let polled = futures_poll_once(stop.as_mut()).await;
        assert!(polled.is_pending());
    }
    assert!(tasks.is_running(), "the cancelled join put its task back");
    tasks.stop().await;
    assert!(!tasks.is_running());
}

/// Poll `future` once.
async fn futures_poll_once<F: Future + Unpin>(mut future: F) -> std::task::Poll<F::Output> {
    std::future::poll_fn(|cx| std::task::Poll::Ready(std::pin::Pin::new(&mut future).poll(cx)))
        .await
}
