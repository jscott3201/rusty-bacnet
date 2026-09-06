use super::deadline_test_support::{poll_io, until};
use super::tasks::Tasks;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Capture(Arc<AtomicUsize>);
impl Drop for Capture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn sealed_spawner_drops_unpolled_and_rejected_captures() {
    let tasks = Tasks::new();
    let spawner = tasks.spawner();
    let dropped = Arc::new(AtomicUsize::new(0));
    let admitted = Capture(dropped.clone());
    assert!(spawner.spawn(async move {
        let _admitted = admitted;
        std::future::pending::<()>().await;
    }));
    tasks.request_shutdown();
    let rejected = Capture(dropped.clone());
    assert!(!spawner.spawn(async move {
        drop(rejected);
    }));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    tasks.drain().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    assert_eq!(tasks.len(), 0);
    drop(tasks);
    let expired = Capture(dropped.clone());
    assert!(!spawner.spawn(async move {
        drop(expired);
    }));
    assert_eq!(dropped.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn spawn_racing_seal_never_strands_owned_captures() {
    let tasks = Tasks::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let runtime = tokio::runtime::Handle::current();
    let producer = std::thread::spawn({
        let spawner = tasks.spawner();
        let dropped = dropped.clone();
        let barrier = barrier.clone();
        move || {
            let _runtime = runtime.enter();
            barrier.wait();
            for _ in 0..1000 {
                let capture = Capture(dropped.clone());
                spawner.spawn(async move {
                    let _capture = capture;
                    std::future::pending::<()>().await;
                });
            }
        }
    });
    barrier.wait();
    tasks.request_shutdown();
    producer.join().unwrap();
    tasks.drain().await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1000);
    assert_eq!(tasks.len(), 0);
}

#[tokio::test]
async fn empty_reaper_wakes_on_spawn_and_releases_completed_entries() {
    let tasks = Tasks::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let (ready, started) = tokio::sync::oneshot::channel();
    let reaper = tokio::spawn({
        let tasks = tasks.clone();
        async move {
            let mut reaper = Box::pin(tasks.reap());
            assert!(futures_util::poll!(&mut reaper).is_pending());
            ready.send(()).unwrap();
            reaper.await;
        }
    });
    started.await.unwrap();
    let capture = Capture(dropped.clone());
    assert!(tasks.spawner().spawn(async move {
        drop(capture);
    }));
    poll_io(reaper).await.unwrap();
    until(|| dropped.load(Ordering::SeqCst) == 1).await;
    assert_eq!(tasks.len(), 0);
}

#[tokio::test]
async fn unexpected_supervisor_exit_cancels_workers_with_owner_still_alive() {
    let tasks = Tasks::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let supervisor = tokio::spawn({
        let tasks = tasks.clone();
        let dropped = dropped.clone();
        async move {
            let _abort_on_exit = tasks.abort_on_exit();
            let capture = Capture(dropped);
            tasks.spawner().spawn(async move {
                let _capture = capture;
                std::future::pending::<()>().await;
            });
            std::future::pending::<()>().await;
        }
    });
    until(|| tasks.len() == 1).await;
    supervisor.abort();
    assert!(supervisor.await.unwrap_err().is_cancelled());
    until(|| dropped.load(Ordering::SeqCst) == 1).await;
    tasks.drain().await;
    assert_eq!(tasks.len(), 0);
}
