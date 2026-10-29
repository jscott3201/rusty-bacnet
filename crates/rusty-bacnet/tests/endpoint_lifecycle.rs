//! Executes the exact private owner without linking or initializing Python.
#[path = "../src/endpoint/lifecycle.rs"]
mod lifecycle;

use lifecycle::{Lifecycle, LifecycleError, Session};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, oneshot};

struct Resource {
    live: Arc<AtomicUsize>,
    start_entered: Option<oneshot::Sender<()>>,
    start_release: Option<oneshot::Receiver<()>>,
    stop_entered: Option<oneshot::Sender<()>>,
    stop_release: Option<oneshot::Receiver<()>>,
}

impl Resource {
    fn new(live: Arc<AtomicUsize>) -> Self {
        live.fetch_add(1, Ordering::SeqCst);
        Self {
            live,
            start_entered: None,
            start_release: None,
            stop_entered: None,
            stop_release: None,
        }
    }
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Session for Resource {
    type Error = &'static str;
    async fn start(&mut self) -> Result<(), Self::Error> {
        if let Some(entered) = self.start_entered.take() {
            entered.send(()).unwrap();
        }
        if let Some(release) = self.start_release.take() {
            release.await.unwrap();
        }
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), Self::Error> {
        if let Some(entered) = self.stop_entered.take() {
            entered.send(()).unwrap();
        }
        if let Some(release) = self.stop_release.take() {
            release.await.unwrap();
        }
        Ok(())
    }
}

type Owner = Lifecycle<Resource, usize>;

#[tokio::test]
async fn close_waits_for_admitted_start_then_drops_resource() {
    let owner = Owner::new();
    let live = Arc::new(AtomicUsize::new(0));
    let (entered, admitted) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let starting = tokio::spawn({
        let owner = owner.clone();
        let live = live.clone();
        async move {
            owner
                .start(false, move |_| async move {
                    let mut resource = Resource::new(live);
                    resource.start_entered = Some(entered);
                    resource.start_release = Some(held);
                    Ok(resource)
                })
                .await
        }
    });
    admitted.await.unwrap();
    assert!(owner.push(7).is_err());
    let closing = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!closing.is_finished());
    release.send(()).unwrap();
    starting.await.unwrap().unwrap();
    closing.await.unwrap().unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert!(owner.session.lock().await.is_none());
    owner.push(8).unwrap();
}

#[tokio::test]
async fn cancelled_start_settles_then_cleans_before_restoring_exact_registrations() {
    let owner = Owner::new();
    owner.push(11).unwrap();
    owner.push(22).unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let (entered, admitted) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let starting = tokio::spawn({
        let owner = owner.clone();
        let live = live.clone();
        async move {
            owner
                .start(false, move |objects| async move {
                    assert_eq!(objects, vec![11, 22]);
                    let mut resource = Resource::new(live);
                    resource.start_entered = Some(entered);
                    resource.start_release = Some(held);
                    Ok(resource)
                })
                .await
        }
    });
    admitted.await.unwrap();
    starting.abort();
    assert!(starting.await.unwrap_err().is_cancelled());
    assert!(owner.push(33).is_err());
    assert_eq!(owner.pending_count(), 0);
    release.send(()).unwrap();
    owner.close().await.unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(owner.pending_count(), 2);
    owner
        .start(false, move |objects| async move {
            assert_eq!(objects, vec![11, 22]);
            Ok(Resource::new(live))
        })
        .await
        .unwrap();
    assert_eq!(owner.pending_count(), 0);
    owner.close().await.unwrap();
    assert_eq!(owner.pending_count(), 0);
}

#[tokio::test]
async fn competing_start_and_context_entry_construct_only_one_resource() {
    // Each wrapper passes its transport-specific factory through these modes.
    for modes in [[false, false], [false, true], [true, false], [true, true]] {
        let owner = Owner::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(Notify::new());
        let (entered, admitted) = oneshot::channel();
        let first = tokio::spawn({
            let owner = owner.clone();
            let live = live.clone();
            let calls = calls.clone();
            let held = held.clone();
            async move {
                owner
                    .start(modes[0], move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let resource = Resource::new(live);
                        entered.send(()).unwrap();
                        held.notified().await;
                        Ok(resource)
                    })
                    .await
            }
        });
        admitted.await.unwrap();
        let second = tokio::spawn({
            let owner = owner.clone();
            let live = live.clone();
            let calls = calls.clone();
            async move {
                owner
                    .start(modes[1], move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(Resource::new(live))
                    })
                    .await
            }
        });
        held.notify_one();
        first.await.unwrap().unwrap();
        let result = second.await.unwrap();
        if modes[1] {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(LifecycleError::AlreadyStarted)));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(live.load(Ordering::SeqCst), 1);
        owner.close().await.unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn cancelled_close_keeps_cleanup_owned_and_next_close_joins() {
    let owner = Owner::new();
    let live = Arc::new(AtomicUsize::new(0));
    let (entered, stopping) = oneshot::channel();
    let (release, held) = oneshot::channel();
    owner
        .start(false, {
            let live = live.clone();
            move |_| async move {
                let mut resource = Resource::new(live);
                resource.stop_entered = Some(entered);
                resource.stop_release = Some(held);
                Ok(resource)
            }
        })
        .await
        .unwrap();
    let close = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    stopping.await.unwrap();
    close.abort();
    assert!(close.await.unwrap_err().is_cancelled());
    assert_eq!(live.load(Ordering::SeqCst), 1);
    assert!(owner.push(1).is_err());
    let next = tokio::spawn({
        let owner = owner.clone();
        async move { owner.close().await }
    });
    tokio::task::yield_now().await;
    assert!(!next.is_finished());
    release.send(()).unwrap();
    next.await.unwrap().unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    owner.push(1).unwrap();
}

#[tokio::test]
async fn failed_start_restores_pending_and_permits_retry() {
    let owner = Owner::new();
    owner.push(9).unwrap();
    let error = owner
        .start(false, |objects| async move {
            assert_eq!(objects, vec![9]);
            Err("setup failed")
        })
        .await;
    assert!(matches!(
        error,
        Err(LifecycleError::Operation("setup failed"))
    ));
    assert_eq!(owner.pending_count(), 1);
    owner.push(10).unwrap();
    owner
        .start(false, |objects| async move {
            assert_eq!(objects, vec![9, 10]);
            Ok(Resource::new(Arc::new(AtomicUsize::new(0))))
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn close_cancels_unbounded_preparation_and_restores_after_drop() {
    let owner = Owner::new();
    owner.push(4).unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let (entered, admitted) = oneshot::channel();
    let start = tokio::spawn({
        let owner = owner.clone();
        let live = live.clone();
        async move {
            owner
                .start(false, move |_| async move {
                    let resource = Resource::new(live);
                    entered.send(()).unwrap();
                    std::future::pending::<()>().await;
                    Ok(resource)
                })
                .await
        }
    });
    admitted.await.unwrap();
    assert!(owner.push(5).is_err());
    owner.close().await.unwrap();
    assert!(matches!(
        start.await.unwrap(),
        Err(LifecycleError::StartupCancelled)
    ));
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(owner.pending_count(), 1);
    owner
        .start(false, move |objects| async move {
            assert_eq!(objects, vec![4]);
            Ok(Resource::new(live))
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_waiter_drops_unbounded_preparation_before_close_returns() {
    let owner = Owner::new();
    owner.push(4).unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let (entered, admitted) = oneshot::channel();
    let start = tokio::spawn({
        let owner = owner.clone();
        let live = live.clone();
        async move {
            owner
                .start(false, move |_| async move {
                    let resource = Resource::new(live);
                    entered.send(()).unwrap();
                    std::future::pending::<()>().await;
                    Ok(resource)
                })
                .await
        }
    });
    admitted.await.unwrap();
    start.abort();
    assert!(start.await.unwrap_err().is_cancelled());
    owner.close().await.unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(owner.pending_count(), 1);
}

#[tokio::test]
async fn cancellation_while_waiting_for_admission_never_acquires_resource() {
    let owner = Owner::new();
    let guard = owner.session.lock().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let start = tokio::spawn({
        let owner = owner.clone();
        let calls = calls.clone();
        async move {
            owner
                .start(false, move |_| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Resource::new(Arc::new(AtomicUsize::new(0))))
                })
                .await
        }
    });
    tokio::task::yield_now().await;
    start.abort();
    assert!(start.await.unwrap_err().is_cancelled());
    drop(guard);
    owner.close().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn session_start_error_releases_resources_before_registration_retry() {
    struct FailedSession(Resource);
    impl Session for FailedSession {
        type Error = &'static str;
        async fn start(&mut self) -> Result<(), Self::Error> {
            assert_eq!(self.0.live.load(Ordering::SeqCst), 1);
            Err("session failed")
        }
        async fn stop(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let owner = Lifecycle::<FailedSession, usize>::new();
    let live = Arc::new(AtomicUsize::new(0));
    owner.push(3).unwrap();
    let result = owner
        .start(false, {
            let live = live.clone();
            move |_| async move { Ok(FailedSession(Resource::new(live))) }
        })
        .await;
    assert!(matches!(
        result,
        Err(LifecycleError::Operation("session failed"))
    ));
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(owner.pending_count(), 1);
    owner.push(4).unwrap();
    owner.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_close_before_admission_cannot_cancel_a_later_start() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let owner = Owner::new();
    let guard = owner.session.lock().await;
    {
        let close = owner.close();
        tokio::pin!(close);
        assert!(matches!(
            close.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }
    drop(guard);
    owner
        .start(false, |_| async {
            Ok(Resource::new(Arc::new(AtomicUsize::new(0))))
        })
        .await
        .unwrap();
    owner.close().await.unwrap();
}
