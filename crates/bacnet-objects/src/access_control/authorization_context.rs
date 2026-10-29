//! Mandatory application-owned facts for local authorization.

use super::{
    AccompanimentDeadline, AccompanimentTiming, AccompanimentWitness, AuthorizationOrder,
    AuthorizationRightsPolicy, CredentialPresentation, DisableReasonPriority,
};
use crate::clock::ClockFrame;
use bacnet_types::enums::AccessPassbackMode;
use bacnet_types::primitives::ObjectIdentifier;
use core::time::Duration;

/// A known application fact or an explicit lack of evidence. No default exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationFact<T> {
    /// The application supplies this value for a genuinely absent property.
    Known(T),
    /// Required evidence is unavailable; this does not mean disabled or zero.
    Unknown,
}

/// Which occupancy boundary a presentation crosses for one relevant zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDirection {
    /// Entry is checked against a nonzero upper bound.
    Entering,
    /// Exit is checked against a nonzero lower bound.
    Leaving,
}

/// One application-selected relevant zone and its enforcement fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationOccupancyZone {
    /// Local Access Zone whose topology/count/limits are read from the database.
    pub zone: ObjectIdentifier,
    /// Established direction; the matching zone point list must contain the point.
    pub direction: AuthorizationDirection,
    /// Used only when the point omits the direction's enforcement property.
    pub fallback_enforced: AuthorizationFact<bool>,
}

/// The application's complete relevant-zone set; the helper invents no topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationOccupancy<'a> {
    /// The application establishes that this presentation affects no zone bound.
    NotApplicable,
    /// Relevant direction/zones are not established.
    Unknown,
    /// All relevant zones, evaluated in this explicit order. Empty is unresolved;
    /// use NotApplicable to state that no zone check is needed.
    Zones(&'a [AuthorizationOccupancyZone]),
}

/// An already established passback observation bound to this transaction/zone.
/// This helper does not reconstruct a location history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationPassbackObservation {
    /// Complete original tuple to which the observation belongs.
    pub presentation: CredentialPresentation,
    /// The local zone in which the observation was established.
    pub zone: ObjectIdentifier,
    /// Whether a violation was established before applying timeout/exemption.
    pub violation: bool,
    /// Last successful entry of this credential into this zone, sharing the
    /// monotonic origin. This is not Last_Use_Time. Required to expire a violation
    /// under a nonzero timeout; a future entry is inconsistent.
    pub last_successful_entry: Option<Duration>,
}

/// Passback applicability and missing-property inputs (Clauses 12.32.21–22).
/// A PASSBACK-exempt credential still reports an established SOFT violation.
/// Unusable optional reporting evidence is retained in
/// [`super::AuthorizationEvaluation::passback_reporting_problem`] without
/// imposing enforcement or another required criterion on the exempt request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationPassback {
    /// The application establishes no relevant passback check for this presentation.
    NotApplicable,
    /// The relevant zone or passback facts are unknown.
    Unknown,
    /// One established local zone, its settings, and presentation observation.
    Zone {
        /// Local Access Zone used for the check.
        zone: ObjectIdentifier,
        /// Used only if Passback_Mode is genuinely absent.
        fallback_mode: AuthorizationFact<AccessPassbackMode>,
        /// Missing Passback_Timeout is specified to mean no expiry. This
        /// optional fallback allows application configuration where the property
        /// is absent; None and zero both mean no expiry, in minutes.
        fallback_timeout_minutes: Option<u32>,
        /// Required for unexempted HARD/SOFT enforcement/reporting. Exempt SOFT
        /// reporting uses it when available; exempt HARD and OFF do not need it.
        observation: AuthorizationFact<AuthorizationPassbackObservation>,
    },
}

/// Authentication outcome for one exact companion presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionAuthentication {
    /// Authentication completed for this tuple; no rights grant is asserted.
    Authenticated,
    /// Authentication was conclusively rejected for this tuple.
    Rejected,
    /// Authentication has not been established.
    Unknown,
}

/// An application attestation bound to sequence, credential, point, and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionAuthenticationAttestation {
    /// Full presentation tuple. Matching only credential identity is insufficient.
    pub presentation: CredentialPresentation,
    /// Authentication evidence, without a caller-provided access-rights decision.
    pub authentication: CompanionAuthentication,
}

/// External verification/delay evidence at a supplied observation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationVerdict {
    /// No external verdict has arrived; the verified deadline determines expiry.
    Waiting,
    /// The application explicitly observed expiry; an early claim is unresolved.
    DeadlineReached,
    /// An approval arrived at this monotonic time. It never shortens a delay.
    Approved(Duration),
    /// A rejection arrived at this monotonic time.
    Rejected(Duration),
}

/// Explicit external-phase correlation and timing, when the mode requires it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationExternalPhase {
    /// No phase evidence supplied; unresolved when an unexempted mode needs it.
    NotSupplied,
    /// Evidence for one application-owned phase of this exact presentation.
    Observed {
        /// Complete original presentation tuple.
        presentation: CredentialPresentation,
        /// Phase start, at or after presentation and no later than `now`.
        started: Duration,
        /// Used only if the point omits Verification_Time, in seconds when served.
        fallback_period: AuthorizationFact<Duration>,
        /// Explicit boundary policy, reused from accompaniment timing.
        deadline: AccompanimentDeadline,
        /// Waiting, a dated verdict, or an explicit expiry claim.
        verdict: AuthorizationVerdict,
    },
}

/// One already-authenticated local presentation and every application-owned
/// authorization choice. There is no Default implementation or installed policy.
///
/// Database properties take precedence over their narrow absent-property
/// fallbacks. Malformed, denied, or otherwise failed reads never use a fallback.
/// All observations belong to one coherent application context; this API does
/// not authenticate hardware, discover remote facts, or publish events.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    /// Authenticated original tuple; this helper is not the factor engine.
    pub original: CredentialPresentation,
    /// Supplied local wall-clock frame used for all credential validity checks.
    /// Finite limits require a valid coherent frame. Two open limits do not.
    pub wall_time: Option<ClockFrame>,
    /// Monotonic observation time, separate from BACnet wall-clock time.
    pub now: Duration,
    /// Every required criterion in the caller's explicit order.
    pub order: AuthorizationOrder,
    /// Explicit precedence among multiple coherent disable reasons.
    pub disable_reason_priority: DisableReasonPriority,
    /// Explicit treatment of original and companion excluded assignments.
    pub rights_policy: AuthorizationRightsPolicy,
    /// Point threat level if its property is genuinely absent; range 0..100.
    /// Zero disables the comparison without reading credential authority.
    /// Otherwise absent credential Threat_Authority means zero by specification.
    pub fallback_threat_level: AuthorizationFact<u32>,
    /// Point Lockout if its property is genuinely absent.
    pub fallback_lockout: AuthorizationFact<bool>,
    /// Complete relevant occupancy context.
    pub occupancy: AuthorizationOccupancy<'a>,
    /// Passback applicability, settings, and observed successful-entry evidence.
    pub passback: AuthorizationPassback,
    /// Witnesses consumed by the existing bounded accompaniment stage.
    pub accompaniment_witnesses: &'a [AccompanimentWitness],
    /// Explicit order/period/boundary policy for selected accompaniment.
    pub accompaniment_timing: AccompanimentTiming,
    /// Existing accompaniment traversal bound; exhaustion cannot grant access.
    pub max_accompaniment_links: usize,
    /// Authentication results for each companion actually relied upon.
    pub companion_authentication: &'a [CompanionAuthenticationAttestation],
    /// External verification/delay observation if needed by the served mode.
    pub external_phase: AuthorizationExternalPhase,
}
