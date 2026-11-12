//! The tasks that let go of a dropped local write's stage while the database
//! guard was taken, counted so `stop()` can wait for them (#2495).
//!
//! A [`HeldStage`](super::HeldStage) dropped while another task holds the
//! guard hands its release to a task that waits for the guard (#2493). Such a
//! task is short and waits on nothing but the guard, yet `stop()` does not own
//! it: one still holding the guard as `stop()` settles the objects would send
//! the settle down its settle-when-free path, and storage would be put back
//! only after `stop()` returned. So each release task takes a [`Release`]
//! before it is spawned and gives it up as it ends, or as its runtime drops
//! it unrun, and `stop()` waits until none is left, for at most
//! [`RELEASE_WAIT`].

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

/// The longest `stop()` waits for release tasks before it settles the
/// objects: the [`SLOW_SAVE_WARNING`](super::SLOW_SAVE_WARNING) after which it
/// warns that storage holds it up. A release task waits only for the guard,
/// so one still outstanding by then waits behind a guard `stop()` does not
/// own, as an application's, or on a runtime nothing drives any more.
// Coupled to SLOW_SAVE_WARNING on purpose (one 5 s mark for stop()): if that
// changes, recheck that a busy runtime's guard wait still fits, and the "5 s"
// in stop()'s docs and docs/rust-api.md.
pub(in crate::server) const RELEASE_WAIT: Duration = super::SLOW_SAVE_WARNING;

/// The release tasks a server's dropped local writes left, counted.
#[derive(Default)]
pub(in crate::server) struct StageReleases {
    outstanding: AtomicUsize,
    idle: Notify,
    #[cfg(test)]
    hooks: hooks::Hooks,
}

/// One release task's place in the count. It goes when the task's future
/// does: when the task ends, or when a runtime that has shut down drops it
/// unrun.
pub(super) struct Release(Arc<StageReleases>);

impl StageReleases {
    /// Count a release task about to be spawned.
    pub(super) fn register(self: &Arc<Self>) -> Release {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
        Release(Arc::clone(self))
    }

    /// Wait until no release task is outstanding, or [`RELEASE_WAIT`] has
    /// passed: a task behind a guard the application holds must not hold
    /// `stop()` up. Holds no lock.
    pub(in crate::server) async fn wait_at_stop(&self) {
        if tokio::time::timeout(RELEASE_WAIT, self.finished())
            .await
            .is_err()
        {
            tracing::warn!(
                outstanding = self.outstanding.load(Ordering::Acquire),
                waited_secs = RELEASE_WAIT.as_secs(),
                "stop() stopped waiting for dropped local writes to let go of their \
                 stages; the database may be held, or their runtime no longer driven, \
                 so staged writes settle once it is free"
            );
        }
    }

    /// Return once no release task is outstanding.
    async fn finished(&self) {
        loop {
            let idle = self.idle.notified();
            tokio::pin!(idle);
            // Enabled before the check, so a task that ends between the two
            // still wakes this.
            idle.as_mut().enable();
            if self.outstanding.load(Ordering::Acquire) == 0 {
                return;
            }
            #[cfg(test)]
            self.hooks.note_wait();
            idle.await;
        }
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        if self.0.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 {
            #[cfg(test)]
            self.0.hooks.on_idle();
            self.0.idle.notify_waiters();
        }
    }
}

/// The gate [`StageReleases::hold_next`] set, which a release task passes
/// once it holds the guard, before it releases. Only tests have one.
#[cfg(test)]
pub(super) async fn gate(release: Option<&Release>) {
    let Some(release) = release else {
        return;
    };
    let gate = release.0.hooks.gate.lock().unwrap().take();
    if let Some((reached, go)) = gate {
        let _ = reached.send(());
        let _ = go.await;
    }
}

#[cfg(test)]
impl StageReleases {
    /// Hold the next release task once it has the guard, before it releases:
    /// it reports that it got there, then waits for the returned sender to
    /// send or drop.
    pub(in crate::server) fn hold_next(
        &self,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (reached, reached_rx) = tokio::sync::oneshot::channel();
        let (go, go_rx) = tokio::sync::oneshot::channel();
        *self.hooks.gate.lock().unwrap() = Some((reached, go_rx));
        (reached_rx, go)
    }

    /// How many release tasks are outstanding.
    pub(in crate::server) fn outstanding(&self) -> usize {
        self.outstanding.load(Ordering::Acquire)
    }

    /// How many times a wait found a release task outstanding and parked.
    pub(in crate::server) fn waits(&self) -> usize {
        self.hooks.waits.load(Ordering::Acquire)
    }

    /// Run `check` as the count next reaches zero, before any waiter wakes.
    pub(in crate::server) fn when_idle(&self, check: impl FnOnce() + Send + 'static) {
        *self.hooks.idle_check.lock().unwrap() = Some(Box::new(check));
    }

    /// Note that a stage counted here has seen all its saves run.
    pub(super) fn note_stage_saved(&self) {
        self.hooks.stages_saved.fetch_add(1, Ordering::AcqRel);
    }

    /// How many stages counted here have seen all their saves run.
    pub(in crate::server) fn stages_saved(&self) -> usize {
        self.hooks.stages_saved.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod hooks {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;

    /// A release task's gate: it says it reached the gate, then waits.
    type Gate = (oneshot::Sender<()>, oneshot::Receiver<()>);

    #[derive(Default)]
    pub(super) struct Hooks {
        pub(super) gate: Mutex<Option<Gate>>,
        pub(super) waits: AtomicUsize,
        pub(super) stages_saved: AtomicUsize,
        pub(super) idle_check: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl Hooks {
        pub(super) fn note_wait(&self) {
            self.waits.fetch_add(1, Ordering::AcqRel);
        }

        pub(super) fn on_idle(&self) {
            let check = self.idle_check.lock().unwrap().take();
            if let Some(check) = check {
                check();
            }
        }
    }
}

#[cfg(test)]
#[path = "stage_releases_tests.rs"]
mod tests;
