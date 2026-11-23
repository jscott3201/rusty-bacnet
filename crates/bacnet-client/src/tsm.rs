//! Transaction State Machine (TSM) per ASHRAE 135-2020 Clause 5.4.
//!
//! Tracks in-flight confirmed requests. Each request gets a unique invoke_id
//! (0-255) scoped per destination MAC. Responses are delivered via oneshot channels.

use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::RemoteError;
use bacnet_types::map_hash::BuildMapHasher;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

mod ack_form;
mod final_segment;
#[cfg(test)]
pub(crate) use final_segment::TerminalResponseAdmission;
pub(crate) use final_segment::{FinalSegmentIssue, FinalSegmentSendToken};
mod segmented_response;
pub(crate) use segmented_response::SegmentedResponseAdmission;
mod completion;
mod coordinated;
mod progress;
pub(crate) use coordinated::{
    CoordinatedCompletion, CoordinatedRegistrationError, CoordinatedTerminalPhase,
    SegmentedAckArrival,
};
use coordinated::{PendingLease, PendingRelease};
use progress::TransactionShared;
pub(crate) use progress::{ProgressReceiver, TransactionProgress};

/// TSM configuration.
#[derive(Debug, Clone)]
pub struct TsmConfig {
    /// APDU timeout in milliseconds (default 6000).
    pub apdu_timeout_ms: u64,
    /// APDU segment timeout in milliseconds (default = apdu_timeout_ms).
    pub apdu_segment_timeout_ms: u64,
    /// Number of APDU retries (default 3).
    pub apdu_retries: u8,
    /// See [`ClientConfig::accept_complex_ack_for_simple_services`](crate::client::ClientConfig::accept_complex_ack_for_simple_services)
    /// (default `false`).
    pub accept_complex_ack_for_simple_services: bool,
}

impl Default for TsmConfig {
    fn default() -> Self {
        Self {
            apdu_timeout_ms: 6000,
            apdu_segment_timeout_ms: 6000,
            apdu_retries: 3,
            accept_complex_ack_for_simple_services: false,
        }
    }
}

/// Response types that complete a transaction.
///
/// Non-exhaustive: the TSM gains completion reasons as more of Clause 5.4's
/// state machine is implemented, and those should be additive for callers.
#[derive(Debug)]
#[non_exhaustive]
pub enum TsmResponse {
    /// SimpleACK — confirmed service completed with no return data.
    SimpleAck,
    /// ComplexACK — confirmed service returned data.
    ComplexAck {
        /// Encoded service ACK parameters, without the APDU header.
        service_data: Bytes,
    },
    /// An Error, Reject or Abort PDU: the peer turned the request down.
    Remote {
        /// What the PDU reports.
        error: RemoteError,
        /// What an Error PDU's structured Clause 21 body adds to the class
        /// and code (ChangeList-Error, CreateObject-Error, WPM-Error and the
        /// others); `None` for a plain error, a Reject or an Abort.
        detail: Option<bacnet_types::error::ErrorDetail>,
    },
    /// The transaction ended without an answer the request can use, and
    /// this is the error it fails with: the router the request went through
    /// refused it, as too long for the routed path
    /// ([`Error::RoutedPathTooLong`](bacnet_types::error::Error::RoutedPathTooLong))
    /// or for another reason that ends a request at once
    /// ([`Error::NetworkRejected`](bacnet_types::error::Error::NetworkRejected),
    /// #2724), or the device acknowledged it with the other kind of
    /// acknowledgement than its service takes
    /// ([`Error::UnexpectedAck`](bacnet_types::error::Error::UnexpectedAck),
    /// #2721). One boxed variant for these rare endings keeps every
    /// completion's size and variant tags as they were.
    Failed(Box<bacnet_types::error::Error>),
}

impl TsmResponse {
    /// The completion an Error PDU produces. A structured body keeps its
    /// detail, such as a ChangeList-Error's First Failed Element Number or
    /// WritePropertyMultiple's first failed write attempt.
    pub(crate) fn from_error_pdu(pdu: &bacnet_encoding::apdu::ErrorPdu) -> Self {
        Self::Remote {
            error: pdu.remote_error(),
            detail: bacnet_services::structured_error::detail(pdu),
        }
    }

    /// The completion a Reject or Abort PDU produces, or an Error PDU
    /// without a structured body: `error` without a detail.
    pub const fn remote(error: RemoteError) -> Self {
        Self::Remote {
            error,
            detail: None,
        }
    }

    /// What a request ends with: an acknowledgement's service data (empty
    /// for a Simple-ACK), or the error the completion reports. Each arm
    /// takes all its variant holds and returns or hands off in a tail call,
    /// so an acknowledgement's ending needs no stack frame and no check of
    /// what is left to drop.
    pub(crate) fn into_result(self) -> Result<Bytes, bacnet_types::error::Error> {
        match self {
            Self::SimpleAck => Ok(Bytes::new()),
            Self::ComplexAck { service_data } => Ok(service_data),
            Self::Remote { error, detail } => remote_failure(error, detail),
            Self::Failed(error) => failed(error),
        }
    }
}

/// The error an Error, Reject or Abort PDU's completion ends with: an
/// Error PDU's class and code with any structured detail; a Reject or
/// Abort, which has no detail, as itself.
#[inline(never)]
fn remote_failure(
    error: RemoteError,
    detail: Option<bacnet_types::error::ErrorDetail>,
) -> Result<Bytes, bacnet_types::error::Error> {
    use bacnet_types::error::Error;
    match error {
        RemoteError::Error { class, code } => Err(Error::with_detail(class, code, detail)),
        _ => Err(Error::Remote(error)),
    }
}

/// The error a [`TsmResponse::Failed`] holds, taken out of its box here
/// rather than in [`TsmResponse::into_result`], which only hands it off.
#[cold]
#[inline(never)]
#[allow(clippy::boxed_local)] // unboxing out of line is this function's job
fn failed(error: Box<bacnet_types::error::Error>) -> Result<Bytes, bacnet_types::error::Error> {
    Err(*error)
}

/// Invoke ID allocator scoped to a single destination MAC.
struct InvokeIdAllocator {
    next_id: u8,
    in_use: [bool; 256],
}

impl InvokeIdAllocator {
    fn new() -> Self {
        Self {
            next_id: 0,
            in_use: [false; 256],
        }
    }

    fn allocate(&mut self) -> Option<u8> {
        let start = self.next_id;
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.in_use[id as usize] {
                self.in_use[id as usize] = true;
                return Some(id);
            }
            if self.next_id == start {
                return None;
            }
        }
    }

    fn release(&mut self, id: u8) {
        self.in_use[id as usize] = false;
    }

    fn all_free(&self) -> bool {
        !self.in_use.iter().any(|&used| used)
    }
}

/// Maximum number of distinct destination MACs tracked by the TSM.
/// Prevents unbounded memory growth from spoofed source addresses.
const MAX_TSM_DESTINATIONS: usize = 1024;

/// What `complete_transaction` did with a response.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionOutcome {
    /// The response matched a pending transaction and was delivered.
    Delivered,
    /// No transaction was pending for this source and invoke ID.
    NoTransaction,
    /// A transaction was pending, but the response was labelled for a
    /// different confirmed service. The transaction is left pending and the
    /// invoke ID stays allocated, so the legitimate response can still arrive.
    ServiceChoiceMismatch {
        /// The service the pending request asked for.
        expected: ConfirmedServiceChoice,
        /// The service the response claimed to answer.
        observed: ConfirmedServiceChoice,
    },
}

#[derive(Debug)]
pub(crate) enum SegmentAckPhase {
    SegmentedRequest(TransactionOwner),
    Outstanding,
    CoordinatorRejected,
    Idle,
}

pub(crate) struct TransactionRegistration {
    pub(crate) response: oneshot::Receiver<TsmResponse>,
    pub(crate) progress: ProgressReceiver,
    pub(crate) owner: TransactionOwner,
}

/// Identity of one registration, independent of its reusable wire key.
///
/// Pointer identity is stable while delayed work retains a clone. The
/// allocation therefore cannot be reused for a replacement transaction until
/// every stale owner reference has gone away.
///
/// The same allocation carries the request-side timer progress (#1727), so
/// a transaction costs one shared allocation rather than two.
#[derive(Clone, Debug)]
pub(crate) struct TransactionOwner(Arc<TransactionShared>);

impl TransactionOwner {
    fn new() -> Self {
        Self(Arc::new(TransactionShared::new()))
    }

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Send every frame of this transaction's request with `attributes`,
    /// when it was made with some (#2723); set before the first frame goes.
    #[inline]
    pub(crate) fn set_data_attributes(
        &self,
        attributes: Option<&Arc<[bacnet_transport::port::DataAttribute]>>,
    ) {
        if let Some(attributes) = attributes {
            self.0.set_data_attributes(Arc::clone(attributes));
        }
    }

    /// The data attributes this transaction's frames carry: empty unless
    /// its request was made with some.
    #[inline]
    pub(crate) fn data_attributes(&self) -> &[bacnet_transport::port::DataAttribute] {
        self.0.data_attributes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestTimerExpiration {
    Retry,
    /// A retry was due while the node's DeviceCommunicationControl
    /// restricts initiation (#2617): the transaction ended, its invoke ID
    /// released, and nothing more is sent.
    Withdrawn,
    SegmentedResponse {
        generation: u64,
    },
    TimedOut,
    NoTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentTimerExpiration {
    Activity { generation: u64 },
    AwaitingResponse,
    TimedOut,
    NoTransaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionPhase {
    AwaitingResponse,
    SegmentedRequest { sent_all_segments: bool },
    SegmentedResponse,
}

/// A confirmed request awaiting its response.
struct PendingTransaction {
    responder: oneshot::Sender<TsmResponse>,
    /// Also publishes this transaction's timer progress.
    owner: TransactionOwner,
    phase: TransactionPhase,
    final_segment_issue: Option<FinalSegmentIssue>,
    /// Monotonic token used to reject a SegmentTimer expiry observed before
    /// newer segment activity acquired the TSM lock.
    segment_generation: u64,
    /// The service this request asked for. Clause 20.1.4.2 and 20.1.5.6 both
    /// require service-ack-choice to match the BACnetConfirmedServiceChoice
    /// of the request being acknowledged. A different choice therefore
    /// cannot identify this transaction's response.
    expected_service_choice: ConfirmedServiceChoice,
    lease: PendingLease,
}

/// Transaction State Machine.
///
/// Tracks pending confirmed requests and correlates responses by
/// `(destination_mac, invoke_id)` and by the confirmed service each request
/// asked for.
pub struct Tsm {
    config: TsmConfig,
    // Only this client inserts: see bacnet_types::map_hash (#1713).
    allocators: HashMap<MacAddr, InvokeIdAllocator, BuildMapHasher>,
    pending: HashMap<(MacAddr, u8), PendingTransaction, BuildMapHasher>,
    coordinator: Option<Arc<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>>,
    /// The coordinator a new transaction leases from: `coordinator` itself,
    /// except while the node this client is seated on restricts initiation
    /// (DeviceCommunicationControl), while the client's requests are its
    /// node's to audit (source Audit), and once its seat has closed (#2617).
    /// Registration reads only this, so neither costs a request anything it
    /// didn't already check; `restricted` and `diverted` say, on the cold
    /// path of a refused registration, which emptied it.
    leasing: Option<Arc<bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator>>,
    /// The node restricts initiation (DeviceCommunicationControl).
    restricted: bool,
    /// The client's registrations go to its node's source Audit.
    diverted: bool,
    /// The task reassembling the segmented requests this client answers,
    /// once the first came (#2646). Only the cold branches of the dispatch,
    /// `stop`, the drop and a closing seat touch it.
    pub(crate) request_reassembly:
        Option<Box<crate::client::request_reassembly::RequestReassembly>>,
}

impl Tsm {
    /// The segment timeout this client's transfers use, which the
    /// reassembly of segmented requests takes when it starts (#2646).
    pub(crate) fn segment_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.config.apdu_segment_timeout_ms)
    }

    /// Create a transaction state machine with the given timing and retry configuration.
    pub fn new(config: TsmConfig) -> Self {
        Self {
            config,
            allocators: HashMap::default(),
            pending: HashMap::default(),
            coordinator: None,
            leasing: None,
            restricted: false,
            diverted: false,
            request_reassembly: None,
        }
    }

    /// Timing and retry configuration in force.
    pub fn config(&self) -> &TsmConfig {
        &self.config
    }

    /// Allocate an invoke ID for the given destination MAC.
    /// Returns `None` if all 256 IDs are in use for this destination,
    /// or if the maximum number of tracked destinations has been reached.
    pub fn allocate_invoke_id(&mut self, destination_mac: &[u8]) -> Option<u8> {
        let key = MacAddr::from_slice(destination_mac);
        if !self.allocators.contains_key(&key) && self.allocators.len() >= MAX_TSM_DESTINATIONS {
            return None;
        }
        let allocator = self
            .allocators
            .entry(key)
            .or_insert_with(InvokeIdAllocator::new);
        allocator.allocate()
    }

    /// Release an invoke ID back to the pool for the given destination.
    /// Removes the allocator entry if all IDs are now free (prevents unbounded growth).
    pub fn release_invoke_id(&mut self, destination_mac: &[u8], invoke_id: u8) {
        let key = MacAddr::from_slice(destination_mac);
        if let Some(allocator) = self.allocators.get_mut(&key) {
            allocator.release(invoke_id);
            if allocator.all_free() {
                self.allocators.remove(&key);
            }
        }
    }

    /// Register a pending transaction. Returns a receiver that will deliver
    /// the response when it arrives.
    ///
    /// `service_choice` is the confirmed service being requested; a response
    /// labelled for any other service will not complete this transaction.
    pub fn register_transaction(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
    ) -> oneshot::Receiver<TsmResponse> {
        self.register_transaction_with_progress(destination_mac, invoke_id, service_choice)
            .response
    }

    pub(crate) fn register_transaction_with_progress(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
    ) -> TransactionRegistration {
        self.register_transaction_in_phase(
            destination_mac,
            invoke_id,
            service_choice,
            TransactionPhase::AwaitingResponse,
            PendingLease::Legacy,
        )
    }

    #[cfg(test)]
    pub(crate) fn register_segmented_transaction_with_progress(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
    ) -> TransactionRegistration {
        self.register_transaction_in_phase(
            destination_mac,
            invoke_id,
            service_choice,
            TransactionPhase::SegmentedRequest {
                sent_all_segments: false,
            },
            PendingLease::Legacy,
        )
    }

    fn register_transaction_in_phase(
        &mut self,
        destination_mac: MacAddr,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        phase: TransactionPhase,
        lease: PendingLease,
    ) -> TransactionRegistration {
        let (pending, registration) = Self::new_pending_transaction(service_choice, phase, lease);
        debug_assert!(
            !self
                .pending
                .contains_key(&(destination_mac.clone(), invoke_id)),
            "duplicate TSM registration for invoke_id {}",
            invoke_id
        );
        self.pending.insert((destination_mac, invoke_id), pending);
        registration
    }

    // Inlined into each registration, as it was when one body registered
    // through the coordinator: a second, the diverted one (#2617), made the
    // compiler call it instead, about 20 instructions on every request.
    #[inline(always)]
    fn new_pending_transaction(
        service_choice: ConfirmedServiceChoice,
        phase: TransactionPhase,
        lease: PendingLease,
    ) -> (PendingTransaction, TransactionRegistration) {
        let (tx, rx) = oneshot::channel();
        let owner = TransactionOwner::new();
        let final_segment_issue =
            matches!(phase, TransactionPhase::SegmentedRequest { .. }).then(FinalSegmentIssue::new);
        (
            PendingTransaction {
                responder: tx,
                owner: owner.clone(),
                phase,
                final_segment_issue,
                segment_generation: 0,
                expected_service_choice: service_choice,
                lease,
            },
            TransactionRegistration {
                response: rx,
                progress: ProgressReceiver::new(owner.clone()),
                owner,
            },
        )
    }

    /// Reserve the first poll of the final N-UNITDATA.request. A terminal PDU
    /// that reaches dispatch while this token is live waits for that poll to
    /// resolve instead of being admitted or rejected early.
    pub(crate) fn begin_final_segment_send(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> Option<FinalSegmentSendToken> {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let pending = self.pending.get(&key)?;
        if !pending.owner.same_as(owner)
            || !matches!(
                pending.phase,
                TransactionPhase::SegmentedRequest {
                    sent_all_segments: false
                }
            )
        {
            return None;
        }
        let issue = pending.final_segment_issue.as_ref()?.clone();
        issue.begin().then_some(FinalSegmentSendToken {
            issue,
            resolved: false,
        })
    }

    /// Publish `SentAllSegments` after the final send future has been polled.
    /// The caller must not hold the TSM lock while performing that poll.
    pub(crate) fn mark_final_segment_issued(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        token: &mut FinalSegmentSendToken,
    ) -> bool {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get_mut(&key) else {
            return false;
        };
        if !pending.owner.same_as(owner) {
            return false;
        }
        let TransactionPhase::SegmentedRequest { sent_all_segments } = &mut pending.phase else {
            return false;
        };
        if !pending
            .final_segment_issue
            .as_ref()
            .is_some_and(|issue| issue.same_as(&token.issue))
            || !token.issue.is_polling()
        {
            return false;
        }
        *sent_all_segments = true;
        token.issued();
        true
    }

    /// FinalACK_Received moves the same owner into AWAIT_CONFIRMATION.
    pub(crate) fn finish_segmented_request(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> bool {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get_mut(&key) else {
            return false;
        };
        if !pending.owner.same_as(owner)
            || !matches!(
                pending.phase,
                TransactionPhase::SegmentedRequest {
                    sent_all_segments: true
                }
            )
        {
            return false;
        }
        pending.phase = TransactionPhase::AwaitingResponse;
        pending
            .owner
            .publish_progress(TransactionProgress::AwaitingResponse);
        true
    }

    pub(crate) fn segment_ack_phase(&self, source_mac: &[u8], invoke_id: u8) -> SegmentAckPhase {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        match self.pending.get(&key) {
            Some(pending) if matches!(pending.phase, TransactionPhase::SegmentedRequest { .. }) => {
                SegmentAckPhase::SegmentedRequest(pending.owner.clone())
            }
            Some(_) => SegmentAckPhase::Outstanding,
            None => SegmentAckPhase::Idle,
        }
    }

    #[cfg(test)]
    pub(crate) fn admit_terminal_response(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: Option<&TransactionOwner>,
    ) -> TerminalResponseAdmission {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return TerminalResponseAdmission::NoTransaction;
        };
        if owner.is_some_and(|owner| !pending.owner.same_as(owner)) {
            return TerminalResponseAdmission::NoTransaction;
        }
        let current_owner = pending.owner.clone();
        if matches!(
            pending.phase,
            TransactionPhase::SegmentedRequest {
                sent_all_segments: false
            }
        ) {
            if pending
                .final_segment_issue
                .as_ref()
                .is_some_and(|issue| issue.is_polling())
            {
                return TerminalResponseAdmission::FinalSegmentSendPolling;
            }
            self.abort_invalid_apdu_in_current_state(source_mac, invoke_id, &current_owner);
            return TerminalResponseAdmission::PrematureSegmentedRequestAborted;
        }
        TerminalResponseAdmission::Active(current_owner)
    }

    /// Enter SEGMENTED_CONF after the first segment has been saved.
    ///
    /// The transition and RequestTimer retry authorization use the same TSM
    /// lock. Whichever changes or observes the phase first wins that decision;
    /// an authorized retry performs transport I/O only after releasing it.
    pub(crate) fn begin_segmented_response(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> Option<u64> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let pending = self.pending.get_mut(&key)?;
        if !pending.owner.same_as(owner) {
            return None;
        }
        if matches!(
            pending.phase,
            TransactionPhase::SegmentedRequest {
                sent_all_segments: false
            }
        ) {
            return None;
        }
        pending.segment_generation = pending.segment_generation.wrapping_add(1);
        pending.phase = TransactionPhase::SegmentedResponse;
        let generation = pending.segment_generation;
        pending
            .owner
            .publish_progress(TransactionProgress::SegmentedResponse { generation });
        Some(generation)
    }

    /// Restart SegmentTimer for a segment handled in SEGMENTED_CONF.
    pub(crate) fn record_segmented_response_activity(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> Option<u64> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let pending = self.pending.get_mut(&key)?;
        if !pending.owner.same_as(owner) || pending.phase != TransactionPhase::SegmentedResponse {
            return None;
        }
        pending.segment_generation = pending.segment_generation.wrapping_add(1);
        let generation = pending.segment_generation;
        pending
            .owner
            .publish_progress(TransactionProgress::SegmentedResponse { generation });
        Some(generation)
    }

    /// Resume AWAIT_CONFIRMATION when a completed reassembly is not this
    /// transaction's response or cannot be delivered.
    pub(crate) fn reset_segmented_response(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> bool {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        let Some(pending) = self.pending.get_mut(&key) else {
            return false;
        };
        if !pending.owner.same_as(owner) || pending.phase != TransactionPhase::SegmentedResponse {
            return false;
        }
        pending.phase = TransactionPhase::AwaitingResponse;
        pending
            .owner
            .publish_progress(TransactionProgress::AwaitingResponse);
        true
    }

    /// Arbitrate RequestTimer against the receive-side phase transition.
    ///
    /// Returning [`RequestTimerExpiration::Retry`] authorizes one retry. The
    /// caller then releases the enclosing TSM lock before transport I/O; a
    /// segmented-response transition that follows does not revoke that send.
    /// While the node this client is seated on restricts initiation, a retry
    /// is not authorized: the transaction ends with
    /// [`RequestTimerExpiration::Withdrawn`] (#2617), as the server's own
    /// confirmed notifications do.
    pub(crate) fn expire_request_timer(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        final_timeout: bool,
    ) -> RequestTimerExpiration {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return RequestTimerExpiration::NoTransaction;
        };
        if !pending.owner.same_as(owner) {
            return RequestTimerExpiration::NoTransaction;
        }
        if pending.phase == TransactionPhase::SegmentedResponse {
            return RequestTimerExpiration::SegmentedResponse {
                generation: pending.segment_generation,
            };
        }
        if !final_timeout && !self.initiation_restricted() {
            return RequestTimerExpiration::Retry;
        }
        let withdrawn = !final_timeout;
        let pending = self
            .pending
            .remove(&key)
            .expect("pending transaction exists");
        self.release_pending(destination_mac, invoke_id, pending, PendingRelease::Release);
        if withdrawn {
            RequestTimerExpiration::Withdrawn
        } else {
            RequestTimerExpiration::TimedOut
        }
    }

    /// Cancel only if no segment activity has advanced past `generation`.
    pub(crate) fn expire_segment_timer(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
        generation: u64,
    ) -> SegmentTimerExpiration {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        let Some(pending) = self.pending.get(&key) else {
            return SegmentTimerExpiration::NoTransaction;
        };
        if !pending.owner.same_as(owner) {
            return SegmentTimerExpiration::NoTransaction;
        }
        if pending.phase == TransactionPhase::AwaitingResponse {
            return SegmentTimerExpiration::AwaitingResponse;
        }
        if pending.segment_generation != generation {
            return SegmentTimerExpiration::Activity {
                generation: pending.segment_generation,
            };
        }
        let pending = self
            .pending
            .remove(&key)
            .expect("pending transaction exists");
        self.release_pending(destination_mac, invoke_id, pending, PendingRelease::Release);
        SegmentTimerExpiration::TimedOut
    }

    /// The confirmed service a pending transaction is waiting on, if any.
    ///
    /// Doubles as the "is a transaction pending" predicate: an inbound
    /// segmented response for an invoke ID nobody is waiting on should not be
    /// allocated a reassembly session.
    pub fn expected_service_choice(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
    ) -> Option<ConfirmedServiceChoice> {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        self.pending.get(&key).map(|p| p.expected_service_choice)
    }

    pub(crate) fn owner_is_current(
        &self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> bool {
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        self.pending
            .get(&key)
            .is_some_and(|pending| pending.owner.same_as(owner))
    }

    fn abort_invalid_apdu_in_current_state(
        &mut self,
        source_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) {
        let _ = self.complete_transaction_for_owner(
            source_mac,
            invoke_id,
            owner,
            None,
            TsmResponse::remote(RemoteError::Abort {
                reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
            }),
        );
    }

    /// Cancel a pending transaction. Returns `true` if found.
    pub fn cancel_transaction(&mut self, destination_mac: &[u8], invoke_id: u8) -> bool {
        self.cancel_transaction_inner(destination_mac, invoke_id, None)
    }

    pub(crate) fn cancel_transaction_for_owner(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: &TransactionOwner,
    ) -> bool {
        self.cancel_transaction_inner(destination_mac, invoke_id, Some(owner))
    }

    fn cancel_transaction_inner(
        &mut self,
        destination_mac: &[u8],
        invoke_id: u8,
        owner: Option<&TransactionOwner>,
    ) -> bool {
        let key = (MacAddr::from_slice(destination_mac), invoke_id);
        if owner.is_some_and(|owner| {
            self.pending
                .get(&key)
                .is_none_or(|pending| !pending.owner.same_as(owner))
        }) {
            return false;
        }
        match self.pending.remove(&key) {
            Some(pending) => {
                self.release_pending(destination_mac, invoke_id, pending, PendingRelease::Cancel);
                true
            }
            _ => false,
        }
    }

    /// Number of confirmed transactions currently awaiting a response.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Whether a transaction keyed to `tsm_mac` with `invoke_id` is pending.
    pub(crate) fn has_transaction(&self, tsm_mac: &[u8], invoke_id: u8) -> bool {
        self.pending
            .contains_key(&(MacAddr::from_slice(tsm_mac), invoke_id))
    }
}

#[cfg(test)]
#[path = "tsm/coordinated_tests.rs"]
mod coordinated_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "tsm/timer_tests.rs"]
mod timer_tests;
