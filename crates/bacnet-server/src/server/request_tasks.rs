use super::request_admission::{Admission, Class, Rejection};
use super::{RequestAdmissionCounters, RequestAdmissionPolicy};
use crate::command_lists::Unfinished;
use bacnet_types::primitives::ObjectIdentifier;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, Sleep};

/// The longest a finished handler waits to be joined while no datagram turns
/// the dispatch loop (#1713).
const REAP_IDLE: Duration = Duration::from_millis(100);

/// Paces the dispatch loop's joins while ingress is open (#1713). The loop
/// joins finished handlers at the top of each turn with
/// [`RequestTasks::reap_finished`] instead of awaiting the JoinSet: awaiting
/// it makes the loop the set's waker, so every finished request woke the loop
/// once more, a thread hand-off per request. This timer turns an idle loop
/// within [`REAP_IDLE`], so a finished handler is still joined without one.
pub(super) struct IdleReap(Pin<Box<Sleep>>);

impl IdleReap {
    pub(super) fn new() -> Self {
        Self(Box::pin(tokio::time::sleep(REAP_IDLE)))
    }

    /// Ready [`REAP_IDLE`] after the last time it was. Cancel-safe: the poll
    /// that finds the timer done also re-arms it.
    pub(super) async fn tick(&mut self) {
        self.0.as_mut().await;
        self.0.as_mut().reset(Instant::now() + REAP_IDLE);
    }
}

/// Owns inbound request handlers, their independent segmented responses and
/// the Command object runs a write starts (#1150), not timers or notification
/// workers started by services. The optional DCC bucket shares this
/// native-server lifetime, independent of task registrations.
///
/// The last field holds the runs let go of while the database was busy
/// (#1252), for `stop()` to end once every task here is joined. It has its
/// own lock: a task the runtime refuses can be dropped while `State` is held.
pub(super) struct RequestTasks(
    Mutex<State>,
    Admission,
    Option<super::dcc_disable_rate::Bucket>,
    Mutex<Vec<Unfinished>>,
);

impl Default for RequestTasks {
    fn default() -> Self {
        Self::new(RequestAdmissionPolicy::default()).expect("valid defaults")
    }
}

#[derive(Default)]
struct State {
    audit_owner: Option<Weak<bacnet_objects::database::AuditOwnership>>,
    closed: bool,
    tasks: JoinSet<()>,
}

/// Descendants must not keep their owning JoinSet alive through a cycle.
#[derive(Clone)]
pub(super) struct RequestTaskSpawner(Weak<RequestTasks>);

impl RequestTaskSpawner {
    pub(super) fn admit_dcc_disable(&self) -> bool {
        self.0.upgrade().is_some_and(|owner| {
            owner
                .2
                .as_ref()
                .is_none_or(super::dcc_disable_rate::Bucket::admit)
        })
    }

    pub(super) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        if let Some(owner) = self.0.upgrade() {
            owner.spawn(task);
        }
    }

    /// Keep a run let go of while the database was busy for `stop()` to end,
    /// or hand it back once the owner is gone.
    pub(super) fn strand(&self, left: Unfinished) -> Result<(), Unfinished> {
        match self.0.upgrade() {
            Some(owner) => {
                owner.3.lock().unwrap().push(left);
                Ok(())
            }
            None => Err(left),
        }
    }

    /// Take back `source`'s run of `generation` if it was stranded.
    pub(super) fn unstrand(&self, source: ObjectIdentifier, generation: u64) -> Option<Unfinished> {
        let owner = self.0.upgrade()?;
        let mut stranded = owner.3.lock().unwrap();
        let at = stranded
            .iter()
            .position(|left| left.is(source, generation))?;
        Some(stranded.swap_remove(at))
    }
}

impl RequestTasks {
    #[cfg(test)]
    pub(super) fn peer_entries(&self) -> [usize; 3] {
        self.1.peer_entries()
    }

    pub(super) fn for_server(
        config: &super::ServerConfig,
    ) -> Result<Arc<Self>, bacnet_types::error::Error> {
        let mut tasks = Self::new(config.request_admission_policy)?;
        // Retain admission validation precedence; both policies must be valid
        // before the lifecycle starts a transport or exposes a request owner.
        config.read_property_multiple_budget.validate()?;
        config.validate_dcc_config()?;
        config.time_sync_policy.validate()?;
        config.get_alarm_summary_budget.validate()?;
        config.get_enrollment_summary_budget.validate()?;
        config.atomic_read_file_budget.validate()?;
        config.atomic_write_file_budget.validate()?;
        config.read_range_budget.validate()?;
        config.get_event_information_budget.validate()?;
        config.cov_policy.validate()?;
        tasks.2 = config
            .dcc_disable_rate_limit
            .map(super::dcc_disable_rate::Bucket::new)
            .transpose()?;
        Ok(Arc::new(tasks))
    }

    pub(super) fn new(policy: RequestAdmissionPolicy) -> Result<Self, bacnet_types::error::Error> {
        Ok(Self(
            Mutex::new(State::default()),
            Admission::new(policy)?,
            None,
            Mutex::default(),
        ))
    }

    /// The runs stranded so far, for `stop()` to end.
    pub(super) fn take_stranded(&self) -> Vec<Unfinished> {
        std::mem::take(&mut *self.3.lock().unwrap())
    }

    pub(super) fn counters(&self) -> RequestAdmissionCounters {
        self.1.snapshot()
    }

    /// Acquire and register synchronously with close; construct the future only
    /// on success. The guard lives in that future, including before its first poll.
    ///
    /// The handler's future is boxed as it is made (#1713): it is several
    /// kilobytes, and tokio's spawn path and task stages otherwise copy all of
    /// it many times over for every request.
    pub(super) fn try_spawn<F: Future<Output = ()> + Send + 'static>(
        &self,
        class: Class,
        peer: super::request_peer::CanonicalRequester,
        make: impl FnOnce() -> F,
    ) -> Result<(), Rejection> {
        let mut state = self.0.lock().unwrap();
        let guard = self.1.try_enter(class, peer, state.closed)?;
        let task = Box::pin(make());
        let owner = state.audit_owner.as_ref().and_then(Weak::upgrade);
        state.tasks.spawn(async move {
            let _owner = owner;
            let _guard = guard;
            task.await;
        });
        Ok(())
    }

    pub(super) fn spawner(self: &Arc<Self>) -> RequestTaskSpawner {
        RequestTaskSpawner(Arc::downgrade(self))
    }

    pub(super) fn set_audit_owner(&self, owner: &Arc<bacnet_objects::database::AuditOwnership>) {
        self.0.lock().unwrap().audit_owner = Some(Arc::downgrade(owner));
    }

    pub(super) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let mut state = self.0.lock().unwrap();
        // Admission and registration are one synchronous critical section.
        if !state.closed {
            let owner = state.audit_owner.as_ref().and_then(Weak::upgrade);
            state.tasks.spawn(async move {
                let _owner = owner;
                task.await;
            });
        }
    }

    /// [`Self::spawn`] onto the runtime `handle` names, for a caller that
    /// checked for one before it changed anything.
    pub(super) fn spawn_on(
        &self,
        task: impl Future<Output = ()> + Send + 'static,
        handle: &tokio::runtime::Handle,
    ) {
        let mut state = self.0.lock().unwrap();
        if !state.closed {
            let owner = state.audit_owner.as_ref().and_then(Weak::upgrade);
            state.tasks.spawn_on(
                async move {
                    let _owner = owner;
                    task.await;
                },
                handle,
            );
        }
    }

    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        state.tasks.abort_all();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.lock().unwrap().tasks.is_empty()
    }

    /// Join every handler that has finished, without waiting and without
    /// registering a waker, so a completion doesn't wake the dispatch loop
    /// (see [`IdleReap`]).
    pub(super) fn reap_finished(&self) {
        loop {
            let Some(result) = self.0.lock().unwrap().tasks.try_join_next() else {
                return;
            };
            Self::observe(Some(result));
        }
    }

    /// One join consumer at a time: dispatch while running, stop after dispatch
    /// is joined. No handle or mutex guard is held across an await, so cancelling
    /// stop leaves the remaining joins in this server-owned set.
    pub(super) async fn join_next(&self) -> Option<Result<(), JoinError>> {
        poll_fn(|cx| self.0.lock().unwrap().tasks.poll_join_next(cx)).await
    }

    pub(super) fn observe(result: Option<Result<(), JoinError>>) {
        if let Some(Err(error)) = result {
            if !error.is_cancelled() {
                tracing::warn!(%error, "Inbound request handler failed");
            }
        }
    }
}

#[cfg(test)]
mod audit_lifetime_tests {
    use super::*;

    #[tokio::test]
    async fn admitted_request_retains_audit_membership_until_its_frame_is_destroyed() {
        use bacnet_objects::database::AuditOwnership;
        use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};
        let owner = AuditOwnership::for_source(
            ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
            ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap(),
        );
        let weak = Arc::downgrade(&owner);
        let requests = RequestTasks::default();
        requests.set_audit_owner(&owner);
        requests
            .try_spawn(
                Class::Confirmed,
                super::super::request_peer::CanonicalRequester::Direct(
                    bacnet_types::MacAddr::from_slice(&[1]),
                ),
                std::future::pending::<()>,
            )
            .unwrap();
        owner.seal();
        requests.close();
        drop(owner);
        assert!(
            weak.upgrade().is_some(),
            "abort request is not task-frame quiescence"
        );
        while requests.join_next().await.is_some() {}
        assert!(weak.upgrade().is_none());
    }
}
