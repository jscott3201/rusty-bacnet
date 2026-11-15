//! Stopping a transport through a shared reference (#2617).
//!
//! [`TransportPort::stop`](crate::port::TransportPort::stop) takes `&self`,
//! so two owners can stop the same link at once. A transport keeps a
//! [`StopGate`]: the first stop runs, one that comes while it runs waits
//! for it, and one after it returns at once. Its background tasks go in a
//! [`Tasks`], which a stop aborts and joins. Neither holds a lock across an
//! await: each lock is taken only to read or change what it guards, never on
//! a send path, and `start`, which has `&mut self`, reaches them through
//! `get_mut`.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;
use tokio::task::JoinHandle;

/// One stop at a time, and every stop returns once the transport has
/// stopped (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct StopGate(Mutex<Gate>);

#[derive(Debug, Default)]
enum Gate {
    /// Nothing has stopped the transport since it was made or last started.
    #[default]
    Open,
    /// A stop is running; it sends `true` as it ends.
    Stopping(watch::Receiver<bool>),
    /// A stop finished; the next start opens the gate again.
    Stopped,
}

/// The right to run a stop, from [`StopGate::enter`]. Call [`Self::finish`]
/// once the stop is done. Dropped unfinished (the stop was cancelled), it
/// opens the gate again, so a waiting or later stop runs what is left.
#[derive(Debug)]
pub(crate) struct Stopping<'a> {
    gate: &'a StopGate,
    done: watch::Sender<bool>,
    finished: bool,
}

impl StopGate {
    fn lock(&self) -> MutexGuard<'_, Gate> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The right to stop the transport, or `None` once a stop has finished,
    /// after waiting for one that is running. The waiting holds no lock: it
    /// watches the running stop end, then looks again.
    pub(crate) async fn enter(&self) -> Option<Stopping<'_>> {
        loop {
            let mut running = {
                let mut gate = self.lock();
                match &*gate {
                    Gate::Open => {
                        let (done, running) = watch::channel(false);
                        *gate = Gate::Stopping(running);
                        return Some(Stopping {
                            gate: self,
                            done,
                            finished: false,
                        });
                    }
                    Gate::Stopping(running) => running.clone(),
                    Gate::Stopped => return None,
                }
            };
            // Finished, or cancelled (the sender went): either way, look again.
            let _ = running.wait_for(|done| *done).await;
        }
    }

    /// Open the gate for the stop after a new start.
    pub(crate) fn open(&mut self) {
        *self.0.get_mut().unwrap_or_else(PoisonError::into_inner) = Gate::Open;
    }

    /// Whether a stop has finished since the last start.
    pub(crate) fn is_stopped(&self) -> bool {
        matches!(*self.lock(), Gate::Stopped)
    }
}

impl Stopping<'_> {
    /// The stop is done: later stops return at once.
    pub(crate) fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        *self.gate.lock() = if self.finished {
            Gate::Stopped
        } else {
            Gate::Open
        };
        // Wakes the stops that wait, which then look at the gate again.
        let _ = self.done.send(true);
    }
}

/// A transport's background tasks, which a stop through `&self` ends.
#[derive(Debug, Default)]
pub(crate) struct Tasks(Mutex<Vec<JoinHandle<()>>>);

impl Tasks {
    fn lock(&self) -> MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keep `task`, which a start spawned.
    pub(crate) fn push(&mut self, task: JoinHandle<()>) {
        self.0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }

    /// Whether any task is kept: started and not yet stopped.
    pub(crate) fn is_running(&self) -> bool {
        !self.lock().is_empty()
    }

    /// Abort every task, in the order they were kept, without waiting.
    pub(crate) fn abort(&self) {
        for task in self.lock().iter() {
            task.abort();
        }
    }

    /// Abort every task and wait for each to end, in the order they were
    /// kept. A join cancelled part way leaves the task it waited for, and
    /// those after it, kept, for the next stop to join.
    pub(crate) async fn stop(&self) {
        self.abort();
        loop {
            let Some(task) = ({
                let mut tasks = self.lock();
                (!tasks.is_empty()).then(|| tasks.remove(0))
            }) else {
                return;
            };
            let mut waiting = Joining {
                tasks: self,
                task: Some(task),
            };
            if let Some(task) = waiting.task.as_mut() {
                let _ = task.await;
            }
            waiting.task = None;
        }
    }

    /// How many tasks are kept.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    /// The last task kept, to abort from a test.
    #[cfg(test)]
    pub(crate) fn last_abort_handle(&self) -> tokio::task::AbortHandle {
        self.lock().last().expect("a task is kept").abort_handle()
    }

    /// Every task, taken out for a test to join.
    #[cfg(test)]
    pub(crate) fn take_all(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut *self.lock())
    }

    /// Abort every task and let go of them, for `abort` and drop.
    pub(crate) fn abort_and_forget(&mut self) {
        for task in self
            .0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            task.abort();
        }
    }
}

/// A task a stop waits for, which goes back first in line if that wait is
/// cancelled.
struct Joining<'a> {
    tasks: &'a Tasks,
    task: Option<JoinHandle<()>>,
}

impl Drop for Joining<'_> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.tasks.lock().insert(0, task);
        }
    }
}

#[cfg(test)]
#[path = "stopping_tests.rs"]
mod tests;
