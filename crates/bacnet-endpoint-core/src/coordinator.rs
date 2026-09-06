use std::fmt;
use std::sync::Mutex;

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::MacAddr;

const INVOKE_ID_COUNT: usize = 256;

/// Stable peer identity used to correlate a received response.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CanonicalPeer {
    /// A device reached directly on the attached data link.
    Direct(MacAddr),
    /// The originating network and address carried by a routed NPDU.
    Routed {
        /// Originating BACnet network number.
        network: u16,
        /// Originating address on that network.
        address: MacAddr,
    },
}

impl CanonicalPeer {
    /// Creates a direct peer from its data-link address.
    pub fn direct(mac: &[u8]) -> Self {
        Self::Direct(MacAddr::from_slice(mac))
    }

    /// Creates a routed peer from its originating network and address.
    pub fn routed(network: u16, address: &[u8]) -> Self {
        Self::Routed {
            network,
            address: MacAddr::from_slice(address),
        }
    }

    /// Selects routed source identity when present, ignoring the immediate router.
    pub fn from_source(immediate_mac: &[u8], routed_source: Option<&NpduAddress>) -> Self {
        match routed_source {
            Some(source) => Self::routed(source.network, &source.mac_address),
            None => Self::direct(immediate_mac),
        }
    }
}

/// Workspace role that owns an outbound confirmed transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseOwner {
    /// A local client-side confirmed request.
    Requester,
    /// A confirmed notification initiated by the local server role.
    ServerNotification,
}

/// Successful acknowledgment shape accepted for a lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalPolicy {
    /// The service completes with a SimpleACK.
    SimpleAck,
    /// The service completes with an unsegmented or reassembled ComplexACK.
    ComplexAck,
    /// The service may complete with either a SimpleACK or ComplexACK.
    EitherAck,
}

impl TerminalPolicy {
    fn accepts_simple_ack(self) -> bool {
        matches!(self, Self::SimpleAck | Self::EitherAck)
    }

    fn accepts_complex_ack(self) -> bool {
        matches!(self, Self::ComplexAck | Self::EitherAck)
    }
}

/// Correlation and response policy retained for one active invoke ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseMetadata {
    owner: LeaseOwner,
    peer: CanonicalPeer,
    service_choice: ConfirmedServiceChoice,
    terminal_policy: TerminalPolicy,
    segmented_request: bool,
}

impl LeaseMetadata {
    /// Metadata for an unsegmented requester transaction.
    pub fn requester(
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        terminal_policy: TerminalPolicy,
    ) -> Self {
        Self {
            owner: LeaseOwner::Requester,
            peer,
            service_choice,
            terminal_policy,
            segmented_request: false,
        }
    }

    /// Metadata for a requester transaction that sends request segments.
    pub fn segmented_requester(
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
        terminal_policy: TerminalPolicy,
    ) -> Self {
        Self {
            owner: LeaseOwner::Requester,
            peer,
            service_choice,
            terminal_policy,
            segmented_request: true,
        }
    }

    /// Metadata for a server notification, whose successful terminal is SimpleACK.
    pub fn server_notification(
        peer: CanonicalPeer,
        service_choice: ConfirmedServiceChoice,
    ) -> Self {
        Self {
            owner: LeaseOwner::ServerNotification,
            peer,
            service_choice,
            terminal_policy: TerminalPolicy::SimpleAck,
            segmented_request: false,
        }
    }

    /// Returns the role that owns the transaction.
    pub fn owner(&self) -> LeaseOwner {
        self.owner
    }

    /// Returns the canonical response peer.
    pub fn peer(&self) -> &CanonicalPeer {
        &self.peer
    }

    /// Returns the confirmed service expected in service-labelled responses.
    pub fn service_choice(&self) -> ConfirmedServiceChoice {
        self.service_choice
    }

    /// Returns the accepted successful acknowledgment policy.
    pub fn terminal_policy(&self) -> TerminalPolicy {
        self.terminal_policy
    }

    /// Reports whether the outbound confirmed request is segmented.
    pub fn is_segmented_request(&self) -> bool {
        self.segmented_request
    }
}

/// Exact local identity for one lease of a reusable wire invoke ID.
///
/// The generation fences delayed local cleanup, timers, and reassembly work.
/// It is not sent on the wire and cannot distinguish an old wire response after
/// the same invoke ID is reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LeaseToken {
    invoke_id: u8,
    generation: u64,
}

impl LeaseToken {
    /// Returns the invoke ID to encode in the confirmed request APDU.
    pub fn invoke_id(self) -> u8 {
        self.invoke_id
    }
}

/// Reservation failures that leave coordinator state unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReserveError {
    /// All 256 device-wide invoke IDs are active.
    Exhausted,
    /// The monotonic generation counter cannot advance without wrapping.
    GenerationExhausted,
    /// A panic while holding the coordinator mutex poisoned shared state.
    StatePoisoned,
}

impl fmt::Display for ReserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted => formatter.write_str("all invoke IDs are leased"),
            Self::GenerationExhausted => formatter.write_str("lease generation exhausted"),
            Self::StatePoisoned => formatter.write_str("coordinator state is poisoned"),
        }
    }
}

impl std::error::Error for ReserveError {}

/// Shared-state failures for non-reservation operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinatorError {
    /// A panic while holding the coordinator mutex poisoned shared state.
    StatePoisoned,
}

impl fmt::Display for CoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("coordinator state is poisoned")
    }
}

impl std::error::Error for CoordinatorError {}

/// Result of exact-token completion, cancellation, or release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The exact active lease was released.
    Released,
    /// This exact lease had already been released and no ID reuse intervened.
    AlreadyReleased,
    /// The token does not identify the current lease of this invoke ID.
    StaleToken,
}

/// Whether an admitted APDU claims the terminal delivery slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionKind {
    /// The APDU claims this lease's one terminal admission.
    Terminal,
    /// The APDU reports segmentation progress and leaves the lease active.
    NonTerminal,
}

/// Exact lease and metadata selected for an admitted APDU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    token: LeaseToken,
    metadata: LeaseMetadata,
    kind: AdmissionKind,
}

impl Admission {
    /// Returns the exact-generation token for later completion or cleanup.
    pub fn token(&self) -> LeaseToken {
        self.token
    }

    /// Returns the active lease's role and correlation policy.
    pub fn metadata(&self) -> &LeaseMetadata {
        &self.metadata
    }

    /// Returns whether the event is terminal at coordinator level.
    pub fn kind(&self) -> AdmissionKind {
        self.kind
    }
}

/// Bounded outcome of matching one decoded APDU to an outbound lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionOutcome {
    /// The APDU matched an active lease.
    Admitted(Admission),
    /// Requests do not belong to the outbound coordinator.
    NotOutbound,
    /// No lease is active for the APDU's invoke ID.
    UnknownInvokeId,
    /// The source identity does not match the active lease.
    PeerMismatch,
    /// The APDU is not valid for the role or request segmentation mode.
    OwnerMismatch,
    /// An unsegmented successful acknowledgment named a different service.
    ServiceMismatch {
        /// Service retained by the active lease.
        expected: ConfirmedServiceChoice,
        /// Service carried by the received APDU.
        observed: ConfirmedServiceChoice,
    },
    /// The acknowledgment shape is excluded by the lease's terminal policy.
    PolicyMismatch,
    /// The APDU's server bit routes it to another transaction role.
    DirectionMismatch,
    /// A terminal APDU already claimed this lease.
    DuplicateTerminal,
}

#[derive(Clone, Debug)]
struct ActiveLease {
    generation: u64,
    metadata: LeaseMetadata,
    terminal_claimed: bool,
}

struct CoordinatorState {
    slots: [Option<ActiveLease>; INVOKE_ID_COUNT],
    last_released: [Option<u64>; INVOKE_ID_COUNT],
    next_invoke_id: usize,
    last_generation: u64,
    active_count: usize,
}

impl CoordinatorState {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            last_released: [None; INVOKE_ID_COUNT],
            next_invoke_id: 0,
            last_generation: 0,
            active_count: 0,
        }
    }

    fn available_slot(&self) -> Option<usize> {
        (0..INVOKE_ID_COUNT)
            .map(|offset| (self.next_invoke_id + offset) % INVOKE_ID_COUNT)
            .find(|index| self.slots[*index].is_none())
    }

    fn release_exact(&mut self, token: LeaseToken) -> ReleaseOutcome {
        let index = usize::from(token.invoke_id);
        match self.slots[index].as_ref() {
            Some(active) if active.generation == token.generation => {
                self.slots[index] = None;
                self.last_released[index] = Some(token.generation);
                self.active_count -= 1;
                ReleaseOutcome::Released
            }
            Some(_) => ReleaseOutcome::StaleToken,
            None if self.last_released[index] == Some(token.generation) => {
                ReleaseOutcome::AlreadyReleased
            }
            None => ReleaseOutcome::StaleToken,
        }
    }
}

/// Thread-safe device-wide invoke-ID lease coordinator.
///
/// Every method holds a synchronous mutex only while inspecting or updating
/// bounded in-memory state. Callers never carry a coordinator lock across an
/// await point.
pub struct OutboundTransactionCoordinator {
    state: Mutex<CoordinatorState>,
}

impl Default for OutboundTransactionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundTransactionCoordinator {
    /// Creates an empty coordinator with one global pool of 256 invoke IDs.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(CoordinatorState::new()),
        }
    }

    /// Reserves one device-wide invoke ID and issues a new exact-generation token.
    pub fn reserve(&self, metadata: LeaseMetadata) -> Result<LeaseToken, ReserveError> {
        let mut state = self.state.lock().map_err(|_| ReserveError::StatePoisoned)?;
        let index = state.available_slot().ok_or(ReserveError::Exhausted)?;
        let generation = state
            .last_generation
            .checked_add(1)
            .ok_or(ReserveError::GenerationExhausted)?;
        let token = LeaseToken {
            invoke_id: index as u8,
            generation,
        };

        state.last_generation = generation;
        state.next_invoke_id = (index + 1) % INVOKE_ID_COUNT;
        state.slots[index] = Some(ActiveLease {
            generation,
            metadata,
            terminal_claimed: false,
        });
        state.active_count += 1;
        Ok(token)
    }

    /// Matches an APDU without releasing its lease.
    ///
    /// A successful terminal match claims exact-once delivery. Segmented
    /// ComplexACK and server SegmentACK matches are non-terminal.
    pub fn admit(
        &self,
        peer: &CanonicalPeer,
        apdu: &Apdu,
    ) -> Result<AdmissionOutcome, CoordinatorError> {
        let Some(invoke_id) = response_invoke_id(apdu) else {
            return Ok(AdmissionOutcome::NotOutbound);
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::StatePoisoned)?;
        let Some(active) = state.slots[usize::from(invoke_id)].as_mut() else {
            return Ok(AdmissionOutcome::UnknownInvokeId);
        };
        if active.metadata.peer != *peer {
            return Ok(AdmissionOutcome::PeerMismatch);
        }

        let kind = match validate_apdu(&active.metadata, apdu) {
            Ok(kind) => kind,
            Err(outcome) => return Ok(outcome),
        };
        if kind == AdmissionKind::Terminal && active.terminal_claimed {
            return Ok(AdmissionOutcome::DuplicateTerminal);
        }
        if kind == AdmissionKind::Terminal {
            active.terminal_claimed = true;
        }

        Ok(AdmissionOutcome::Admitted(Admission {
            token: LeaseToken {
                invoke_id,
                generation: active.generation,
            },
            metadata: active.metadata.clone(),
            kind,
        }))
    }

    /// Releases a lease after its admitted terminal has been delivered.
    pub fn complete(&self, token: LeaseToken) -> Result<ReleaseOutcome, CoordinatorError> {
        self.release_token(token)
    }

    /// Releases a lease because its owning operation was cancelled.
    pub fn cancel(&self, token: LeaseToken) -> Result<ReleaseOutcome, CoordinatorError> {
        self.release_token(token)
    }

    /// Releases a lease for timeout or other owner-directed cleanup.
    pub fn release(&self, token: LeaseToken) -> Result<ReleaseOutcome, CoordinatorError> {
        self.release_token(token)
    }

    /// Returns the number of device-wide active leases.
    pub fn active_count(&self) -> Result<usize, CoordinatorError> {
        self.state
            .lock()
            .map(|state| state.active_count)
            .map_err(|_| CoordinatorError::StatePoisoned)
    }

    fn release_token(&self, token: LeaseToken) -> Result<ReleaseOutcome, CoordinatorError> {
        self.state
            .lock()
            .map(|mut state| state.release_exact(token))
            .map_err(|_| CoordinatorError::StatePoisoned)
    }
}

fn response_invoke_id(apdu: &Apdu) -> Option<u8> {
    match apdu {
        Apdu::SimpleAck(pdu) => Some(pdu.invoke_id),
        Apdu::ComplexAck(pdu) => Some(pdu.invoke_id),
        Apdu::SegmentAck(pdu) => Some(pdu.invoke_id),
        Apdu::Error(pdu) => Some(pdu.invoke_id),
        Apdu::Reject(pdu) => Some(pdu.invoke_id),
        Apdu::Abort(pdu) => Some(pdu.invoke_id),
        Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_) => None,
    }
}

fn validate_apdu(metadata: &LeaseMetadata, apdu: &Apdu) -> Result<AdmissionKind, AdmissionOutcome> {
    match apdu {
        Apdu::SimpleAck(pdu) => {
            validate_service(metadata, pdu.service_choice)?;
            if !metadata.terminal_policy.accepts_simple_ack() {
                return Err(AdmissionOutcome::PolicyMismatch);
            }
            Ok(AdmissionKind::Terminal)
        }
        Apdu::ComplexAck(pdu) => {
            if !pdu.segmented {
                validate_service(metadata, pdu.service_choice)?;
            }
            if metadata.owner != LeaseOwner::Requester {
                return Err(AdmissionOutcome::OwnerMismatch);
            }
            if !metadata.terminal_policy.accepts_complex_ack() {
                return Err(AdmissionOutcome::PolicyMismatch);
            }
            if pdu.segmented {
                Ok(AdmissionKind::NonTerminal)
            } else {
                Ok(AdmissionKind::Terminal)
            }
        }
        Apdu::Error(_) => Ok(AdmissionKind::Terminal),
        Apdu::Reject(_) => Ok(AdmissionKind::Terminal),
        Apdu::Abort(pdu) => {
            let expected_server_bit = match metadata.owner {
                LeaseOwner::Requester => true,
                LeaseOwner::ServerNotification => false,
            };
            if pdu.sent_by_server != expected_server_bit {
                return Err(AdmissionOutcome::DirectionMismatch);
            }
            Ok(AdmissionKind::Terminal)
        }
        Apdu::SegmentAck(pdu) => {
            if !pdu.sent_by_server {
                return Err(AdmissionOutcome::DirectionMismatch);
            }
            if metadata.owner != LeaseOwner::Requester || !metadata.segmented_request {
                return Err(AdmissionOutcome::OwnerMismatch);
            }
            Ok(AdmissionKind::NonTerminal)
        }
        Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_) => {
            Err(AdmissionOutcome::NotOutbound)
        }
    }
}

fn validate_service(
    metadata: &LeaseMetadata,
    observed: ConfirmedServiceChoice,
) -> Result<(), AdmissionOutcome> {
    if metadata.service_choice == observed {
        Ok(())
    } else {
        Err(AdmissionOutcome::ServiceMismatch {
            expected: metadata.service_choice,
            observed,
        })
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
