use super::request_admission::{Admission, Class, Rejection};
use super::{RequestAdmissionCounters, RequestAdmissionPolicy};
use std::future::{poll_fn, Future};
use std::sync::{Arc, Mutex, Weak};
use tokio::task::{JoinError, JoinSet};

/// Owns inbound request handlers and their independent segmented responses,
/// not timers or notification workers started by services. The optional DCC
/// bucket shares this native-server lifetime, independent of task registrations.
pub(super) struct RequestTasks(
    Mutex<State>,
    Admission,
    Option<super::dcc_disable_rate::Bucket>,
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
        ))
    }

    pub(super) fn counters(&self) -> RequestAdmissionCounters {
        self.1.snapshot()
    }

    /// Acquire and register synchronously with close; construct the future only
    /// on success. The guard lives in that future, including before its first poll.
    pub(super) fn try_spawn<F: Future<Output = ()> + Send + 'static>(
        &self,
        class: Class,
        peer: super::request_peer::CanonicalRequester,
        make: impl FnOnce() -> F,
    ) -> Result<(), Rejection> {
        let mut state = self.0.lock().unwrap();
        let guard = self.1.try_enter(class, peer, state.closed)?;
        let task = make();
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

    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        state.tasks.abort_all();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.lock().unwrap().tasks.is_empty()
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
        let owner = AuditOwnership::new(
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
