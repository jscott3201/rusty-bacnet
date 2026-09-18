//! Single-owner endpoint session: one ingress + shared coordinator.
//!
//! Lifecycle lives here ONLY. Role handles expose no lifecycle methods.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use bacnet_client::client::ClientConfig;
use bacnet_client::EndpointRequester;
use bacnet_encoding::apdu::Apdu;
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CoordinatorError, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::{
    ClassifierExit, EndpointIngress, PolicyOutcome, PolicyReason,
};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::roles::{
    admit_once, decode_terminal, inbound_canonical_peer, is_requester_lease, ClientRoleHandle,
    ServerRoleHandle, SessionToken,
};
use bacnet_server::server::{
    __endpoint_EndpointResponder as EndpointResponder,
    __endpoint_NotificationTransactions as NotificationTransactions,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready = 0,
    Running = 1,
    Stopped = 2,
}

/// Which roles the session composes.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRole {
    /// Client requester only.
    ClientOnly,
    /// Server responder + notification pool only.
    ServerOnly,
    /// Both sibling roles above one ingress/coordinator.
    Both,
}

/// Session tuning (bounded queues + client timers).
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// Bounded capacity for each ingress queue + egress channel.
    pub queue_capacity: usize,
    /// Client APDU timeout (ms).
    pub apdu_timeout_ms: u64,
    /// Client APDU retries.
    pub apdu_retries: u8,
    /// Client max APDU length.
    pub max_apdu_length: u16,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 16,
            apdu_timeout_ms: 1_000,
            apdu_retries: 0,
            max_apdu_length: 480,
        }
    }
}

/// Snapshot of policy-outcome ownership (session-owned, bounded).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyCountersSnapshot {
    /// Ingress classifier outcomes owned by the session.
    pub ingress_policy: u64,
    /// Inbound requests with no server role (client-only).
    pub no_server_role: u64,
    /// Terminal responses with no client/notification role.
    pub no_client_role: u64,
    /// Terminal responses no lease claimed (unknown/peer/service/policy).
    pub unclaimed_terminal: u64,
    /// Responder declined (group/unsupported/closed).
    pub responder_declined: u64,
}

struct PolicyCounters {
    ingress_policy: u64,
    no_server_role: u64,
    no_client_role: u64,
    unclaimed_terminal: u64,
    responder_declined: u64,
}

struct SessionShared {
    token: Arc<SessionToken>,
    counters: Mutex<PolicyCounters>,
}

/// One owned private endpoint session.
///
/// Owns ONE [`EndpointIngress`], the shared [`OutboundTransactionCoordinator`],
/// role registration, policy-outcome ownership, timers, egress and
/// termination. Start-once/stop-once; `Drop` aborts without orphaning.
///
/// RB-16 identity: an optional [`DeviceIdentity`](crate::identity::DeviceIdentity)
/// is the single source for I-Am + Device readback + role limits when
/// composed via [`with_identity`](Self::with_identity). Standalone sessions
/// without an identity keep the `SessionConfig` 480 default untouched.
#[doc(hidden)]
pub struct EndpointSession<T: TransportPort + 'static> {
    shared: Arc<SessionShared>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    ingress: Option<EndpointIngress<T>>,
    requester: Option<EndpointRequester>,
    responder: Option<Arc<EndpointResponder>>,
    notifications: Option<Arc<NotificationTransactions>>,
    client_handle: Option<ClientRoleHandle>,
    server_handle: Option<ServerRoleHandle>,
    dispatch_task: Option<JoinHandle<SessionExit>>,
    cancel_tx: Option<oneshot::Sender<()>>,
    lifecycle: AtomicU8,
    role: SessionRole,
    #[allow(dead_code)]
    config: SessionConfig,
    client_config: ClientConfig,
    database: Option<ObjectDatabase>,
    identity: Option<crate::identity::DeviceIdentity>,
    egress: Option<bacnet_endpoint_core::endpoint_ingress::EndpointEgress>,
}

/// Terminal state of the session dispatch task.
#[doc(hidden)]
#[derive(Debug)]
pub enum SessionExit {
    /// Explicit `stop()` won the race.
    Cancelled,
    /// Ingress classifier exited (input closed / policy route full/closed).
    Ingress(ClassifierExit),
    /// Dispatch receivers all closed.
    ReceiversClosed,
}

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Creates a session owning `transport` (not yet started).
    #[doc(hidden)]
    pub fn new(transport: T, role: SessionRole, config: SessionConfig) -> Result<Self, Error> {
        if config.queue_capacity == 0 {
            return Err(Error::Encoding(
                "endpoint session queue capacity must be greater than zero".into(),
            ));
        }
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let token = SessionToken::new(Arc::clone(&coordinator));
        let client_config = ClientConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
            max_apdu_length: config.max_apdu_length,
            ..ClientConfig::default()
        };
        Ok(Self {
            shared: Arc::new(SessionShared {
                token,
                counters: Mutex::new(PolicyCounters {
                    ingress_policy: 0,
                    no_server_role: 0,
                    no_client_role: 0,
                    unclaimed_terminal: 0,
                    responder_declined: 0,
                }),
            }),
            coordinator,
            ingress: Some(EndpointIngress::new(transport, config.queue_capacity)),
            requester: None,
            responder: None,
            notifications: None,
            client_handle: None,
            server_handle: None,
            dispatch_task: None,
            cancel_tx: None,
            lifecycle: AtomicU8::new(Lifecycle::Ready as u8),
            role,
            config,
            client_config,
            database: None,
            identity: None,
            egress: None,
        })
    }

    /// Attaches the object database for the server responder (before start).
    #[doc(hidden)]
    pub fn with_database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Composes the single Device identity (before start).
    ///
    /// Truth direction: identity overrides the standalone `SessionConfig`
    /// 480 default for the client role max-APDU; timers/retries stay from
    /// `SessionConfig`. The database should already be built from the same
    /// identity (see `DeviceIdentity::build_database`) so I-Am vs
    /// ReadProperty vs role limits agree. No existing-test churn: sessions
    /// without an identity keep the 480 default.
    #[doc(hidden)]
    pub fn with_identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        identity.apply_to_client_config(&mut self.client_config);
        self.config.max_apdu_length = identity.max_apdu_length();
        self.identity = Some(identity);
        self
    }

    /// Starts ingress, roles and the single dispatch consumer once.
    #[doc(hidden)]
    pub async fn start(&mut self) -> Result<(), Error> {
        if self.lifecycle.compare_exchange(
            Lifecycle::Ready as u8,
            Lifecycle::Running as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Ok(Lifecycle::Ready as u8)
        {
            return Err(Error::Encoding(
                "endpoint session cannot be started more than once".into(),
            ));
        }
        let ingress = self
            .ingress
            .as_mut()
            .ok_or_else(|| Error::Encoding("endpoint session ingress owner is missing".into()))?;
        let receivers = ingress.start().await?;
        let egress = receivers.egress.clone();

        // Role registration shares ONE coordinator + ONE egress. No second
        // demultiplexer, no role-side Invoke-ID allocation: the requester and
        // notification pool reserve from `self.coordinator`; the responder
        // reuses the wire invoke ID directly.
        let (requester, client_handle) =
            if matches!(self.role, SessionRole::ClientOnly | SessionRole::Both) {
                let requester = EndpointRequester::new(
                    egress.clone(),
                    Arc::clone(&self.coordinator),
                    self.client_config.clone(),
                )?;
                let handle = ClientRoleHandle::new(&self.shared.token, requester.clone());
                (Some(requester), Some(handle))
            } else {
                (None, None)
            };
        let (responder, notifications, server_handle) =
            if matches!(self.role, SessionRole::ServerOnly | SessionRole::Both) {
                let db = self.database.take().unwrap_or_else(ObjectDatabase::new);
                let responder = Arc::new(EndpointResponder::new(
                    Arc::new(tokio::sync::RwLock::new(db)),
                    egress.clone(),
                ));
                let notifications =
                    NotificationTransactions::with_coordinator(Arc::clone(&self.coordinator));
                let handle = ServerRoleHandle::new(
                    &self.shared.token,
                    Arc::clone(&responder),
                    Arc::clone(&notifications),
                );
                (Some(responder), Some(notifications), Some(handle))
            } else {
                (None, None, None)
            };

        let (cancel_tx, cancel_rx) = oneshot::channel();
        let dispatch = DispatchParts {
            inbound: receivers.inbound_requests,
            terminal: receivers.terminal_or_segment,
            policy: receivers.policy_outcomes,
            requester: requester.clone(),
            responder: responder.clone(),
            notifications: notifications.clone(),
            coordinator: Arc::clone(&self.coordinator),
            shared: Arc::clone(&self.shared),
        };
        let task = tokio::spawn(dispatch_loop(dispatch, cancel_rx));

        // Dispatch owns the three ingress receivers (single consumer); the
        // session retains one egress clone for the identity I-Am path while
        // the receiver halves move into dispatch. No second demultiplexer.
        self.egress = Some(egress);
        self.requester = requester;
        self.responder = responder;
        self.notifications = notifications;
        self.client_handle = client_handle;
        self.server_handle = server_handle;
        self.dispatch_task = Some(task);
        self.cancel_tx = Some(cancel_tx);
        Ok(())
    }

    /// Stops dispatch, roles and ingress once; joins termination.
    #[doc(hidden)]
    pub async fn stop(&mut self) -> Result<SessionExit, Error> {
        if self.lifecycle.compare_exchange(
            Lifecycle::Running as u8,
            Lifecycle::Stopped as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) != Ok(Lifecycle::Running as u8)
        {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        self.shared.token.shutdown();
        if let Some(requester) = self.requester.take() {
            requester.close();
        }
        if let Some(responder) = self.responder.take() {
            responder.close();
        }
        if let Some(notifications) = self.notifications.take() {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        let exit = match self.dispatch_task.take() {
            Some(task) => task
                .await
                .map_err(|e| Error::Encoding(format!("endpoint session dispatch failed: {e}")))?,
            None => SessionExit::ReceiversClosed,
        };
        if let Some(ingress) = self.ingress.as_mut() {
            // Ingress stop reports its classifier exit; session exit above
            // already owns termination, so a closed ingress here is expected.
            let _ = ingress.stop().await;
        }
        // Dispatch owned the ingress receivers (single consumer); dropping
        // the task + ingress ends them with no detached queue outliving
        // termination.
        self.client_handle = None;
        self.server_handle = None;
        Ok(exit)
    }

    /// Borrows the client role (fails when not composed).
    #[doc(hidden)]
    pub fn client(&self) -> Option<&ClientRoleHandle> {
        self.client_handle.as_ref()
    }

    /// Borrows the server role (fails when not composed).
    #[doc(hidden)]
    pub fn server(&self) -> Option<&ServerRoleHandle> {
        self.server_handle.as_ref()
    }

    /// Cloned client handle proving session binding (for drop tests).
    #[doc(hidden)]
    pub fn cloned_client_handle(&self) -> Option<ClientRoleHandle> {
        self.client_handle.clone()
    }

    /// Cloned server handle proving session binding (for drop tests).
    #[doc(hidden)]
    pub fn cloned_server_handle(&self) -> Option<ServerRoleHandle> {
        self.server_handle.clone()
    }

    /// Samples policy-outcome ownership counters.
    #[doc(hidden)]
    pub async fn policy_counters(&self) -> PolicyCountersSnapshot {
        let counters = self.shared.counters.lock().await;
        PolicyCountersSnapshot {
            ingress_policy: counters.ingress_policy,
            no_server_role: counters.no_server_role,
            no_client_role: counters.no_client_role,
            unclaimed_terminal: counters.unclaimed_terminal,
            responder_declined: counters.responder_declined,
        }
    }

    /// Samples the shared outbound lease count.
    #[doc(hidden)]
    pub fn active_leases(&self) -> usize {
        self.coordinator.active_count().unwrap_or(usize::MAX)
    }

    /// Shares the device-wide coordinator (dispatch + tests only).
    #[doc(hidden)]
    pub fn coordinator(&self) -> Arc<OutboundTransactionCoordinator> {
        Arc::clone(&self.coordinator)
    }

    /// Borrows the composed identity (narrow admin; no lifecycle).
    ///
    /// RB-15 narrow admin borrow: roles hold no transport/session lifecycle;
    /// only the session owner exposes identity + counters + coordinator.
    /// A role handle attempting `stop`/transport access fails at compile
    /// time (no such method) and at runtime its post-stop calls fail closed.
    #[doc(hidden)]
    pub fn identity(&self) -> Option<&crate::identity::DeviceIdentity> {
        self.identity.as_ref()
    }

    /// Returns the composed session role (narrow admin).
    #[doc(hidden)]
    pub fn session_role(&self) -> SessionRole {
        self.role
    }

    /// Returns true while the session dispatch is running (narrow admin).
    #[doc(hidden)]
    pub fn is_running(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) == Lifecycle::Running as u8
    }

    /// Broadcasts an I-Am consistent with the composed identity.
    ///
    /// Missing-identity path: errors without sending (no invented device).
    /// Present-identity path: encodes [`DeviceIdentity::encode_iam_apdu`](crate::identity::DeviceIdentity::encode_iam_apdu)
    /// and sends one local-broadcast Unconfirmed-Request via the session
    /// egress. Field-for-field identical to the server discovery I-Am built
    /// from [`DeviceIdentity::server_config`](crate::identity::DeviceIdentity::server_config).
    /// Transport-dependent: B/IP reaches the local subnet broadcast; SC
    /// relays via the hub as a broadcast NPDU.
    #[doc(hidden)]
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint I-Am requires a composed identity".into()))?;
        let egress = self
            .egress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session is not running".into()))?;
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let apdu = identity.encode_iam_apdu()?;
        egress
            .send_apdu(
                apdu,
                bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::LocalBroadcast,
                false,
                bacnet_types::enums::NetworkPriority::NORMAL,
                Vec::new(),
            )
            .await
    }
}

impl<T: TransportPort + 'static> Drop for EndpointSession<T> {
    fn drop(&mut self) {
        // Synchronous abort path: never orphan dispatch/ingress/role work.
        // `stop()` remains the graceful path; Drop only seals + aborts.
        self.shared.token.shutdown();
        if let Some(requester) = self.requester.take() {
            requester.close();
        }
        if let Some(responder) = self.responder.take() {
            responder.close();
        }
        if let Some(notifications) = self.notifications.take() {
            notifications.close();
        }
        if let Some(cancel) = self.cancel_tx.take() {
            let _ = cancel.send(());
        }
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
        }
    }
}

struct DispatchParts {
    inbound: mpsc::Receiver<ReceivedApdu>,
    terminal: mpsc::Receiver<ReceivedApdu>,
    policy: mpsc::Receiver<PolicyOutcome>,
    requester: Option<EndpointRequester>,
    responder: Option<Arc<EndpointResponder>>,
    notifications: Option<Arc<NotificationTransactions>>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    shared: Arc<SessionShared>,
}

async fn dispatch_loop(mut parts: DispatchParts, mut cancel: oneshot::Receiver<()>) -> SessionExit {
    // Single ingress consumer: this task alone polls all three receivers.
    // Every await is cancellation-safe: leases are held by RAII guards
    // (request guard / notification operation) so dropping this future
    // cannot strand a coordinator slot.
    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => return SessionExit::Cancelled,
            received = parts.inbound.recv() => {
                let Some(received) = received else {
                    // Inbound closed: keep terminal/policy draining; ingress
                    // closure is reported via policy/terminal shutdown below.
                    // If both are also closed, exit.
                    if parts.terminal.is_closed() && parts.policy.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_inbound(&mut parts, received).await;
            }
            received = parts.terminal.recv() => {
                let Some(received) = received else {
                    if parts.inbound.is_closed() && parts.policy.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_terminal(&mut parts, received).await;
            }
            outcome = parts.policy.recv() => {
                let Some(outcome) = outcome else {
                    if parts.inbound.is_closed() && parts.terminal.is_closed() {
                        return SessionExit::ReceiversClosed;
                    }
                    continue;
                };
                handle_ingress_policy(&parts.shared, outcome).await;
            }
        }
    }
}

async fn handle_inbound(parts: &mut DispatchParts, received: ReceivedApdu) {
    // Preserve the full envelope structurally (raw + effective group,
    // attributes, ingress identity, provenance) — no new decisions here
    // beyond role presence + responder scope.
    let _peer = inbound_canonical_peer(&received);
    let _link_group = received.link_layer_group;
    let _is_group = received.is_group;
    let _attributes = received.data_attributes.clone();
    let _provenance = received.provenance;
    let _ingress = received.ingress_network;
    let Some(responder) = parts.responder.as_ref() else {
        let mut counters = parts.shared.counters.lock().await;
        counters.no_server_role += 1;
        // One-use reply ownership: release a lone reply sender without
        // sending bytes, mirroring queue-admission drop semantics.
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    };
    // RB-17 deferred-reply wiring (minimal correlation, session-side): when
    // the one-shot suspension arm is set and this request carries the MS/TP
    // one-use prompt reply sender, drop the sender BEFORE the responder sees
    // it. Dropping releases the MAC to send ReplyPostponed; the responder
    // then observes `reply_tx: None` and answers through the single
    // EndpointEgress → queue_npdu path, which the MAC transmits as
    // DataNotExpectingReply at the next token opportunity with the identical
    // wire invoke ID + destination. Prompt (sender present, unarmed) and
    // token-owned (sender absent) stay distinct by construction: the
    // responder never sees both for one request. No second serial owner, no
    // MAC state duplication, no transport change.
    let mut received = received;
    if received.reply_tx.is_some() && parts.shared.token.take_suspend() {
        let _ = received.reply_tx.take();
    }
    match responder.handle(received).await {
        Ok(true) | Ok(false) => {}
        Err(_) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.responder_declined += 1;
        }
    }
}

async fn handle_terminal(parts: &mut DispatchParts, received: ReceivedApdu) {
    let Some(apdu) = decode_terminal(&received) else {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    };
    // Inbound server transactions never allocate here: requests are not
    // terminal traffic. A Confirmed/Unconfirmed request in this queue is a
    // classifier violation → policy-owned, no coordinator interaction.
    if matches!(
        apdu,
        Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
    ) {
        let mut counters = parts.shared.counters.lock().await;
        counters.unclaimed_terminal += 1;
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
        return;
    }
    // Exactly one shared-coordinator admit per terminal APDU (exact-once
    // terminal claim). Equal numeric IDs are unambiguous: the admitted
    // lease owner selects requester vs notification.
    let outcome = admit_once(&parts.coordinator, &received, &apdu);
    let admission = match outcome {
        Ok(AdmissionOutcome::Admitted(admission)) => admission,
        Ok(_) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
        Err(CoordinatorError::StatePoisoned) => {
            let mut counters = parts.shared.counters.lock().await;
            counters.unclaimed_terminal += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        }
    };
    if is_requester_lease(&admission) {
        let Some(requester) = parts.requester.as_ref() else {
            // Lease was admitted but has no consumer: release the exact
            // lease so exhaustion → release → reuse still holds.
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        // `complete_pre_admitted` delivers via the exact token without
        // re-admitting; a `false` means the TSM no longer owns it (lost
        // consumer / stale), the coordinator lease was already completed
        // inside the TSM path on success.
        let _ = requester
            .complete_pre_admitted(admission, apdu, received)
            .await;
    } else {
        let Some(notifications) = parts.notifications.as_ref() else {
            let _ = parts.coordinator.release(admission.token());
            let mut counters = parts.shared.counters.lock().await;
            counters.no_client_role += 1;
            if let Some(reply_tx) = received.reply_tx {
                drop(reply_tx);
            }
            return;
        };
        let _ = notifications.complete_pre_admitted(admission, &apdu);
        // Notification replies carry no reply sender; drop one if present.
        if let Some(reply_tx) = received.reply_tx {
            drop(reply_tx);
        }
    }
}

async fn handle_ingress_policy(shared: &Arc<SessionShared>, outcome: PolicyOutcome) {
    // Policy-outcome ownership: the session counts every classifier outcome
    // (malformed/unsupported/route-full/route-closed) instead of dropping it.
    let mut counters = shared.counters.lock().await;
    counters.ingress_policy += 1;
    let _ = outcome.reason;
    // Policy outcomes own the full envelope including a possible one-use
    // reply sender; releasing here preserves single-consumer + one-use
    // ownership without sending bytes.
    if let Some(reply_tx) = outcome.received.reply_tx {
        drop(reply_tx);
    }
    let _ = PolicyReason::MalformedApdu;
}
