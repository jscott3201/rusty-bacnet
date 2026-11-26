use std::fmt;
use std::sync::Mutex;

use bacnet_encoding::apdu::Apdu;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;
use bacnet_types::enums::ConfirmedServiceChoice;

const INVOKE_ID_COUNT: usize = 256;

/// The invoke IDs a node's pool keeps for its client role (#2617), the
/// `requester` reserve an endpoint session with a client role passes to
/// [`with_reserves`](OutboundTransactionCoordinator::with_reserves):
/// [`LeaseOwner::Requester`] leases can always reach this many, whatever the
/// server role holds. Half the pool, since a request refused an ID fails at
/// once, where the server's work waits for one.
pub const REQUESTER_RESERVE: usize = 128;

/// The invoke IDs a node's pool keeps for its server role (#2617), the
/// `notification` reserve an endpoint session with a server role passes to
/// [`with_reserves`](OutboundTransactionCoordinator::with_reserves):
/// [`LeaseOwner::Notification`] leases (confirmed notifications, Audit
/// records and the requests a server's runs make) can always reach this
/// many, whatever the client holds. A quarter of the pool; with both roles,
/// the quarter neither keeps is shared.
pub const NOTIFICATION_RESERVE: usize = 64;

/// Stable peer identity used to correlate a received response.
#[derive(Debug, PartialEq, Eq, Hash)]
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

// By hand: a derived clone copies each address a byte at a time, and every
// admitted answer clones its lease's peer (#1978).
impl Clone for CanonicalPeer {
    fn clone(&self) -> Self {
        match self {
            Self::Direct(mac) => Self::direct(mac),
            Self::Routed { network, address } => Self::routed(*network, address),
        }
    }
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

    /// The peer an inbound request came from: the SNET/SADR a router added
    /// when there is one, ignoring the immediate router, otherwise the
    /// immediate link MAC.
    ///
    /// `local_network` is the number of the network the receiving link is
    /// attached to, once known. A router attached to it that passes a
    /// station's NPDU back onto it adds that number and the station's MAC as
    /// SNET/SADR (Clause 6.5.4), and network numbers are unique, so the
    /// source is that station: the same direct peer as an NPDU from its MAC
    /// with no SNET, as the server reads a request's source (#1404). Any
    /// other SNET, and every SNET while the number is unknown, stays a routed
    /// peer. A routed source with no SADR names no station, so the immediate
    /// MAC stands.
    ///
    /// Answers are not read this way: [`Self::of_answer`] keeps that form
    /// routed, so another station can't complete a request sent straight to
    /// a device by naming the device as SADR (#2831).
    pub fn from_source(
        immediate_mac: &[u8],
        routed_source: Option<&NpduAddress>,
        local_network: Option<u16>,
    ) -> Self {
        match routed_source {
            Some(source)
                if !source.mac_address.is_empty() && Some(source.network) == local_network =>
            {
                Self::direct(&source.mac_address)
            }
            _ => Self::of_answer(immediate_mac, routed_source),
        }
    }

    /// The peer an answer is matched to: the SNET/SADR a router added when
    /// there is one, otherwise the immediate link MAC. A routed source with
    /// no SADR names no station, so the immediate MAC stands.
    ///
    /// Unlike [`Self::from_source`], an SNET naming this network stays
    /// routed (#2831). A request sent straight to a station on this link is
    /// keyed to its MAC, so only an answer from that MAC with no SNET
    /// completes it. One relayed with this network as SNET completes only a
    /// request still keyed to that routed form, sent before the number was
    /// known. The client then checks its sender as it checks any routed
    /// answer's; the server takes the same admission but doesn't check the
    /// sender yet (the server half of #2831). A frame's SNET and SADR are
    /// its sender's own claim; the link address it comes from is the part a
    /// sender can't choose on most links.
    pub fn of_answer(immediate_mac: &[u8], routed_source: Option<&NpduAddress>) -> Self {
        match routed_source {
            Some(source) if !source.mac_address.is_empty() => {
                Self::routed(source.network, &source.mac_address)
            }
            _ => Self::direct(immediate_mac),
        }
    }

    /// The routed form of a direct answer's source: `Routed(local_network,
    /// immediate MAC)` when the answer has no SNET and `local_network` is
    /// known, otherwise `None`. A request routed to a network that has since
    /// become this one is keyed to that form, and its station, now on this
    /// link, may answer from its own MAC with no SNET (#2371).
    pub fn direct_alias(
        immediate_mac: &[u8],
        routed_source: Option<&NpduAddress>,
        local_network: Option<u16>,
    ) -> Option<Self> {
        match (routed_source, local_network) {
            (None, Some(network)) if !immediate_mac.is_empty() => {
                Some(Self::routed(network, immediate_mac))
            }
            _ => None,
        }
    }
}

/// Workspace role that owns an outbound confirmed transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseOwner {
    /// A local client-side confirmed request.
    Requester,
    /// A confirmed request the local server role initiates: a notification,
    /// a write a Command or Channel object makes in another device, or the
    /// read a Channel makes there to learn a member's datatype. The recipient
    /// answers it as that transaction's server.
    Notification,
}

/// Which role an answer goes to, as [`OutboundTransactionCoordinator::holder_of`]
/// reads a lease without claiming it (#2617).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseHolder {
    /// The client's request ([`LeaseOwner::Requester`]).
    Client,
    /// A request the server role started ([`LeaseOwner::Notification`]).
    Server,
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

    /// Metadata for a request the server role initiates, whose successful
    /// terminal is SimpleACK.
    pub fn notification(peer: CanonicalPeer, service_choice: ConfirmedServiceChoice) -> Self {
        Self {
            owner: LeaseOwner::Notification,
            peer,
            service_choice,
            terminal_policy: TerminalPolicy::SimpleAck,
            segmented_request: false,
        }
    }

    /// Metadata for a read the server role initiates, whose successful
    /// terminal is an unsegmented ComplexACK. The server role reassembles
    /// nothing: it asks for an unsegmented answer and admits no segment.
    pub fn server_read(peer: CanonicalPeer, service_choice: ConfirmedServiceChoice) -> Self {
        Self {
            owner: LeaseOwner::Notification,
            peer,
            service_choice,
            terminal_policy: TerminalPolicy::ComplexAck,
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
    /// No invoke ID this lease's owner may take is free: all 256 are active,
    /// or the free ones are kept for the other owner's reserve
    /// ([`OutboundTransactionCoordinator::with_reserves`]).
    Exhausted,
    /// The monotonic generation counter cannot advance without wrapping.
    GenerationExhausted,
    /// A panic while holding the coordinator mutex poisoned shared state.
    StatePoisoned,
}

impl fmt::Display for ReserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted => formatter
                .write_str("invoke IDs exhausted: none free in this owner's share of the pool"),
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

/// What one [`LeaseOwner`] may lease, as
/// [`OutboundTransactionCoordinator::room`] read it (#2617).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    /// The invoke IDs it could take: free, and within all of the pool but
    /// the other owner's reserve.
    pub available: usize,
    /// What is left of its own reserve: IDs the other owner can't take
    /// first. `None` when the pool keeps it no reserve.
    pub reserve_left: Option<usize>,
}

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

// The per-invoke-ID tables live on the heap. Inline, they made the state about
// 18 KB, and a debug build built and moved it through several stack frames
// (about 110 KB of stack) on every server, client and endpoint start (#953).
// Boxed arrays, not slices: an index from an invoke ID, or taken modulo 256,
// is in bounds by its type, so a lease, an admission and a release check none.
struct CoordinatorState {
    slots: Box<[Option<ActiveLease>; INVOKE_ID_COUNT]>,
    last_released: Box<[Option<u64>; INVOKE_ID_COUNT]>,
    next_invoke_id: usize,
    last_generation: u64,
    /// The leases each owner may still take, indexed by [`LeaseOwner`]:
    /// its ceiling less what it holds. Counting down to the ceiling keeps the
    /// reserve check to the one compare a lease needs (#2617).
    room: [usize; 2],
    /// The most each owner may hold at once: the pool less the other's
    /// reserve.
    ceiling: [usize; 2],
}

/// One entry per invoke ID, built on the heap, never on the stack.
fn table<T: Clone>() -> Box<[Option<T>; INVOKE_ID_COUNT]> {
    match vec![None; INVOKE_ID_COUNT].into_boxed_slice().try_into() {
        Ok(table) => table,
        Err(_) => unreachable!("a table of {INVOKE_ID_COUNT} entries"),
    }
}

impl CoordinatorState {
    fn new(ceiling: [usize; 2]) -> Self {
        Self {
            slots: table(),
            last_released: table(),
            next_invoke_id: 0,
            last_generation: 0,
            room: ceiling,
            ceiling,
        }
    }

    /// The leases active, every owner's.
    fn active(&self) -> usize {
        (self.ceiling[0] - self.room[0]) + (self.ceiling[1] - self.room[1])
    }

    /// What `owner` could lease now.
    fn room(&self, owner: LeaseOwner) -> Room {
        let at = owner as usize;
        // The other's ceiling is the pool less this owner's reserve.
        let reserve = INVOKE_ID_COUNT - self.ceiling[1 - at];
        let held = self.ceiling[at] - self.room[at];
        Room {
            available: self.room[at].min(INVOKE_ID_COUNT - self.active()),
            reserve_left: (reserve > 0).then(|| reserve.saturating_sub(held)),
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
                // Before the old lease drops, so its owner isn't kept across it.
                self.room[active.metadata.owner as usize] += 1;
                self.slots[index] = None;
                self.last_released[index] = Some(token.generation);
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
/// await point, and none takes another lock while holding it: it is the last
/// lock in any order.
///
/// A coordinator made [`with_reserves`](Self::with_reserves) keeps part of
/// its pool for each [`LeaseOwner`] (#2617): an owner may always hold up to
/// its reserve, and beyond it borrows from what neither reserve keeps, so an
/// owner under its reserve is never refused because of the other owner.
/// [`new`](Self::new) keeps nothing back: either owner may take the whole
/// pool.
pub struct OutboundTransactionCoordinator {
    /// The slots and their generations. A panic under this lock may leave
    /// a lease half-recorded, and leasing on from that could give two
    /// transactions one invoke ID, so a poisoned state fails closed: every
    /// call reports [`CoordinatorError::StatePoisoned`] or
    /// [`ReserveError::StatePoisoned`], or sees no lease (#2559).
    state: Mutex<CoordinatorState>,
    released: tokio::sync::Notify,
}

impl Default for OutboundTransactionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundTransactionCoordinator {
    /// Creates an empty coordinator with one global pool of 256 invoke IDs,
    /// any of which either owner may lease: a standalone client's or
    /// server's, or a node's that runs one role.
    pub fn new() -> Self {
        Self::with_reserves(0, 0)
    }

    /// Creates an empty coordinator whose pool of 256 invoke IDs a client
    /// and a server role share (#2617): `requester` of them are kept for
    /// [`LeaseOwner::Requester`] leases and `notification` for
    /// [`LeaseOwner::Notification`] ones. Each owner may hold all of the pool
    /// but the other's reserve, so the two always find their reserves free.
    /// A node with both roles keeps [`REQUESTER_RESERVE`] and
    /// [`NOTIFICATION_RESERVE`].
    ///
    /// # Panics
    ///
    /// When the two reserves together exceed 256.
    ///
    /// ```
    /// use bacnet_endpoint_core::coordinator::{
    ///     CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator, ReserveError,
    /// };
    /// use bacnet_types::enums::ConfirmedServiceChoice;
    ///
    /// let coordinator = OutboundTransactionCoordinator::with_reserves(200, 56);
    /// let notification = || {
    ///     LeaseMetadata::notification(
    ///         CanonicalPeer::direct(&[1]),
    ///         ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
    ///     )
    /// };
    /// for _ in 0..56 {
    ///     coordinator.reserve(notification()).unwrap();
    /// }
    /// // The other 200 are the client's.
    /// assert_eq!(coordinator.reserve(notification()), Err(ReserveError::Exhausted));
    /// ```
    pub fn with_reserves(requester: usize, notification: usize) -> Self {
        let kept = requester.checked_add(notification);
        assert!(
            kept.is_some_and(|kept| kept <= INVOKE_ID_COUNT),
            "invoke-ID reserves {requester} and {notification} exceed the pool of {INVOKE_ID_COUNT}"
        );
        let mut ceiling = [0; 2];
        ceiling[LeaseOwner::Requester as usize] = INVOKE_ID_COUNT - notification;
        ceiling[LeaseOwner::Notification as usize] = INVOKE_ID_COUNT - requester;
        Self {
            state: Mutex::new(CoordinatorState::new(ceiling)),
            released: tokio::sync::Notify::new(),
        }
    }

    /// Reserves one device-wide invoke ID and issues a new exact-generation
    /// token, or [`ReserveError::Exhausted`] when none is free that the
    /// lease's owner may take ([`Self::with_reserves`]).
    pub fn reserve(&self, metadata: LeaseMetadata) -> Result<LeaseToken, ReserveError> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        let mut state = self.state.lock().map_err(|_| ReserveError::StatePoisoned)?;
        let owner = metadata.owner as usize;
        // At its ceiling, the owner leaves the rest to the other's reserve.
        if state.room[owner] == 0 {
            return Err(ReserveError::Exhausted);
        }
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
        state.room[owner] -= 1;
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
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
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

    /// [`Self::admit`] for an answer, by its envelope: the link MAC, the
    /// SNET/SADR, and `local_network`, the receiving link's known network
    /// number. The answer is matched against [`CanonicalPeer::of_answer`].
    /// Only when that misses on the peer, and the answer has a
    /// [`CanonicalPeer::direct_alias`], is the alias tried: a lease reserved
    /// for the routed form of a network that has since become this one then
    /// gets its station's direct answer (#2371). An answer relayed with this
    /// network as SNET never stands for the station it names (#2831). The
    /// admission's metadata names the peer that matched, which the owner
    /// keys its completion to.
    pub fn admit_from_source(
        &self,
        immediate_mac: &[u8],
        routed_source: Option<&NpduAddress>,
        local_network: Option<u16>,
        apdu: &Apdu,
    ) -> Result<AdmissionOutcome, CoordinatorError> {
        let peer = CanonicalPeer::of_answer(immediate_mac, routed_source);
        match self.admit(&peer, apdu)? {
            AdmissionOutcome::PeerMismatch => {
                match CanonicalPeer::direct_alias(immediate_mac, routed_source, local_network) {
                    Some(alias) => self.admit(&alias, apdu),
                    None => Ok(AdmissionOutcome::PeerMismatch),
                }
            }
            outcome => Ok(outcome),
        }
    }

    /// The role that holds the active lease of `invoke_id` for `peer`, or
    /// `None` when no lease is active for that pair. Nothing is claimed or
    /// changed: the role [`admits`](Self::admit) the answer itself. A
    /// poisoned coordinator reads as no holder.
    pub fn holder_of(&self, peer: &CanonicalPeer, invoke_id: u8) -> Option<LeaseHolder> {
        self.lease_of(peer, invoke_id).map(|(holder, _)| holder)
    }

    /// The holder of the active lease of `invoke_id` for `peer`, and the
    /// service it asked for. Claims nothing.
    fn lease_of(
        &self,
        peer: &CanonicalPeer,
        invoke_id: u8,
    ) -> Option<(LeaseHolder, ConfirmedServiceChoice)> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        let state = self.state.lock().ok()?;
        let active = state.slots[usize::from(invoke_id)].as_ref()?;
        if active.metadata.peer != *peer {
            return None;
        }
        let holder = match active.metadata.owner {
            LeaseOwner::Notification => LeaseHolder::Server,
            LeaseOwner::Requester => LeaseHolder::Client,
        };
        Some((holder, active.metadata.service_choice))
    }

    /// [`Self::holder_of`] for an answer, by its envelope, matched as
    /// [`Self::admit_from_source`] matches it: the peer
    /// [`CanonicalPeer::of_answer`] reads, then its direct alias.
    pub fn holder_of_source(
        &self,
        immediate_mac: &[u8],
        routed_source: Option<&NpduAddress>,
        local_network: Option<u16>,
        invoke_id: u8,
    ) -> Option<LeaseHolder> {
        self.lease_of_source(immediate_mac, routed_source, local_network, invoke_id)
            .map(|(holder, _)| holder)
    }

    /// [`Self::holder_of_source`], with the service the lease asked for, so
    /// a caller can match an answer's service choice as
    /// [`Self::admit_from_source`] does (#2617). Claims nothing.
    #[doc(hidden)]
    pub fn lease_of_source(
        &self,
        immediate_mac: &[u8],
        routed_source: Option<&NpduAddress>,
        local_network: Option<u16>,
        invoke_id: u8,
    ) -> Option<(LeaseHolder, ConfirmedServiceChoice)> {
        let peer = CanonicalPeer::of_answer(immediate_mac, routed_source);
        self.lease_of(&peer, invoke_id).or_else(|| {
            let alias = CanonicalPeer::direct_alias(immediate_mac, routed_source, local_network)?;
            self.lease_of(&alias, invoke_id)
        })
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
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        self.state
            .lock()
            .map(|state| state.active())
            .map_err(|_| CoordinatorError::StatePoisoned)
    }

    /// What a lease of `owner`'s could take now (#2617). Racing work may
    /// take it first; [`Self::reserve`] decides.
    pub fn room(&self, owner: LeaseOwner) -> Result<Room, CoordinatorError> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        self.state
            .lock()
            .map(|state| state.room(owner))
            .map_err(|_| CoordinatorError::StatePoisoned)
    }

    fn release_token(&self, token: LeaseToken) -> Result<ReleaseOutcome, CoordinatorError> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        let outcome = self
            .state
            .lock()
            .map(|mut state| state.release_exact(token))
            .map_err(|_| CoordinatorError::StatePoisoned)?;
        // Wake after releasing the state lock, and only for actual capacity.
        if outcome == ReleaseOutcome::Released {
            self.released.notify_waiters();
        }
        Ok(outcome)
    }

    /// A wait for room in the pool: it completes once a lease ends after it
    /// is made, or once [`Self::wake_released`] is called, whether or not it
    /// was polled by then. Nothing is stored for a wait made later, and the
    /// wait takes no ID: an owner makes it before it reads the room or
    /// tries [`Self::reserve`], then, on waking, reads or tries again, since
    /// another owner may have taken what came back. Dropping it gives up
    /// nothing.
    pub fn released(&self) -> tokio::sync::futures::Notified<'_> {
        self.released.notified()
    }

    /// Wakes every [`Self::released`] wait made so far, as a lease's end
    /// does, though none ended. For an owner whose own rule held its work
    /// back while IDs were free, once the rule no longer does (#2617): no
    /// lease may end to wake that work. Every other wait made so far wakes
    /// too, to read the room again and find what it found before; a wait
    /// made after this call is not woken by it.
    pub fn wake_released(&self) {
        self.released.notify_waiters();
    }
}

/// The invoke ID an answer carries, or `None` for a request, which no
/// outbound lease owns.
pub fn response_invoke_id(apdu: &Apdu) -> Option<u8> {
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
            // A server-role lease takes a ComplexACK only when it asked for
            // one ([`LeaseMetadata::server_read`]), and never a segment.
            let server_read = !pdu.segmented && metadata.terminal_policy.accepts_complex_ack();
            if metadata.owner != LeaseOwner::Requester && !server_read {
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
        Apdu::Error(pdu) => {
            validate_service(metadata, pdu.service_choice)?;
            Ok(AdmissionKind::Terminal)
        }
        Apdu::Reject(_) => Ok(AdmissionKind::Terminal),
        Apdu::Abort(pdu) => {
            // Every lease is the requesting side of its transaction, a
            // confirmed notification included, so the peer's Abort comes from
            // the responding side and has the server flag set (Clause 5.4).
            // A clear flag marks an Abort meant for a transaction this device
            // serves, whatever the lease owner.
            if !pdu.sent_by_server {
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

#[cfg(test)]
#[path = "coordinator_peer_tests.rs"]
mod peer_tests;

#[cfg(test)]
#[path = "coordinator_reserve_tests.rs"]
mod reserve_tests;
