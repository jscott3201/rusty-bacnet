//! Source Audit on a seated client (#2617, Clause 19.6): the confirmed
//! requests of a client whose node has a source Audit Reporter go through
//! that Reporter.
//!
//! Such a client's TSM refuses every registration (its registrar is empty,
//! see `Tsm::divert_registrations`), so each request reaches the client's
//! cold path at the registration test every request already makes. A
//! standalone client, and a seated one on a node with no Reporter, pay
//! nothing for any of this. There the request is handed, with its turn at
//! the destination, to the client's [`SourceAuditor`]: its node. A request
//! of a service Table 19-5 doesn't map to READ or WRITE in this release
//! runs as it is, in the caller's task.
//!
//! The node runs the request through a *runner*: a copy of the client that
//! shares its transaction state, link and discovery table, owns no task,
//! and registers its one transaction through the coordinator itself. A
//! runner starts the request over from the top, so every request goes out
//! through the one request path, segmented ones included. An audited
//! client takes no turn at the destination in its request path: the node
//! takes it as it admits the request (#2652), and the turn then travels
//! with the request, so the runner's own pacer lets it straight through. A
//! routed path the caller held is let go and taken again.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use bacnet_endpoint_core::seat::ClientSeat;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::AuditOperation;

use super::pacing::{PaceGuard, PaceKey, Turn};
use super::segmentation_context::SegmentedRequestLimits;
use super::source_audit_targets::{self, AuditTarget, AuditedOutcome, Decoded};
use super::*;
use crate::tsm::{CoordinatedRegistrationError, TransactionRegistration};

/// A node's source Audit, as its seated client hands it the requests its
/// Reporter may cover (#2617). The endpoint session implements it.
#[doc(hidden)]
pub trait SourceAuditor: Send + Sync + 'static {
    /// Carry out `request` and return what the request itself returns:
    /// audited, in a task the node owns, when the Reporter's settings
    /// select one of its targets, and as it is, in the caller's task,
    /// otherwise.
    fn audit(
        &self,
        request: AuditedRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Bytes, Error>> + Send>>;
}

/// A seated client's part in its node's source Audit (#2617), which its
/// pacer holds.
pub(super) enum Role {
    /// The client a caller holds, whose requests its node carries out.
    Audited(Arc<dyn SourceAuditor>),
    /// A copy of that client running one request for the node. It
    /// registers through the coordinator itself and notes the invoke ID.
    Runner(Arc<OnceLock<u8>>),
}

impl Role {
    pub(super) fn is_runner(&self) -> bool {
        matches!(self, Self::Runner(_))
    }
}

impl std::fmt::Debug for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audited(_) => f.write_str("Audited"),
            Self::Runner(invoke_id) => f.debug_tuple("Runner").field(&invoke_id.get()).finish(),
        }
    }
}

/// What a refused registration comes to.
pub(super) enum Refused {
    /// A runner's registration, made through the coordinator.
    Registered((u8, TransactionRegistration)),
    /// An audited client's request, which goes to its node.
    Audited(Arc<dyn SourceAuditor>),
}

/// The request, not yet started: its result and the invoke ID it went with.
type Run = Pin<Box<dyn Future<Output = (Result<Bytes, Error>, Option<u8>)> + Send>>;

/// A diverted request's end, as its caller awaits it.
pub(super) type Diverted = Pin<Box<dyn Future<Output = Result<Bytes, Error>> + Send>>;

/// A confirmed request a seated client hands its node's source Audit
/// (#2617): what it asks for, where it goes, and the request itself, which
/// nothing has sent yet. Its turn at the destination, once taken, goes
/// with it and ends when it does.
#[doc(hidden)]
pub struct AuditedRequest {
    operation: AuditOperation,
    targets: Vec<AuditTarget>,
    station: BACnetAddress,
    decoded: Decoded,
    run: Run,
    /// The client's transaction state, for the node's
    /// DeviceCommunicationControl state at admission.
    tsm: Arc<SyncMutex<Tsm>>,
    /// Its turn at the destination, once taken.
    pace: PaceGuard,
    /// The turn still to take, when the client paces its requests.
    turn: Option<Turn>,
}

impl AuditedRequest {
    /// The operation it requests: READ or WRITE (Table 19-5).
    pub fn operation(&self) -> AuditOperation {
        self.operation
    }

    /// Its targets, in the request's order (Clause 19.6.5).
    pub fn targets(&self) -> &[AuditTarget] {
        &self.targets
    }

    /// The station it goes to: network 0 for one on this link, which a
    /// station on this client's own network is once the number is known.
    pub fn station(&self) -> &BACnetAddress {
        &self.station
    }

    /// Whether the node's DeviceCommunicationControl restricts this
    /// client's initiation now (#2617): a request admitted while it does
    /// would be refused when it registers, so a node refuses it first.
    pub fn initiation_restricted(&self) -> bool {
        self.tsm.lock().initiation_restricted()
    }

    /// Wait until the request's turn at its destination
    /// (`min_request_interval_ms`) is due, without taking it: a node waits
    /// for that before it admits the request, holding nothing, and then
    /// takes the turn with [`Self::try_take_turn`] (#2652). At once when the
    /// client doesn't pace or the turn is taken.
    ///
    /// # Errors
    ///
    /// [`Error::RequestCutOff`] with `sent: false` when the client's tasks
    /// end first: its stop, or its node's stop or drop (#2720).
    pub async fn turn_due(&mut self) -> Result<(), Error> {
        match &self.turn {
            Some(turn) if !turn.due().await => Err(super::requests::cut_off(false)),
            _ => Ok(()),
        }
    }

    /// Take the request's turn at its destination if it is due now, as a
    /// node does as it admits the request, so the interval holds between
    /// requests as they are admitted, however long each waited. `false`
    /// when another request to the destination took it since
    /// [`Self::turn_due`]; always `true` once taken, or when the client
    /// doesn't pace. It takes only the client's pacer lock.
    pub fn try_take_turn(&mut self) -> bool {
        let Some(turn) = &self.turn else {
            return true;
        };
        let Some(pace) = turn.try_take() else {
            return false;
        };
        self.pace = pace;
        self.turn = None;
        true
    }

    /// Send the request, through the one request path every request takes,
    /// and wait for its end: first for its turn at the destination, if
    /// [`Self::try_take_turn`] hasn't taken it. Dropping the future before
    /// the end cancels the request as dropping a request does.
    pub async fn execute(mut self) -> AuditedOutcome {
        if let Some(turn) = self.turn.take() {
            let Some(pace) = turn.take().await else {
                return AuditedOutcome {
                    result: Err(super::requests::cut_off(false)),
                    invoke_id: None,
                    attempted: false,
                    records: Vec::new(),
                    target_device: None,
                };
            };
            self.pace = pace;
        }
        let Self {
            targets,
            decoded,
            run,
            pace,
            ..
        } = self;
        let (result, invoke_id) = run.await;
        drop(pace);
        let attempted = invoke_id.is_some() && source_audit_targets::attempted(&result);
        let (records, target_device) = if attempted {
            decoded.records(targets, &result)
        } else {
            (Vec::new(), None)
        };
        AuditedOutcome {
            result,
            invoke_id,
            attempted,
            records,
            target_device,
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// [`Self::start_on_seat`] on a node whose source Audit Reporter covers
    /// the client's requests (#2617): each confirmed request of a service
    /// Table 19-5 maps to READ or WRITE goes to `auditor`. See the module
    /// documentation for how.
    #[doc(hidden)]
    pub fn start_on_audited_seat(
        config: ClientConfig,
        seat: ClientSeat<T>,
        options: ClientOptions,
        auditor: Arc<dyn SourceAuditor>,
    ) -> Result<Self, Error> {
        let client = Self::start_on_seat(config, seat, options)?;
        client.tsm.lock().divert_registrations();
        let _ = client.pacer.audit.set(Role::Audited(auditor));
        Ok(client)
    }

    /// The TSM refused to register a request to `target`: its seat has
    /// closed, the node's DeviceCommunicationControl restricts initiation,
    /// or the client's requests are its node's to audit (#2617). Only the
    /// last goes on: a runner registers through the coordinator here, and
    /// an audited client's request goes to its node. The others fail as
    /// every seated client's request does, a closed seat as its sends do
    /// and the restriction with [`Error::InitiationDisabled`], with nothing
    /// sent and before an audited request takes a slot or a sequence
    /// number.
    #[cold]
    #[inline(never)]
    pub(super) fn on_refused(
        &self,
        refused: CoordinatedRegistrationError,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        segmented: bool,
    ) -> Result<Refused, Error> {
        match refused {
            CoordinatedRegistrationError::Diverted => {}
            CoordinatedRegistrationError::CoordinatorUnavailable => {
                return Err(super::seat::seat_closed());
            }
            refused => return Err(Error::from(refused)),
        }
        match self.pacer.audit.get() {
            Some(Role::Runner(invoke_id)) => {
                let peer = target.transaction_peer();
                let registered = self
                    .tsm
                    .lock()
                    .register_diverted(peer.tsm_mac, peer.canonical, service_choice, segmented)
                    .map_err(Error::from)?;
                let _ = invoke_id.set(registered.0);
                Ok(Refused::Registered(registered))
            }
            Some(Role::Audited(auditor)) => Ok(Refused::Audited(Arc::clone(auditor))),
            None => Err(Error::from(CoordinatedRegistrationError::Diverted)),
        }
    }

    /// Hand a request whose registration was diverted to the node's source
    /// Audit (#2617). `named` is the target as the caller named it, which
    /// the runner starts from again. The caller has taken no turn at the
    /// destination: the node takes it as it admits the request (#2652), and
    /// the request keeps it until it ends. A routed path the caller took is
    /// the caller's to let go first, and the runner takes it again. The
    /// runner's frames carry `attributes`, the caller's (#2723).
    ///
    /// The future awaits the request's end, owns everything it needs, and
    /// is boxed and type-erased: the request path it runs ends here again.
    ///
    /// # Errors
    ///
    /// The request's own errors, and the node's refusal. A request of a
    /// service the node audits that doesn't decode is refused before
    /// anything is sent.
    #[cold]
    #[inline(never)]
    pub(super) fn audited_request(
        &self,
        auditor: Arc<dyn SourceAuditor>,
        named: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        attributes: Attributes<'_>,
    ) -> Diverted {
        // A seat that has closed, by the node's stop or the client's own,
        // takes nothing more: refused at once, before the node is asked.
        if let Err(error) = self.seat_open() {
            return Box::pin(std::future::ready(Err(error)));
        }
        // Where it goes, as the request path localizes it: a station on
        // this client's own network is one on this link (#1358).
        let to = named.localized(self.network.local_network_number().get());
        let audited = match source_audit_targets::decode(service_choice, service_data) {
            Some(Err(error)) => return Box::pin(std::future::ready(Err(error))),
            audited => audited.and_then(Result::ok),
        };
        let (runner, invoke_id) = self.runner();
        let station = Station::of(named);
        let data = Bytes::copy_from_slice(service_data);
        // The node runs the request in a task of its own, so it owns the
        // data attributes its caller made it with (#2723).
        let attributes = attributes.cloned();
        let run: Run = Box::pin(async move {
            let result = runner
                .confirmed_request_with(&station, service_choice, &data, attributes.as_ref())
                .await;
            (result, invoke_id.get().copied())
        });
        // The turn it takes on admission (#2652): on the caller's pacer, by
        // the key the request path would have used.
        let turn = (self.pacer.paces())
            .then(|| Turn::new(&self.pacer, PaceKey::of(to), &self.routing_query.ended));
        let Some((operation, targets, decoded)) = audited else {
            // Not the node's to audit: it runs as it is, in the caller's
            // task, once it has its turn.
            return Box::pin(async move {
                let _pace = match &turn {
                    Some(turn) => turn
                        .take()
                        .await
                        .ok_or_else(|| super::requests::cut_off(false))?,
                    None => PaceGuard::unpaced(),
                };
                run.await.0
            });
        };
        let (network_number, mac) = to.station();
        let station = BACnetAddress {
            network_number,
            mac_address: MacAddr::from_slice(mac),
        };
        auditor.audit(AuditedRequest {
            operation,
            targets,
            station,
            decoded,
            run,
            tsm: Arc::clone(&self.tsm),
            pace: PaceGuard::unpaced(),
            turn,
        })
    }

    /// [`Self::audited_request`] for a request diverted where it was about
    /// to go segmented. Its routed path is its caller's, reached through
    /// `limits`, and the caller ends once this does.
    #[cold]
    #[inline(never)]
    pub(super) fn audited_segmented(
        &self,
        auditor: Arc<dyn SourceAuditor>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        limits: &mut SegmentedRequestLimits<'_>,
    ) -> Diverted {
        drop(limits.routed_path_lease.take());
        self.audited_request(
            auditor,
            *limits.named,
            service_choice,
            service_data,
            limits.attributes,
        )
    }

    /// A copy of this client that runs one request for its node: the same
    /// configuration, link, transaction state, discovery table and routed
    /// paths, no task of its own, and a pacer that never waits, since the
    /// request it runs has had its turn. Dropping it ends nothing but its
    /// own request. The cell takes the invoke ID its request registers.
    fn runner(&self) -> (Self, Arc<OnceLock<u8>>) {
        let invoke_id = Arc::new(OnceLock::new());
        let pacer = pacing::RequestPacer::new(Duration::ZERO);
        let _ = pacer.audit.set(Role::Runner(Arc::clone(&invoke_id)));
        let runner = Self {
            config: self.config.clone(),
            network: Arc::clone(&self.network),
            tsm: Arc::clone(&self.tsm),
            device_table: Arc::clone(&self.device_table),
            cov_tx: self.cov_tx.clone(),
            event_tx: self.event_tx.clone(),
            device_tx: self.device_tx.clone(),
            device_collision_tx: self.device_collision_tx.clone(),
            i_have_tx: self.i_have_tx.clone(),
            network_report_tx: self.network_report_tx.clone(),
            routing_query: Arc::clone(&self.routing_query),
            dispatch_task: None,
            network_number_task: None,
            device_purge_task: None,
            seg_ack_senders: Arc::clone(&self.seg_ack_senders),
            cleanup_tx: self.cleanup_tx.clone(),
            #[cfg(test)]
            segmented_post_wait_cleanup: Arc::clone(&self.segmented_post_wait_cleanup),
            #[cfg(test)]
            segmented_cleanup: Arc::clone(&self.segmented_cleanup),
            #[cfg(test)]
            decode_warning: Arc::clone(&self.decode_warning),
            #[cfg(test)]
            response_warning: Arc::clone(&self.response_warning),
            local_mac: self.local_mac.clone(),
            routed_path_limits: Arc::clone(&self.routed_path_limits),
            group_source_request_drops: Arc::clone(&self.group_source_request_drops),
            pacer: Arc::new(pacer),
        };
        (runner, invoke_id)
    }
}

impl Station {
    /// The station a request target names, as an owned [`Station`].
    fn of(target: ConfirmedTarget<'_>) -> Self {
        match target {
            ConfirmedTarget::Local { mac } => Self::local(mac),
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
                one_station,
            } => {
                let station = if router_mac == super::station::VIA_LOCAL_BROADCAST {
                    Self::routed_via_local_broadcast(dest_network, dest_mac)
                } else {
                    Self::routed(router_mac, dest_network, dest_mac)
                };
                if one_station {
                    station.one_station()
                } else {
                    station
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "source_audit_tests.rs"]
mod tests;
