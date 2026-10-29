//! Private lifecycle ordering, independent of Python and transport construction.
//!
//! Admission is acquisition of `session`, not creation of a Python Future.
//! An admitted worker owns that guard through startup/publication or teardown.
//! Dropping its waiter never aborts partial transport startup or joined cleanup.

use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, oneshot};

/// The concrete owner must finish failed-start cleanup before returning Err;
/// stop must settle teardown even when it reports a terminal dispatch error.
pub(crate) trait Session: Send + 'static {
    type Error: Send + 'static;
    fn start(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn stop(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug)]
pub(crate) enum LifecycleError<E> {
    AlreadyStarted,
    StartupCancelled,
    Operation(E),
    WorkerEnded,
}

struct Pending<P> {
    sealed: bool,
    objects: Vec<P>,
}

#[derive(Default)]
struct Preparation {
    cancel: Option<oneshot::Sender<()>>,
    closing: usize,
}

// A close request remains visible until admission or waiter cancellation. This
// covers a concurrent start that acquired the async lock but has not installed
// its preparation sender yet; the sender alone would leave a liveness gap.
struct CloseRequest(Arc<StdMutex<Preparation>>);
impl Drop for CloseRequest {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).closing -= 1;
    }
}

pub(crate) struct Lifecycle<S, P> {
    pub(crate) session: Arc<Mutex<Option<S>>>,
    pending: Arc<StdMutex<Pending<P>>>,
    preparing: Arc<StdMutex<Preparation>>,
}

impl<S, P> Clone for Lifecycle<S, P> {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            pending: self.pending.clone(),
            preparing: self.preparing.clone(),
        }
    }
}

impl<S, P> Lifecycle<S, P> {
    pub(crate) fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            preparing: Arc::new(StdMutex::new(Preparation::default())),
            pending: Arc::new(StdMutex::new(Pending {
                sealed: false,
                objects: Vec::new(),
            })),
        }
    }

    pub(crate) fn push(&self, object: P) -> Result<(), ()> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.sealed {
            return Err(());
        }
        pending.objects.push(object);
        Ok(())
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .objects
            .len()
    }
}

impl<S: Session, P: Clone + Send + 'static> Lifecycle<S, P> {
    pub(crate) async fn start<F, Fut>(
        &self,
        reenter: bool,
        build: F,
    ) -> Result<(), LifecycleError<S::Error>>
    where
        F: FnOnce(Vec<P>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<S, S::Error>> + Send,
    {
        let mut session = self.session.clone().lock_owned().await;
        if session.is_some() {
            return if reenter {
                Ok(())
            } else {
                Err(LifecycleError::AlreadyStarted)
            };
        }
        // No await between admission, registration sealing, and worker transfer.
        let objects = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.sealed = true;
            std::mem::take(&mut pending.objects)
        };
        let pending = self.pending.clone();
        let preparing = self.preparing.clone();
        let (cancel, cancelled) = oneshot::channel();
        {
            let mut preparation = preparing.lock().unwrap_or_else(|e| e.into_inner());
            if preparation.closing > 0 {
                let _ = cancel.send(());
            } else {
                preparation.cancel = Some(cancel);
            }
        }
        let (mut result, waiter) = oneshot::channel();
        tokio::spawn(async move {
            // Preparation owns only local resources. TLS dial/handshake has no
            // local timeout and no spawned tasks, so cancellation drops it safely.
            // The transport session's start is a separate, retained phase below.
            let prepared = tokio::select! {
                biased;
                _ = result.closed() => None,
                _ = cancelled => Some(Err(LifecycleError::StartupCancelled)),
                prepared = build(objects.clone()) => Some(prepared.map_err(LifecycleError::Operation)),
            };
            preparing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cancel
                .take();
            let outcome = match prepared {
                Some(Ok(mut running)) => {
                    match running.start().await {
                        Err(error) => Err(LifecycleError::Operation(error)),
                        Ok(()) if result.is_closed() => {
                            // Settle ingress.start before stopping. Aborting it
                            // could strand a Ready ingress with resources.
                            running.stop().await.map_err(LifecycleError::Operation)
                        }
                        Ok(()) => {
                            *session = Some(running);
                            Ok(())
                        }
                    }
                }
                Some(Err(error)) => Err(error),
                None => Ok(()),
            };
            if session.is_none() {
                let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
                pending.objects = objects;
                pending.sealed = false;
            }
            // A cancellation racing this publication can leave a running session.
            // Awaited close is the cleanup barrier in either outcome.
            let _ = result.send(outcome);
        });
        waiter.await.map_err(|_| LifecycleError::WorkerEnded)?
    }

    pub(crate) async fn close(&self) -> Result<(), LifecycleError<S::Error>> {
        // Request cancellation before waiting for admission. Keep intent visible
        // through the start-admission/sender-publication gap. Once admitted, this
        // request cannot cancel a later generation; cancellation drops it too.
        let request = {
            let mut preparing = self.preparing.lock().unwrap_or_else(|e| e.into_inner());
            preparing.closing += 1;
            if let Some(cancel) = preparing.cancel.take() {
                let _ = cancel.send(());
            }
            CloseRequest(self.preparing.clone())
        };
        let mut session = self.session.clone().lock_owned().await;
        drop(request);
        let pending = self.pending.clone();
        let (result, waiter) = oneshot::channel();
        tokio::spawn(async move {
            let outcome = match session.as_mut() {
                Some(running) => running.stop().await.map_err(LifecycleError::Operation),
                None => Ok(()),
            };
            // stop has settled; dropping the terminal owner releases resources.
            session.take();
            pending.lock().unwrap_or_else(|e| e.into_inner()).sealed = false;
            let _ = result.send(outcome);
        });
        waiter.await.map_err(|_| LifecycleError::WorkerEnded)?
    }
}

#[cfg(test)]
mod admission_gap_tests {
    use super::*;
    use std::task::{Context, Poll, Waker};

    struct Ready;
    impl Session for Ready {
        type Error = ();
        async fn start(&mut self) -> Result<(), ()> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<(), ()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)] // Deliberate private barrier before sender publication.
    async fn close_intent_covers_admitted_start_before_preparation_sender_exists() {
        let owner = Lifecycle::<Ready, ()>::new();
        let pending = owner.pending.lock().unwrap();
        let start = tokio::spawn({
            let owner = owner.clone();
            async move {
                owner
                    .start(false, |_| async {
                        std::future::pending::<()>().await;
                        Ok(Ready)
                    })
                    .await
            }
        });
        // One runtime worker holds session and waits on this exact private barrier.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while owner.session.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let close = owner.close();
        tokio::pin!(close);
        assert!(matches!(
            close.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(pending);
        tokio::time::timeout(std::time::Duration::from_secs(2), close)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            start.await.unwrap(),
            Err(LifecycleError::StartupCancelled)
        ));
        owner.start(false, |_| async { Ok(Ready) }).await.unwrap();
        owner.close().await.unwrap();
    }
}
