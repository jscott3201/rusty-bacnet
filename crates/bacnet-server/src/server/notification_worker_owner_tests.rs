use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Barrier, Semaphore};

struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn reserve(owner: &NotificationTransactions) -> NotificationOperation {
    owner
        .reserve(
            canonical_direct_peer(&[1]),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        )
        .unwrap()
        .0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notification_worker_spawn_close_race_releases_all_resources() {
    let owner = NotificationTransactions::new();
    let count = Arc::new(AtomicUsize::new(0));
    let permits = Arc::new(Semaphore::new(64));
    let barrier = Arc::new(Barrier::new(65));
    let mut producers = JoinSet::new();
    for _ in 0..64 {
        let operation = reserve(&owner);
        let permit = permits.clone().try_acquire_owned().unwrap();
        let guard = CountDrop(count.clone());
        let owner = owner.clone();
        let barrier = barrier.clone();
        producers.spawn(async move {
            barrier.wait().await;
            owner.spawn(async move {
                let _resources = (operation, permit, guard);
                std::future::pending::<()>().await;
            });
        });
    }
    barrier.wait().await;
    owner.close();
    while let Some(result) = producers.join_next().await {
        result.unwrap();
    }
    while let Some(result) = owner.join_next().await {
        assert!(result.unwrap_err().is_cancelled());
    }
    assert!(owner.workers_empty());
    assert_eq!(owner.active_count(), 0);
    assert_eq!(permits.available_permits(), 64);
    assert_eq!(count.load(Ordering::SeqCst), 64);
    assert!(matches!(
        owner.reserve(
            canonical_direct_peer(&[1]),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
        ),
        Err(NotificationReserveError::Closed)
    ));
}

#[tokio::test]
async fn notification_worker_post_close_rejects_outside_owner_lock() {
    struct ReentrantDrop(std::sync::Weak<NotificationTransactions>);
    impl Drop for ReentrantDrop {
        fn drop(&mut self) {
            assert!(self.0.upgrade().unwrap().workers_empty());
        }
    }
    let owner = NotificationTransactions::new();
    let operation = reserve(&owner);
    let reentrant = ReentrantDrop(Arc::downgrade(&owner));
    let count = Arc::new(AtomicUsize::new(0));
    let guard = CountDrop(count.clone());
    owner.close();
    owner.spawn(async move {
        let _resources = (operation, reentrant, guard);
        panic!("a post-close future must never be polled");
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(owner.workers_empty());
    assert_eq!(owner.active_count(), 0);
}

#[tokio::test]
async fn notification_worker_empty_wait_wakes_and_cancelled_join_retains_task() {
    let owner = NotificationTransactions::new();
    // Register an empty-set consumer before an independent producer spawns.
    let (polled, ready) = oneshot::channel();
    let consumer_owner = owner.clone();
    let consumer = tokio::spawn(async move {
        let mut join = std::pin::pin!(consumer_owner.join_next());
        poll_fn(|cx| {
            assert!(join.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        polled.send(()).unwrap();
        join.await.unwrap().unwrap();
    });
    ready.await.unwrap();
    owner.spawn(async {});
    tokio::time::timeout(Duration::from_secs(2), consumer)
        .await
        .unwrap()
        .unwrap();
    assert!(owner.workers_empty());

    let (release, held) = oneshot::channel::<()>();
    owner.spawn(async move {
        let _ = held.await;
    });
    {
        let mut join = std::pin::pin!(owner.join_next());
        poll_fn(|cx| {
            assert!(join.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert!(!owner.workers_empty());
    release.send(()).unwrap();
    owner.join_next().await.unwrap().unwrap();
    assert!(owner.workers_empty());
}

#[tokio::test]
async fn notification_worker_operation_does_not_retain_owner() {
    let owner = NotificationTransactions::new();
    let weak = Arc::downgrade(&owner);
    let operation = reserve(&owner);
    let core = Arc::downgrade(&operation.transactions);
    let (dropped, observed) = oneshot::channel();
    struct NotifyDrop(Option<oneshot::Sender<()>>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let guard = NotifyDrop(Some(dropped));
    owner.spawn(async move {
        let _resources = (operation, guard);
        std::future::pending::<()>().await;
    });
    drop(owner);
    assert!(
        weak.upgrade().is_none(),
        "worker retained its owning JoinSet"
    );
    observed.await.unwrap();
    assert!(core.upgrade().is_none());
}
