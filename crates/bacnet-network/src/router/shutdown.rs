//! Retained teardown: cancellation drops a waiter, never its resource owners.

use std::fmt;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::task::Poll;

use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// One failure from the router's single shutdown attempt.
#[derive(Debug)]
pub enum RouterStopFailure {
    /// A transport returned its original cleanup error.
    Transport {
        /// The configured BACnet network number.
        network_number: u16,
        /// The original error, including any owned I/O cause.
        error: Error,
    },
    /// A worker panicked or its owner disappeared unexpectedly.
    Worker {
        /// The port's configured network number, when the worker belongs to a port.
        network_number: Option<u16>,
        /// The worker or cleanup operation that failed.
        worker: &'static str,
        /// The retained panic or join diagnostic.
        detail: String,
    },
}

/// Failures retained from one settled router shutdown attempt.
///
/// Clones share the original errors. Repeated stop calls replay this report;
/// they do not retry failed cleanup. Errors mean cleanup attempts settled,
/// but do not guarantee that every external resource was released.
#[derive(Clone, Debug)]
pub struct RouterStopError(Arc<[RouterStopFailure]>);

impl RouterStopError {
    /// All failures. Transport owners are joined in configured-port order.
    pub fn failures(&self) -> &[RouterStopFailure] {
        &self.0
    }

    fn from_failures(failures: Vec<RouterStopFailure>) -> Result<(), Self> {
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Self(failures.into()))
        }
    }
}

impl fmt::Display for RouterStopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "router shutdown failed: {:?}", self.0)
    }
}

impl std::error::Error for RouterStopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.iter().find_map(|failure| match failure {
            RouterStopFailure::Transport { error, .. } => {
                Some(error as &(dyn std::error::Error + 'static))
            }
            RouterStopFailure::Worker { .. } => None,
        })
    }
}

/// Catch a worker's panic without surrendering the transport it borrowed.
/// This is local to owned router work; ordinary transport errors stay typed.
pub(super) async fn observe<F: Future>(future: F) -> Result<F::Output, String> {
    let mut future = std::pin::pin!(future);
    poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(panic) => {
                let detail = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "non-string panic payload".into());
                Poll::Ready(Err(detail))
            }
        },
    )
    .await
}

/// Abort is only a fallback when async cleanup failed or was interrupted by
/// runtime shutdown. Successful cleanup disarms it.
pub(super) struct TransportOwner<T: TransportPort> {
    pub transport: T,
    pub stopped: bool,
}

impl<T: TransportPort> Drop for TransportOwner<T> {
    fn drop(&mut self) {
        if !self.stopped {
            self.transport.abort();
        }
    }
}

pub(super) struct Workers {
    pub dispatch: Vec<JoinHandle<()>>,
    pub senders: Vec<(u16, JoinHandle<Vec<RouterStopFailure>>)>,
    pub aging: JoinHandle<()>,
    pub limits: Vec<JoinHandle<()>>,
    pub stop_senders: watch::Sender<bool>,
}

impl Workers {
    async fn stop(mut self) -> Result<(), RouterStopError> {
        // Signal every sender before awaiting any worker, so a pending send
        // cannot block another port's cleanup.
        self.stop_senders.send_replace(true);
        for task in &self.dispatch {
            task.abort();
        }
        self.aging.abort();
        for task in &self.limits {
            task.abort();
        }
        let mut failures = Vec::new();
        for task in &mut self.dispatch {
            joined(task, "dispatch", &mut failures).await;
        }
        joined(&mut self.aging, "aging", &mut failures).await;
        for task in &mut self.limits {
            joined(task, "limit", &mut failures).await;
        }
        for (network_number, task) in &mut self.senders {
            match task.await {
                Ok(port_failures) => failures.extend(port_failures),
                Err(error) => failures.push(RouterStopFailure::Worker {
                    network_number: Some(*network_number),
                    worker: "sender owner",
                    detail: error.to_string(),
                }),
            }
        }
        RouterStopError::from_failures(failures)
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.stop_senders.send_replace(true);
        for task in &self.dispatch {
            task.abort();
        }
        self.aging.abort();
        for task in &self.limits {
            task.abort();
        }
        // Sender owners must keep running to await transport cleanup. Their
        // watch receiver also observes this sender's closure on runtime exit.
    }
}

async fn joined(
    task: &mut JoinHandle<()>,
    worker: &'static str,
    failures: &mut Vec<RouterStopFailure>,
) {
    if let Err(error) = task.await
        && !error.is_cancelled()
    {
        failures.push(RouterStopFailure::Worker {
            network_number: None,
            worker,
            detail: error.to_string(),
        });
    }
}

pub(super) struct Lifecycle {
    pub workers: Option<Workers>,
    cleanup: Option<JoinHandle<Result<(), RouterStopError>>>,
    settled: Option<Result<(), RouterStopError>>,
}

impl Lifecycle {
    pub fn new(workers: Workers) -> Self {
        Self {
            workers: Some(workers),
            cleanup: None,
            settled: None,
        }
    }

    pub fn begin(&mut self) {
        if let Some(workers) = self.workers.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                self.cleanup = Some(runtime.spawn(workers.stop()));
            } else {
                // No async completion promise is possible without a runtime.
                drop(workers);
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), RouterStopError> {
        self.begin();
        if self.settled.is_none() {
            let result = match self.cleanup.as_mut() {
                Some(task) => match task.await {
                    Ok(result) => result,
                    Err(error) => RouterStopError::from_failures(vec![RouterStopFailure::Worker {
                        network_number: None,
                        worker: "shutdown owner",
                        detail: error.to_string(),
                    }]),
                },
                None => RouterStopError::from_failures(vec![RouterStopFailure::Worker {
                    network_number: None,
                    worker: "shutdown owner",
                    detail: "cleanup owner unavailable".into(),
                }]),
            };
            self.cleanup.take();
            self.settled = Some(result);
        }
        self.settled.as_ref().unwrap().clone()
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.begin();
    }
}
