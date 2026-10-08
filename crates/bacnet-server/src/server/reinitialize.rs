//! ReinitializeDevice (Clause 16.4): the application handler that carries a
//! request out, the requester context it receives, the builder setters for
//! the handler and the password, and the guarded reply the full server and
//! endpoint sessions share.

use std::panic::{catch_unwind, AssertUnwindSafe};

use bacnet_transport::port::{DirectScIdentity, TransportProvenance};
use bacnet_types::enums::ReinitializedState;

use super::requests::confirmed_response::error_apdu_from_error;
use super::*;

/// Carries out a ReinitializeDevice request (Clause 16.4) on a full server or
/// an endpoint session.
///
/// It is called only for a request that decoded, passed its password check
/// and names a state the clause defines. `Ok(())` sends the SimpleACK; an
/// `Err` is sent in its place. The rules:
///
/// - **Reply first, restart later.** The SimpleACK goes out only after the
///   handler returns, so it must not restart the device inline. Accept or
///   refuse, prepare, and schedule the restart (or the backup or restore
///   step) to run after the reply: Clause 16.4.2 has the device acknowledge
///   the request before it shuts down.
/// - **No reply signal yet.** Nothing tells the application when the
///   SimpleACK has left (#1565), so any delay before restarting is best
///   effort, and on MS/TP it can need longer: a postponed reply waits for
///   the token. [`BACnetServer::stop`] seals responses and aborts request
///   tasks before joining them, so a restart path that stops the server
///   before the reply has left drops the SimpleACK. On a full server, accepted
///   WARMSTART/COLDSTART ends DISABLE_INITIATION immediately (Clause 16.1.2),
///   cancels its timer and resumes held COV, even if the reply later fails.
/// - **Be quick.** It runs synchronously on a runtime worker with the object
///   database write-locked, so every other request waits for it. Hand slow
///   work, such as writing backup files, to a task. On an endpoint session a
///   blocking handler stalls the whole session.
/// - **Database edits are raw.** On a full server, changes made through the
///   `&mut ObjectDatabase` skip what [`BACnetServer::write_local`] adds
///   around a write: the Object_Name uniqueness check, the COV fanout, the
///   event pass, Schedule and Command follow-ups, and the Audit record. A
///   change that needs them, as ACTIVATE_CHANGES work on Network Port or
///   Device properties may, belongs in `write_local`, called after the
///   reply from a task the handler schedules (for example by sending the
///   change to the code that holds the server).
/// - **Gate the requester.** Without `reinit_password` any peer reaches the
///   handler. [`MutationPolicy`](crate::mutation::MutationPolicy) and the
///   mutation authorizer don't cover ReinitializeDevice, so check the
///   [`ReinitializeContext`] to restrict sources.
/// - **Refuse with [`Error::Protocol`].** Its class and code go back in an
///   Error PDU, for example DEVICE / CONFIGURATION_IN_PROGRESS (Clause
///   16.4.1.3.1). Once the handler has run, the request may no longer be
///   rejected (Clause 20.1.8), so an [`Error::Reject`], or any error other
///   than `Protocol` or `Structured`, is sent as SERVICES / OTHER. A panic is
///   caught (with unwind builds) and also answered SERVICES / OTHER, since
///   the handler may have partly acted.
pub type ReinitializeHandler =
    Arc<dyn Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error> + Send + Sync>;

/// The ReinitializeDevice request a [`ReinitializeHandler`] is asked to carry
/// out, and who sent it.
///
/// The addresses are claims, not an authenticated operator. For routed
/// traffic `source_mac` is the last router, so restrict routed requesters by
/// `source_network`; only [`direct_sc_identity`](Self::direct_sc_identity) is
/// verified. Neither the mutation policy nor the mutation authorizer sees
/// this service, so a handler that serves only some peers checks them here.
///
/// The server builds the context; [`new`](Self::new) lets a test build one
/// to call a handler directly.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReinitializeContext {
    /// The requested state, one of those Clause 16.4 defines.
    pub state: ReinitializedState,
    /// Immediate data-link peer (or router) address.
    pub source_mac: MacAddr,
    /// Originating NPDU source when the request was routed.
    pub source_network: Option<NpduAddress>,
    /// Immutable ingress snapshot, retained after the admitting connection closes.
    pub provenance: TransportProvenance,
    /// Confirmed-request invoke identifier.
    pub invoke_id: u8,
}

impl ReinitializeContext {
    /// A context with these fields, for calling a handler outside the server,
    /// such as in its unit tests. Outside `bacnet-transport` only
    /// [`TransportProvenance::unverified`] can be built.
    pub fn new(
        state: ReinitializedState,
        source_mac: MacAddr,
        source_network: Option<NpduAddress>,
        provenance: TransportProvenance,
        invoke_id: u8,
    ) -> Self {
        Self {
            state,
            source_mac,
            source_network,
            provenance,
            invoke_id,
        }
    }

    /// Original verified direct-SC leaf and incarnation, separate from claims.
    pub fn direct_sc_identity(&self) -> Option<DirectScIdentity> {
        self.provenance.direct_sc_identity()
    }
}

impl std::fmt::Debug for ReinitializeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReinitializeContext")
            .field("state", &self.state)
            .field("source_mac_len", &self.source_mac.len())
            .field(
                "source_network",
                &self
                    .source_network
                    .as_ref()
                    .map(|s| (s.network, s.mac_address.len())),
            )
            .field("provenance", &self.provenance)
            .field("invoke_id", &self.invoke_id)
            .finish_non_exhaustive()
    }
}

/// Where a ReinitializeDevice request came from: its [`ReinitializeContext`]
/// short of the state, which decoding supplies.
pub(in crate::server) struct Requester {
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    provenance: TransportProvenance,
}

impl Requester {
    pub(in crate::server) fn new(
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        provenance: TransportProvenance,
    ) -> Self {
        Self {
            source_mac: MacAddr::from_slice(source_mac),
            source_network: source_network.cloned(),
            provenance,
        }
    }
}

/// Answer a ReinitializeDevice request: decode it, check its password and its
/// state, then run `handler` with the database write-locked. A valid request
/// with no handler is refused with SERVICES / SERVICE_REQUEST_DENIED.
///
/// `still_open` runs once the write lock is held, so an owner that closed
/// while the request waited refuses it before the handler runs. The guard is
/// released before `on_accepted` runs. That callback runs synchronously on
/// success, before reply construction and with no intervening suspension.
pub(in crate::server) async fn response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    password: &Option<String>,
    handler: Option<&ReinitializeHandler>,
    requester: Requester,
    still_open: impl FnOnce() -> Result<(), Error>,
    on_accepted: impl FnOnce(ReinitializedState),
) -> Apdu {
    let outcome = match handlers::handle_reinitialize_device(&request.service_request, password) {
        Ok(state) => match handler {
            Some(handler) => {
                let context = ReinitializeContext::new(
                    state,
                    requester.source_mac,
                    requester.source_network,
                    requester.provenance,
                    request.invoke_id,
                );
                let mut db = db.write().await;
                still_open()
                    .and_then(|()| invoke(handler, &context, &mut db))
                    .map(|()| state)
            }
            None => Err(services_error(ErrorCode::SERVICE_REQUEST_DENIED)),
        },
        Err(error) => Err(error),
    };
    match outcome {
        Ok(state) => {
            on_accepted(state);
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            })
        }
        Err(error) => error_apdu_from_error(request.invoke_id, request.service_choice, &error),
    }
}

/// Full-server acceptance also owns the DCC restart transition. Acquire the
/// timer slot before the handler's database lock, as DCC replacement/expiry do.
pub(in crate::server) async fn server_response<T: TransportPort + 'static>(
    services: &super::request_services::RequestServices<T>,
    request: &ConfirmedRequestPdu,
    requester: Requester,
) -> Apdu {
    let cov_resume = services.cov_table.read().await.timed().clone();
    let mut slot = services.dcc_timer.lock().await;
    let mut restart_accepted = false;
    let reply = response(
        &services.db,
        request,
        &services.config.reinit_password,
        services.config.on_reinitialize.as_ref(),
        requester,
        || Ok(()),
        |state| {
            if matches!(
                state,
                ReinitializedState::WARMSTART | ReinitializedState::COLDSTART
            ) {
                super::dcc_timer::enable_for_restart(&slot, &services.comm_state, &cov_resume);
                restart_accepted = true;
            }
        },
    )
    .await;
    if restart_accepted {
        // The commit already happened. Cancellation during this borrowed join
        // retains the aborted handle for later replacement/shutdown cleanup.
        super::dcc_timer::cancel(&mut slot).await;
    }
    reply
}

/// Run `handler` under a panic guard. Execution has begun once it is called,
/// so the request may no longer draw a Reject (Clause 20.1.8): a panic, a
/// Reject, or an error with no class and code is answered SERVICES / OTHER.
fn invoke(
    handler: &ReinitializeHandler,
    context: &ReinitializeContext,
    db: &mut ObjectDatabase,
) -> Result<(), Error> {
    match catch_unwind(AssertUnwindSafe(|| handler(context, db))) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error @ (Error::Protocol { .. } | Error::Structured { .. }))) => Err(error),
        Ok(Err(error)) => {
            debug!(%error, "ReinitializeDevice handler error answered SERVICES / OTHER");
            Err(services_error(ErrorCode::OTHER))
        }
        Err(_) => {
            warn!(
                state = context.state.to_raw(),
                invoke_id = context.invoke_id,
                "ReinitializeDevice handler panicked; answered SERVICES / OTHER"
            );
            Err(services_error(ErrorCode::OTHER))
        }
    }
}

fn services_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the ReinitializeDevice handler. It must reply before restarting and
    /// stay quick; see [`ReinitializeHandler`].
    pub fn on_reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.config.on_reinitialize = Some(Arc::new(handler));
        self
    }
}

impl BipServerBuilder {
    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the ReinitializeDevice handler. It must reply before restarting and
    /// stay quick; see [`ReinitializeHandler`].
    pub fn on_reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.config.on_reinitialize = Some(Arc::new(handler));
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the ReinitializeDevice handler. It must reply before restarting and
    /// stay quick; see [`ReinitializeHandler`].
    pub fn on_reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.config.on_reinitialize = Some(Arc::new(handler));
        self
    }
}
