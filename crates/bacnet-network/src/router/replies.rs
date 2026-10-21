//! Weak application reply authority for one running router.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

use bacnet_router_core::router::ReplyPlan;
use bacnet_transport::port::{DirectResponseScope, ResponseScopeGuard};
use bacnet_types::{error::Error, MacAddr};
use bytes::Bytes;
use tokio::sync::oneshot;

use super::{dispatch::Outputs, SendRequest};

#[derive(Debug, Default)]
pub(crate) struct ReplyLifetime {
    stopped: AtomicBool,
    pub(crate) direct: DirectResponseScope,
}

impl ReplyLifetime {
    pub(super) fn seal(&self) {
        self.stopped.store(true, Ordering::Release);
        self.direct.seal();
    }
    fn check(&self) -> Result<(), Error> {
        if self.stopped.load(Ordering::Acquire) {
            Err(unavailable())
        } else {
            Ok(())
        }
    }
}

/// Saved router endpoint and return path. It owns no router, queue or socket.
/// Only local delivery by a running router creates this capability.
#[derive(Clone)]
pub struct RouterResponse {
    /// Shared, so the capability is one pointer in every locally delivered
    /// APDU and in a server's saved response route, and a clone is a
    /// reference count (#1713). Every field is fixed at creation.
    inner: Arc<RouterResponseInner>,
}

struct RouterResponseInner {
    plan: ReplyPlan,
    max_npdu: u16,
    outputs: Weak<Outputs>,
    lifetime: Weak<ReplyLifetime>,
}

impl std::fmt::Debug for RouterResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterResponse")
            .field("plan", &self.inner.plan)
            .finish_non_exhaustive()
    }
}

impl RouterResponse {
    pub(super) fn new(plan: ReplyPlan, max_npdu: u16, outputs: &Arc<Outputs>) -> Self {
        Self {
            inner: Arc::new(RouterResponseInner {
                plan,
                max_npdu,
                outputs: Arc::downgrade(outputs),
                lifetime: Arc::downgrade(&outputs.replies),
            }),
        }
    }
    /// The saved reply addressing.
    pub(crate) fn plan(&self) -> &ReplyPlan {
        &self.inner.plan
    }
    /// The return port's NPDU limit when the request arrived.
    pub(crate) fn max_npdu(&self) -> u16 {
        self.inner.max_npdu
    }
    pub(crate) fn same_origin(&self, other: &Self) -> bool {
        let (this, other) = (&*self.inner, &*other.inner);
        let same_port = this.plan.port() == other.plan.port();
        this.lifetime.ptr_eq(&other.lifetime)
            && this.plan.source() == other.plan.source()
            && this.plan.destination() == other.plan.destination()
            // SNET/SADR identifies a routed requester independently of its
            // immediate return path. Port-local addresses remain scoped to
            // their ingress link; equal MACs on two links are different peers.
            && (this.plan.destination().is_some()
                || (same_port && this.plan.peer() == other.plan.peer()))
            // With no explicitly addressed endpoint, the ingress port selects
            // the local endpoint. An explicit endpoint may be reached via any
            // port of this same router. The saved plan itself never changes.
            && (this.plan.source().is_some() || same_port)
    }
    pub(crate) fn check(&self) -> Result<(), Error> {
        self.lifetime()?.check()
    }
    pub(crate) fn lifetime(&self) -> Result<Arc<ReplyLifetime>, Error> {
        let lifetime = self.inner.lifetime.upgrade().ok_or_else(unavailable)?;
        lifetime.check()?;
        Ok(lifetime)
    }
    pub(crate) async fn send(
        &self,
        npdu: Bytes,
        priority: bacnet_npdu::NetworkPriority,
        owner: ResponseScopeGuard,
        on_issuance: impl FnOnce() + Send + 'static,
    ) -> Result<(), Error> {
        self.check()?;
        if !owner.is_open() {
            return Err(unavailable());
        }
        let (done, completed) = oneshot::channel();
        {
            let outputs = self.inner.outputs.upgrade().ok_or_else(unavailable)?;
            let queue = outputs
                .send_txs
                .get(usize::from(self.inner.plan.port().0))
                .ok_or_else(unavailable)?;
            queue
                .try_send(
                    priority,
                    SendRequest::Unicast {
                        npdu,
                        mac: MacAddr::from_slice(self.inner.plan.peer()),
                        data_attributes: vec![],
                        reject_origin: None,
                        response: Some(ReplyCompletion {
                            done,
                            owner,
                            lifetime: self.inner.lifetime.clone(),
                            on_issuance: Box::new(on_issuance),
                        }),
                    },
                )
                .map_err(|_| unavailable())?;
        }
        completed.await.map_err(|_| unavailable())?
    }
}

pub(super) struct ReplyCompletion {
    done: oneshot::Sender<Result<(), Error>>,
    owner: ResponseScopeGuard,
    lifetime: Weak<ReplyLifetime>,
    on_issuance: Box<dyn FnOnce() + Send>,
}

impl std::fmt::Debug for ReplyCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplyCompletion").finish_non_exhaustive()
    }
}

impl ReplyCompletion {
    /// Cancellation or owner retirement before selection emits no bytes.
    pub(super) fn start(self) -> Option<oneshot::Sender<Result<(), Error>>> {
        if !self.owner.is_open()
            || self.done.is_closed()
            || self
                .lifetime
                .upgrade()
                .is_none_or(|lifetime| lifetime.check().is_err())
        {
            let _ = self.done.send(Err(unavailable()));
            return None;
        }
        (self.on_issuance)();
        Some(self.done)
    }
}

fn unavailable() -> Error {
    Error::Transport(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "original router response route unavailable",
    ))
}
